use crate::direct_api::api_client::{
    api_error_message_from_sse_text, api_error_message_from_value,
    call_chat_completions_non_stream, call_chat_completions_stream, ChatCompletionChunk,
    DirectApiConfig, SseParser, UserAttachment,
};
use crate::direct_api::tools::execute_tool;
use crate::storage::ChatMessage;
use crate::AppState;
use base64::Engine;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tauri::{Emitter, Manager};

const MAX_AGENT_TURNS: u32 = 160;
const MAX_VISION_IMAGES: usize = 5;
const MAX_VISION_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Serialize)]
struct ToolConfirmPayload {
    id: String,
    tool_name: String,
    arguments: String,
    summary: String,
    command: Option<String>,
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_avatar: Option<String>,
}

/// ask_user 工具弹窗里的单个候选项。
#[derive(Clone, Serialize)]
pub struct AskUserOption {
    label: String,
    description: Option<String>,
}

/// ai-ask-user 事件载荷: 智能体主动向用户提问, 前端渲染内嵌弹窗。
#[derive(Clone, Serialize)]
pub struct AskUserPayload {
    id: String,
    question: String,
    options: Vec<AskUserOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_avatar: Option<String>,
}

#[derive(Clone, Serialize)]
struct ToolEventPayload {
    id: String,
    tool_name: String,
    status: String,
    summary: String,
    arguments: String,
    command: Option<String>,
    path: Option<String>,
    output: Option<String>,
    approved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_name: Option<String>,
}

struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Serialize)]
struct AnswerDeltaPayload {
    text: String,
}

pub async fn run_direct_api_agent(
    app_handle: tauri::AppHandle,
    message: String,
    config: DirectApiConfig,
    system_prompt: String,
    chat_history: Vec<ChatMessage>,
    attachments: Vec<UserAttachment>,
    quote: Option<(String, String)>,
    agent: Option<crate::AgentContext>,
    tool_whitelist: Option<Vec<String>>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let state = app_handle.state::<AppState>();

    let mut api_messages = Vec::new();
    api_messages.push(json!({
        "role": "system",
        "content": system_prompt
    }));

    for msg in chat_history {
        let content = if msg.role == "assistant" {
            assistant_history_content_for_context(&msg, agent.as_ref())
        } else {
            msg.content
        };
        api_messages.push(json!({
            "role": msg.role,
            "content": content
        }));
    }

    api_messages.push(json!({
        "role": "user",
        "content": build_user_content(&apply_quote_context(&message, quote.as_ref()), &attachments).await
    }));

    let mut current_turn = 0;
    let mut full_text = String::new();
    let mut full_thinking = String::new();

    loop {
        current_turn += 1;
        if current_turn > MAX_AGENT_TURNS {
            let limit_msg = turn_limit_message(MAX_AGENT_TURNS, &full_thinking);
            full_text.push_str(&limit_msg);
            full_thinking.push_str(&limit_msg);
            append_active_thinking(&state, &limit_msg);
            let _ = app_handle.emit("ai-thinking", &limit_msg);
            break;
        }

        if cancel.is_cancelled() {
            send_aborted(&app_handle, &full_text, &full_thinking).await;
            return;
        }

        // 工具白名单 = 能力限制：不在名单内的工具模型看不到。
        // 名单为 None 或空 = 不限制（全部工具）。
        let tools = if config.execution_mode == "plan" {
            None
        } else {
            Some(crate::direct_api::tools::tool_definitions_for(
                tool_whitelist.as_deref(),
            ))
        };

        let mut accumulated_tool_calls: HashMap<usize, AccumulatedToolCall> = HashMap::new();
        let mut turn_text = String::new();

        // 根据stream_mode选择不同的调用方式
        match config.stream_mode.as_str() {
            "non_stream" => {
                // 非流式解析：直接调用non-stream函数
                match call_chat_completions_non_stream(
                    &state.http_client,
                    &config,
                    &api_messages,
                    tools,
                )
                .await
                {
                    Ok(result) => {
                        // 处理文本内容
                        if let Some(ref content) = result.content {
                            if !content.is_empty() {
                                turn_text.push_str(content);
                                full_text.push_str(content);
                                let _ = app_handle.emit(
                                    "ai-answer-delta",
                                    AnswerDeltaPayload {
                                        text: content.clone(),
                                    },
                                );
                            }
                        }

                        // 处理工具调用 - 填充 accumulated_tool_calls
                        // 这样现有的工具执行循环会处理它们
                        for (i, tc) in result.tool_calls.iter().enumerate() {
                            accumulated_tool_calls.insert(
                                i,
                                AccumulatedToolCall {
                                    id: tc.id.clone().unwrap_or_default(),
                                    name: tc.function.name.clone(),
                                    arguments: tc.function.arguments.clone(),
                                },
                            );
                        }
                    }
                    Err(e) => {
                        send_error(&app_handle, format!("API 请求失败: {e}"), &full_thinking).await;
                        return;
                    }
                }
            }
            "auto" | _ => {
                // 自动模式或流式模式：使用流式调用，支持重试
                // 重试上限提高到 5 次：用户的中转服务 (xiaomimimo 等) 经常中途断流
                const MAX_STREAM_RETRIES: u32 = 5;
                let mut stream_success = false;
                let mut raw_buffer = String::new();

                for attempt in 0..MAX_STREAM_RETRIES {
                    if attempt > 0 {
                        let delay = Duration::from_secs(2u64.pow(attempt as u32));
                        let _ = app_handle.emit(
                            "ai-thinking",
                            format!(
                                "\n[连接中断，{}秒后重试 ({}/{})]\n",
                                delay.as_secs(),
                                attempt,
                                MAX_STREAM_RETRIES
                            ),
                        );
                        tokio::time::sleep(delay).await;
                    }

                    let res = match call_chat_completions_stream(
                        &state.http_client,
                        &config,
                        &api_messages,
                        tools.clone(),
                    )
                    .await
                    {
                        Ok(res) => res,
                        Err(e) => {
                            if attempt < MAX_STREAM_RETRIES - 1 {
                                continue;
                            }
                            send_error(&app_handle, format!("API 请求失败: {e}"), &full_thinking)
                                .await;
                            return;
                        }
                    };

                    let mut stream = res.bytes_stream();
                    let mut sse = SseParser::new();
                    raw_buffer.clear(); // 清空上次重试的缓冲区
                    let mut done = false;
                    let mut stream_eof = false;

                    loop {
                        if cancel.is_cancelled() {
                            send_aborted(&app_handle, &full_text, &full_thinking).await;
                            return;
                        }

                        let chunk_res = match tokio::time::timeout(
                            Duration::from_secs(25),
                            stream.next(),
                        )
                        .await
                        {
                            Ok(Some(chunk_res)) => chunk_res,
                            Ok(None) => break,
                            Err(_) => {
                                // 超时：如果有部分内容，保留它而非丢弃
                                if !turn_text.trim().is_empty() || !full_thinking.trim().is_empty()
                                {
                                    break;
                                }
                                send_error(
                                    &app_handle,
                                    "API 超过 25 秒没有返回新内容，已停止等待。".to_string(),
                                    &full_thinking,
                                )
                                .await;
                                return;
                            }
                        };

                        let chunk_bytes = match chunk_res {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                let message = e.to_string();
                                let content_len =
                                    turn_text.trim().len() + full_thinking.trim().len();
                                eprintln!(
                                    "[stream-error] content={}B recoverable={} err={}",
                                    content_len,
                                    is_recoverable_stream_error(&message),
                                    message
                                );
                                if is_recoverable_stream_error(&message) {
                                    // 早期流中断（总内容 < 100 字符）：
                                    // 重试比保留部分响应更有价值——中转服务经常在
                                    // 模型刚输出几个 token 时切断连接
                                    if content_len < 100 {
                                        eprintln!(
                                            "[stream-retry] early stream termination, will retry"
                                        );
                                        stream_eof = true;
                                        break;
                                    }
                                    // 已有较多内容 -> 保留
                                    break;
                                }
                                // 非可恢复错误：如果有部分内容，保留它而非丢弃
                                if !turn_text.trim().is_empty() || !full_thinking.trim().is_empty()
                                {
                                    break;
                                }
                                send_error(
                                    &app_handle,
                                    format!("流式读取失败: {message}"),
                                    &full_thinking,
                                )
                                .await;
                                return;
                            }
                        };

                        let chunk_str = String::from_utf8_lossy(&chunk_bytes);
                        raw_buffer.push_str(&chunk_str);
                        let data_lines = sse.push(&chunk_str);

                        for (event_type, data) in data_lines {
                            if data == "[DONE]" {
                                done = true;
                                break;
                            }

                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(message) = api_error_message_from_value(&value) {
                                    send_error(&app_handle, message, &full_thinking).await;
                                    return;
                                }
                            }

                            if config.api_mode == "responses" {
                                // ===== Responses API 流式解析 =====
                                let data_value: serde_json::Value =
                                    match serde_json::from_str(&data) {
                                        Ok(v) => v,
                                        Err(_) => continue,
                                    };
                                match event_type.as_str() {
                                    "response.output_item.added" => {
                                        // 记录输出项类型（保留用于后续扩展）
                                    }
                                    "response.output_text.delta" => {
                                        if let Some(text) =
                                            data_value.get("delta").and_then(|d| d.as_str())
                                        {
                                            if !text.is_empty() {
                                                turn_text.push_str(text);
                                                full_text.push_str(text);
                                                let _ = app_handle.emit(
                                                    "ai-answer-delta",
                                                    AnswerDeltaPayload {
                                                        text: text.to_string(),
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    "response.reasoning_summary_text.delta" => {
                                        if let Some(text) =
                                            data_value.get("delta").and_then(|d| d.as_str())
                                        {
                                            if !text.is_empty() {
                                                full_thinking.push_str(text);
                                                append_active_thinking(&state, text);
                                                let _ = app_handle.emit("ai-thinking", text);
                                            }
                                        }
                                    }
                                    "response.function_call_arguments.delta" => {
                                        let idx = data_value
                                            .get("output_index")
                                            .and_then(|i| i.as_u64())
                                            .unwrap_or(0)
                                            as usize;
                                        if let Some(delta) =
                                            data_value.get("delta").and_then(|d| d.as_str())
                                        {
                                            let entry = accumulated_tool_calls
                                                .entry(idx)
                                                .or_insert(AccumulatedToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                });
                                            entry.arguments.push_str(delta);
                                        }
                                    }
                                    "response.function_call_arguments.done" => {
                                        let idx = data_value
                                            .get("output_index")
                                            .and_then(|i| i.as_u64())
                                            .unwrap_or(0)
                                            as usize;
                                        if let Some(args) =
                                            data_value.get("arguments").and_then(|a| a.as_str())
                                        {
                                            let entry = accumulated_tool_calls
                                                .entry(idx)
                                                .or_insert(AccumulatedToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: String::new(),
                                                });
                                            // done 事件包含完整参数，覆盖累积值
                                            entry.arguments = args.to_string();
                                        }
                                    }
                                    "response.output_item.done" => {
                                        let idx = data_value
                                            .get("output_index")
                                            .and_then(|i| i.as_u64())
                                            .unwrap_or(0)
                                            as usize;
                                        if let Some(item) = data_value.get("item") {
                                            let item_type = item
                                                .get("type")
                                                .and_then(|t| t.as_str())
                                                .unwrap_or("");
                                            if item_type == "function_call" {
                                                let entry = accumulated_tool_calls
                                                    .entry(idx)
                                                    .or_insert(AccumulatedToolCall {
                                                        id: String::new(),
                                                        name: String::new(),
                                                        arguments: String::new(),
                                                    });
                                                if let Some(id) = item
                                                    .get("call_id")
                                                    .and_then(|c| c.as_str())
                                                    .or_else(|| {
                                                        item.get("id").and_then(|i| i.as_str())
                                                    })
                                                {
                                                    entry.id = id.to_string();
                                                }
                                                if let Some(name) =
                                                    item.get("name").and_then(|n| n.as_str())
                                                {
                                                    entry.name = name.to_string();
                                                }
                                                if let Some(args) =
                                                    item.get("arguments").and_then(|a| a.as_str())
                                                {
                                                    entry.arguments = args.to_string();
                                                }
                                            }
                                        }
                                    }
                                    "response.completed" | "response.done" | "response.failed" => {
                                        done = true;
                                    }
                                    _ => {}
                                }
                            } else {
                                // ===== Chat Completions 流式解析 =====
                                if let Ok(parsed) =
                                    serde_json::from_str::<ChatCompletionChunk>(&data)
                                {
                                    if let Some(choice) = parsed.choices.first() {
                                        if let Some(reasoning) = reasoning_delta_text(&choice.delta)
                                        {
                                            full_thinking.push_str(&reasoning);
                                            append_active_thinking(&state, &reasoning);
                                            let _ = app_handle.emit("ai-thinking", reasoning);
                                        }

                                        if let Some(ref text) = choice.delta.content {
                                            turn_text.push_str(text);
                                            full_text.push_str(text);
                                            let _ = app_handle.emit(
                                                "ai-answer-delta",
                                                AnswerDeltaPayload { text: text.clone() },
                                            );
                                        }

                                        if let Some(ref tool_calls) = choice.delta.tool_calls {
                                            for tc in tool_calls {
                                                let entry = accumulated_tool_calls
                                                    .entry(tc.index)
                                                    .or_insert(AccumulatedToolCall {
                                                        id: String::new(),
                                                        name: String::new(),
                                                        arguments: String::new(),
                                                    });

                                                if let Some(ref id) = tc.id {
                                                    entry.id.push_str(id);
                                                }
                                                if let Some(ref func) = tc.function {
                                                    if let Some(ref name) = func.name {
                                                        entry.name.push_str(name);
                                                    }
                                                    if let Some(ref args) = func.arguments {
                                                        entry.arguments.push_str(args);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if done {
                            break;
                        }
                    }

                    if done || !stream_eof {
                        stream_success = true;
                        break;
                    }
                    // stream_eof 且无内容 -> 继续重试
                }

                if !stream_success && turn_text.is_empty() && accumulated_tool_calls.is_empty() {
                    send_error(
                        &app_handle,
                        "流式连接多次中断，无法获取响应".to_string(),
                        &full_thinking,
                    )
                    .await;
                    return;
                }

                // auto fallback: 如果 SSE 解析无内容，尝试将原始响应当作 JSON 解析
                if turn_text.is_empty()
                    && accumulated_tool_calls.is_empty()
                    && !raw_buffer.is_empty()
                {
                    if let Some(message) = api_error_message_from_sse_text(&raw_buffer) {
                        send_error(&app_handle, message, &full_thinking).await;
                        return;
                    }
                    // HTML 响应检测
                    let buf_trimmed = raw_buffer.trim();
                    if buf_trimmed.starts_with("<!DOCTYPE")
                        || buf_trimmed.starts_with("<!doctype")
                        || buf_trimmed.starts_with("<html")
                        || buf_trimmed.starts_with("<HTML")
                    {
                        send_error(
                        &app_handle,
                        "API 返回了 HTML 页面而非 JSON 响应。请检查 Base URL 是否指向了正确的 API 端点（而非网关首页），以及 API Key 是否有效。".to_string(),
                        &full_thinking,
                    ).await;
                        return;
                    }

                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_buffer) {
                        use crate::direct_api::api_client::ContentExtractor;

                        // 提取内容
                        if let Some(content) = ContentExtractor::extract_content(&value) {
                            turn_text.push_str(&content);
                            full_text.push_str(&content);
                            let _ = app_handle
                                .emit("ai-answer-delta", AnswerDeltaPayload { text: content });
                        }

                        // 提取工具调用
                        let tool_calls = ContentExtractor::extract_tool_calls(&value);
                        for (i, tc) in tool_calls.iter().enumerate() {
                            accumulated_tool_calls.insert(
                                i,
                                AccumulatedToolCall {
                                    id: tc.id.clone().unwrap_or_default(),
                                    name: tc.function.name.clone(),
                                    arguments: tc.function.arguments.clone(),
                                },
                            );
                        }

                        // 提取 reasoning
                        if turn_text.is_empty() && accumulated_tool_calls.is_empty() {
                            if let Some(reasoning) = ContentExtractor::extract_reasoning(&value) {
                                full_thinking.push_str(&reasoning);
                            }
                        }
                    }

                    // 如果 JSON 解析也失败了，尝试从 raw_buffer 中提取 SSE data 行的文本内容
                    if turn_text.is_empty() && accumulated_tool_calls.is_empty() {
                        let mut extracted_text = String::new();
                        for line in raw_buffer.lines() {
                            let line = line.trim();
                            if let Some(data) = line.strip_prefix("data:") {
                                let data = data.trim();
                                if data == "[DONE]" {
                                    continue;
                                }
                                // 尝试解析 JSON，如果失败则跳过
                                if let Ok(parsed) =
                                    serde_json::from_str::<ChatCompletionChunk>(data)
                                {
                                    if let Some(choice) = parsed.choices.first() {
                                        if let Some(ref text) = choice.delta.content {
                                            extracted_text.push_str(text);
                                        }
                                    }
                                }
                            }
                        }
                        if !extracted_text.is_empty() {
                            turn_text = extracted_text.clone();
                            full_text.push_str(&extracted_text);
                            let _ = app_handle.emit(
                                "ai-answer-delta",
                                AnswerDeltaPayload {
                                    text: extracted_text,
                                },
                            );
                        }
                    }

                    // 策略4: 纯文本 fallback（raw_buffer 非空、非 HTML、非 JSON、且前面所有策略都没提取到内容）
                    if turn_text.is_empty() && accumulated_tool_calls.is_empty() {
                        let buf_trimmed = raw_buffer.trim();
                        if !buf_trimmed.is_empty()
                            && !buf_trimmed.starts_with('<')
                            && !buf_trimmed.starts_with('{')
                            && !buf_trimmed.starts_with('[')
                        {
                            turn_text = buf_trimmed.to_string();
                            full_text.push_str(buf_trimmed);
                            let _ = app_handle.emit(
                                "ai-answer-delta",
                                AnswerDeltaPayload {
                                    text: buf_trimmed.to_string(),
                                },
                            );
                        }
                    }
                }
            } // end of "auto" | _ branch
        }

        if !turn_text.is_empty() && !accumulated_tool_calls.is_empty() {
            full_thinking.push_str(&turn_text);
            full_thinking.push('\n');
        }

        if accumulated_tool_calls.is_empty() {
            break;
        }

        let mut ordered_tool_calls = accumulated_tool_calls.into_iter().collect::<Vec<_>>();
        ordered_tool_calls.sort_by_key(|(index, _)| *index);

        let mut assistant_tool_calls_json = Vec::new();
        for (_, tc) in &ordered_tool_calls {
            assistant_tool_calls_json.push(json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": tc.arguments
                }
            }));
        }

        api_messages.push(json!({
            "role": "assistant",
            "content": if turn_text.is_empty() { None } else { Some(turn_text.as_str()) },
            "tool_calls": assistant_tool_calls_json
        }));

        for (_, tc) in ordered_tool_calls {
            let tc_id = if tc.id.is_empty() {
                format!("tool_{}_{}", tc.name, crate::unix_now())
            } else {
                tc.id
            };
            let tc_name = tc.name;
            let tc_args_str = tc.arguments;
            let tc_args: serde_json::Value =
                serde_json::from_str(&tc_args_str).unwrap_or_else(|_| json!({}));
            let pretty_args =
                serde_json::to_string_pretty(&tc_args).unwrap_or_else(|_| tc_args_str.clone());

            emit_tool_event(
                &app_handle,
                &tc_id,
                &tc_name,
                "requested",
                &tc_args,
                None,
                None,
            );
            let starting_msg = format!(
                "\n[调用工具] {}\n{}\n",
                tool_summary(&tc_name, &tc_args),
                pretty_args
            );
            full_thinking.push_str(&starting_msg);
            append_active_thinking(&state, &starting_msg);
            let _ = app_handle.emit("ai-thinking", &starting_msg);

            if config.execution_mode == "plan" {
                let tool_output =
                    format!("计划模式已阻止执行工具 {tc_name}。请切换执行模式后再操作。");
                emit_tool_event(
                    &app_handle,
                    &tc_id,
                    &tc_name,
                    "skipped",
                    &tc_args,
                    Some(&tool_output),
                    Some(false),
                );
                full_thinking.push_str(&format!("[计划模式未执行]\n{}\n", tool_output));
                append_active_thinking(&state, &format!("[计划模式未执行]\n{}\n", tool_output));
                push_tool_message(&mut api_messages, &tc_id, &tc_name, &tool_output);
                continue;
            }

            // 🔥 ask_user 特殊分支: 不走确认/执行流程, 直接弹内嵌询问弹窗等用户作答。
            // 用户的回答会作为工具结果回传给模型, 让它据此决定后续方向。
            if tc_name == "ask_user" {
                let single_label = agent.as_ref().and_then(|ctx| ctx.single()).map(|a| a.name.clone());
                let tool_output = request_user_answer(
                    &state,
                    &app_handle,
                    &mut full_thinking,
                    &tc_id,
                    &tc_args,
                    single_label.as_deref(),
                    &cancel,
                )
                .await;

                let output_for_log = tool_output_for_log(&tool_output);
                let output_display = format!("[执行结果]\n{}\n", output_for_log);
                full_thinking.push_str(&output_display);
                append_active_thinking(&state, &output_display);
                let _ = app_handle.emit("ai-thinking", &output_display);
                push_tool_message(&mut api_messages, &tc_id, &tc_name, &tool_output);
                append_visual_tool_message(&mut api_messages, &tc_name, &tool_output);
                continue;
            }

            let already_approved = {
                let approved_tool_types = state.approved_tool_types.lock().await;
                approved_tool_types.contains(&tc_name)
            };
            // 🔥 关键修复: 每次 tool call 前从 state 读最新 config,
            // 而不是用 turn 开头传入的快照 (用户在思考途中切无审查模式会立即生效)
            let latest_config = state.direct_api_config.lock().await.clone();
            let effective_config = latest_config.as_ref().unwrap_or(&config);
            let requires_confirm =
                mode_requires_confirmation(effective_config, &tc_name, already_approved);
            let focus_snapshot = if requires_confirm
                && crate::computer_use::should_restore_focus_after_confirmation(&tc_name)
            {
                crate::computer_use::capture_foreground_window().await
            } else {
                None
            };

            let approved = if requires_confirm {
                let single_label = agent
                    .as_ref()
                    .and_then(|ctx| ctx.single())
                    .map(|a| a.name.clone());
                request_tool_confirmation(
                    &state,
                    &app_handle,
                    &mut full_thinking,
                    &tc_id,
                    &tc_name,
                    &tc_args,
                    &pretty_args,
                    single_label.as_deref(),
                    &cancel,
                )
                .await
            } else {
                true
            };

            let tool_output = if approved {
                if focus_snapshot.is_some() {
                    crate::computer_use::restore_foreground_window(focus_snapshot.as_ref()).await;
                }
                emit_tool_event(
                    &app_handle,
                    &tc_id,
                    &tc_name,
                    "running",
                    &tc_args,
                    None,
                    Some(true),
                );
                let output =
                    execute_tool(&tc_name, &tc_args, &config.search_provider, cancel.clone()).await;
                let output_for_event = tool_output_for_log(&output);
                emit_tool_event(
                    &app_handle,
                    &tc_id,
                    &tc_name,
                    "completed",
                    &tc_args,
                    Some(&output_for_event),
                    Some(true),
                );

                if tc_name == "create_scheduled_task" || tc_name == "list_scheduled_tasks" {
                    let _ = app_handle.emit("scheduled-tasks-changed", json!({}));
                }
                if tc_name == "save_memory" || tc_name == "delete_memory" {
                    let _ = app_handle.emit("memories-changed", json!({}));
                }

                output
            } else {
                "用户拒绝了此工具的操作权限。请说明为什么需要此权限，并让用户重新发起或授权。"
                    .to_string()
            };

            let output_for_log = tool_output_for_log(&tool_output);
            let output_for_message = tool_output_for_message(&tool_output);
            let output_display = format!("[执行结果]\n{}\n", output_for_log);
            full_thinking.push_str(&output_display);
            append_active_thinking(&state, &output_display);
            let _ = app_handle.emit("ai-thinking", &output_display);
            push_tool_message(&mut api_messages, &tc_id, &tc_name, &output_for_message);
            append_visual_tool_message(&mut api_messages, &tc_name, &tool_output);
        }
    }

    // 保存前再次检查是否已取消
    if cancel.is_cancelled() {
        send_aborted(&app_handle, &full_text, &full_thinking).await;
        return;
    }

    if full_text.trim().is_empty() {
        // 根据配置和响应情况提供更具体的错误信息
        let error_msg = if config.stream_mode == "non_stream" {
            // 非流式模式已经经过ContentExtractor，错误信息已经具体化
            "API 请求结束了，但没有返回可显示的回复内容。请检查模型配置和接口返回。".to_string()
        } else {
            // 流式/auto模式：可能是SSE格式问题
            "接口返回了HTTP 200，但响应不是OpenAI SSE格式。如果使用的是Auto Code等非标准接口，请在设置中将流式模式改为non_stream。".to_string()
        };

        send_error(&app_handle, error_msg, &full_thinking).await;
        return;
    }

    let single_agent = agent.as_ref().and_then(|ctx| ctx.single());
    {
        let db = state.db.lock().await;
        let _ = db.save_message_with_agent(
            "assistant",
            &full_text,
            if full_thinking.is_empty() {
                None
            } else {
                Some(&full_thinking)
            },
            single_agent.map(|a| a.id.as_str()),
            single_agent.map(|a| a.name.as_str()),
            single_agent.map(|a| a.avatar.as_str()),
        );
    }

    {
        let mut behavior = state.behavior.lock().await;
        behavior.set_state(crate::behavior::PetState::Speaking);
        behavior.mood.update(Some(0.05), None);
    }

    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }

    let _ = app_handle.emit(
        "ai-finished",
        crate::AiFinishedPayload {
            text: full_text,
            thinking: if full_thinking.is_empty() {
                None
            } else {
                Some(full_thinking)
            },
            agent_id: single_agent.map(|a| a.id.clone()),
            agent_name: single_agent.map(|a| a.name.clone()),
            agent_avatar: single_agent.map(|a| a.avatar.clone()),
        },
    );
}

async fn build_user_content(message: &str, attachments: &[UserAttachment]) -> Value {
    let image_attachments = attachments.iter().filter(|attachment| {
        attachment.is_image || image_mime_for_attachment(attachment).is_some()
    });
    let mut image_parts = Vec::new();
    let mut warnings = Vec::new();

    for attachment in image_attachments {
        if image_parts.len() >= MAX_VISION_IMAGES {
            warnings.push(format!(
                "{} 未作为视觉输入发送：最多支持同时识别 {MAX_VISION_IMAGES} 张图片",
                attachment_display_name(attachment)
            ));
            continue;
        }

        match attachment_to_image_url(attachment).await {
            Ok(url) => {
                image_parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                }));
            }
            Err(warning) => warnings.push(warning),
        }
    }

    let text = append_attachment_warnings(message, &warnings);
    if image_parts.is_empty() {
        return Value::String(text);
    }

    let mut content = vec![json!({
        "type": "text",
        "text": text
    })];
    content.extend(image_parts);
    Value::Array(content)
}

fn append_attachment_warnings(message: &str, warnings: &[String]) -> String {
    if warnings.is_empty() {
        return message.to_string();
    }

    format!("{}\n\n[系统提示：{}]", message, warnings.join("；"))
}

/// 把用户引用的前文对话拼装进发送给模型的用户消息
fn apply_quote_context(message: &str, quote: Option<&(String, String)>) -> String {
    let Some((quoted_role, quoted_content)) = quote else {
        return message.to_string();
    };
    let speaker = if quoted_role == "assistant" {
        "助手（你）"
    } else {
        "用户"
    };
    let body = if message.trim().is_empty() {
        "（用户仅发送了引用，没有输入其他文字）".to_string()
    } else {
        message.to_string()
    };
    format!(
        "[引用回复]\n被引用的原文（说话者：{speaker}）：\n{quoted_content}\n\n用户针对这条引用的输入：\n{body}"
    )
}

fn assistant_history_content_for_context(
    msg: &ChatMessage,
    agent: Option<&crate::AgentContext>,
) -> String {
    let content = msg.content.as_str();
    let thinking = msg.thinking.as_deref();

    // 带智能体归属的历史消息：前缀 【名字】，让模型知道每句话是谁说的。
    let content = match (agent, msg.agent_name.as_deref()) {
        (Some(_), Some(name)) if !name.trim().is_empty() => format!("【{name}】\n{content}"),
        _ => content.to_string(),
    };

    let Some(thinking) = thinking else {
        return content;
    };
    if content.contains("工具进度摘要") {
        return content;
    }
    let should_restore_progress = content.contains("[执行中断]")
        || content.contains("[思考中，已被中止]")
        || content.contains("已达到最大迭代限制")
        || content.contains("⚠️");
    if !should_restore_progress {
        return content;
    }
    let Some(progress) = compact_execution_progress_for_context(thinking) else {
        return content;
    };
    format!("{content}\n\n[上次工具进度摘要]\n{progress}")
}

fn turn_limit_message(limit: u32, thinking: &str) -> String {
    let mut message = format!(
        "\n[已达到最大迭代限制 {limit} 轮，已保存当前工具进度。请直接发送“继续”，AI 会根据进度和新的屏幕状态接着执行。]"
    );
    if let Some(progress) = compact_execution_progress_for_context(thinking) {
        message.push_str("\n\n[当前工具进度摘要]\n");
        message.push_str(&progress);
    }
    message
}

fn interrupted_message_with_progress(base: &str, thinking: &str) -> String {
    if base.contains("工具进度摘要") {
        return base.to_string();
    }
    let Some(progress) = compact_execution_progress_for_context(thinking) else {
        return base.to_string();
    };
    format!("{base}\n\n[当前工具进度摘要]\n{progress}")
}

fn compact_execution_progress_for_context(thinking: &str) -> Option<String> {
    const MAX_PROGRESS_CHARS: usize = 8000;
    let mut lines = Vec::new();
    let mut capture_remaining = 0usize;

    for raw_line in thinking.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let starts_block = line.starts_with("[调用工具]")
            || line.starts_with("[执行结果]")
            || line.starts_with("[计划模式未执行]")
            || line.starts_with("[等待授权]")
            || line.starts_with("[用户已授权")
            || line.starts_with("[用户拒绝")
            || line.starts_with("[确认超时")
            || line.starts_with("[授权通道");

        if starts_block {
            lines.push(line.to_string());
            capture_remaining = if line.starts_with("[执行结果]") {
                12
            } else {
                8
            };
            continue;
        }

        if capture_remaining > 0 {
            lines.push(line.to_string());
            capture_remaining -= 1;
        }
    }

    if lines.is_empty() {
        return None;
    }

    let progress = lines.join("\n");
    Some(compact_tail_chars(&progress, MAX_PROGRESS_CHARS))
}

fn compact_tail_chars(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_string();
    }
    let tail = input
        .chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect::<String>();
    format!("... [已省略较早工具进度]\n{tail}")
}

async fn attachment_to_image_url(attachment: &UserAttachment) -> Result<String, String> {
    let mime = image_mime_for_attachment(attachment).ok_or_else(|| {
        format!(
            "{} 未作为视觉输入发送：当前仅支持 PNG、JPEG、WEBP、GIF 图片",
            attachment_display_name(attachment)
        )
    })?;

    if attachment.path.trim().is_empty() {
        return Err(format!(
            "{} 未作为视觉输入发送：缺少本地路径",
            attachment_display_name(attachment)
        ));
    }

    let metadata = tokio::fs::metadata(&attachment.path)
        .await
        .map_err(|e| format!("{} 无法读取：{e}", attachment_display_name(attachment)))?;
    if !metadata.is_file() {
        return Err(format!(
            "{} 未作为视觉输入发送：路径不是文件",
            attachment_display_name(attachment)
        ));
    }
    if metadata.len() > MAX_VISION_IMAGE_BYTES {
        return Err(format!(
            "{} 未作为视觉输入发送：图片超过 50MB",
            attachment_display_name(attachment)
        ));
    }

    let bytes = tokio::fs::read(&attachment.path)
        .await
        .map_err(|e| format!("{} 读取失败：{e}", attachment_display_name(attachment)))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn image_mime_for_attachment(attachment: &UserAttachment) -> Option<&'static str> {
    match attachment_extension(attachment).as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn attachment_extension(attachment: &UserAttachment) -> String {
    let explicit = attachment.extension.trim().trim_start_matches('.');
    if !explicit.is_empty() {
        return explicit.to_lowercase();
    }

    Path::new(&attachment.path)
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn attachment_display_name(attachment: &UserAttachment) -> String {
    let name = attachment.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }

    Path::new(&attachment.path)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "图片附件".to_string())
}

fn reasoning_delta_text(delta: &crate::direct_api::api_client::Delta) -> Option<String> {
    if let Some(text) = delta.reasoning_content.as_deref() {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }

    value_to_text(delta.reasoning.as_ref()).or_else(|| value_to_text(delta.thinking.as_ref()))
}

fn value_to_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Object(map) => map
            .get("content")
            .or_else(|| map.get("text"))
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty())
            .map(|text| text.to_string()),
        _ => None,
    }
}

fn is_recoverable_stream_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // 原有的 chunk EOF 匹配
    (lower.contains("unexpected eof") && lower.contains("chunk"))
        // 连接被对端重置
        || lower.contains("connection reset")
        // 管道断裂
        || lower.contains("broken pipe")
        // 连接被关闭（通用）
        || lower.contains("connection closed")
        // hyper 特定：响应体未接收完成就被关闭
        || lower.contains("before message completed")
        // 通用 EOF (body 相关)
        || (lower.contains("unexpected eof") && lower.contains("body"))
        // TLS 握手阶段 EOF
        || (lower.contains("unexpected eof") && lower.contains("handshake"))
        // 远端主动关闭
        || lower.contains("peer closed")
        // reqwest 传输层错误
        || lower.contains("error decoding response body")
        // 请求中途被取消（区分于用户主动中止）
        || lower.contains("request was canceled")
}

pub(crate) fn append_active_thinking(state: &tauri::State<'_, AppState>, text: &str) {
    if let Ok(mut active) = state.active_chat.lock() {
        if let Some(active) = active.active.as_mut() {
            active.thinking.push_str(text);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn request_tool_confirmation(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    full_thinking: &mut String,
    event_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    pretty_args: &str,
    agent_label: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> bool {
    let prefix = agent_label
        .map(|name| format!("【{name}】"))
        .unwrap_or_default();
    let waiting_msg = format!("{prefix}[等待授权] {}\n", tool_summary(tool_name, args));
    full_thinking.push_str(&waiting_msg);
    append_active_thinking(state, &waiting_msg);
    let _ = app_handle.emit("ai-thinking", &waiting_msg);
    emit_tool_event(app_handle, event_id, tool_name, "waiting", args, None, None);

    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    // 用 UUID 防止同秒并发的确认 ID 碰撞（多智能体并行时会出现同时待确认）
    let confirm_id = format!("confirm_{}", uuid::Uuid::new_v4().simple());
    {
        let mut confirms = state.pending_confirms.lock().await;
        confirms.insert(confirm_id.clone(), tx);
    }

    let _ = app_handle.emit(
        "ai-tool-confirm",
        ToolConfirmPayload {
            id: confirm_id.clone(),
            tool_name: tool_name.to_string(),
            arguments: pretty_args.to_string(),
            summary: format!("{prefix}{}", tool_summary(tool_name, args)),
            command: tool_command(tool_name, args),
            path: tool_path(tool_name, args),
            agent_name: agent_label.map(str::to_string),
            agent_avatar: None,
        },
    );

    // 等待用户授权 / 120 秒超时 / 本次运行被中止
    let approved_result = tokio::select! {
        result = tokio::time::timeout(std::time::Duration::from_secs(120), rx) => result,
        _ = cancel.cancelled() => {
            let _ = app_handle.emit(
                "ai-tool-confirm-resolved",
                json!({ "id": confirm_id.clone(), "approved": false }),
            );
            let mut confirms = state.pending_confirms.lock().await;
            confirms.remove(&confirm_id);
            let aborted_msg = format!("{prefix}[运行已中止，取消此工具授权]\n");
            full_thinking.push_str(&aborted_msg);
            append_active_thinking(state, &aborted_msg);
            let _ = app_handle.emit("ai-thinking", &aborted_msg);
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "denied",
                args,
                Some("运行已中止。"),
                Some(false),
            );
            return false;
        }
    };

    {
        let mut confirms = state.pending_confirms.lock().await;
        confirms.remove(&confirm_id);
    }

    let emit_resolved = |approved: bool| {
        let _ = app_handle.emit(
            "ai-tool-confirm-resolved",
            json!({ "id": confirm_id.clone(), "approved": approved }),
        );
    };

    match approved_result {
        Ok(Ok(true)) => {
            emit_resolved(true);
            let approved_msg = format!("{prefix}[用户已授权，开始执行]\n");
            full_thinking.push_str(&approved_msg);
            append_active_thinking(state, &approved_msg);
            let _ = app_handle.emit("ai-thinking", &approved_msg);
            {
                let mut approved_tool_types = state.approved_tool_types.lock().await;
                if !is_high_risk_tool(tool_name) {
                    approved_tool_types.insert(tool_name.to_string());
                }
            }
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "approved",
                args,
                None,
                Some(true),
            );
            true
        }
        Ok(Ok(false)) => {
            emit_resolved(false);
            let denied_msg = format!("{prefix}[用户拒绝执行此操作]\n");
            full_thinking.push_str(&denied_msg);
            append_active_thinking(state, &denied_msg);
            let _ = app_handle.emit("ai-thinking", &denied_msg);
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "denied",
                args,
                Some("用户拒绝执行此操作。"),
                Some(false),
            );
            false
        }
        Ok(Err(_)) => {
            emit_resolved(false);
            let err_msg = "[授权通道失效]\n".to_string();
            full_thinking.push_str(&err_msg);
            append_active_thinking(state, &err_msg);
            let _ = app_handle.emit("ai-thinking", &err_msg);
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "denied",
                args,
                Some("授权通道失效。"),
                Some(false),
            );
            false
        }
        Err(_) => {
            emit_resolved(false);
            let timeout_msg = "[确认超时 (120秒)，自动拒绝]\n".to_string();
            full_thinking.push_str(&timeout_msg);
            append_active_thinking(state, &timeout_msg);
            let _ = app_handle.emit("ai-thinking", &timeout_msg);
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "denied",
                args,
                Some("确认超时，自动拒绝。"),
                Some(false),
            );
            false
        }
    }
}

pub(crate) fn push_tool_message(
    api_messages: &mut Vec<serde_json::Value>,
    tool_call_id: &str,
    tool_name: &str,
    content: &str,
) {
    api_messages.push(json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "name": tool_name,
        "content": content
    }));
}

/// ask_user 工具处理: 向前端弹内嵌询问弹窗, 等待用户选择选项或填写自定义答案。
/// 返回的字符串会作为工具结果回传给模型。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn request_user_answer(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    full_thinking: &mut String,
    event_id: &str,
    args: &serde_json::Value,
    agent_label: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> String {
    let prefix = agent_label
        .map(|name| format!("【{name}】"))
        .unwrap_or_default();
    let question = args["question"]
        .as_str()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "智能体想向你确认一个问题。".to_string());
    let options = args["options"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let label = item["label"].as_str()?.trim().to_string();
                    if label.is_empty() {
                        return None;
                    }
                    Some(AskUserOption {
                        description: item["description"].as_str().map(|value| value.to_string()),
                        label,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let waiting_msg = format!("[等待用户回答] {}\n", question);
    full_thinking.push_str(&waiting_msg);
    append_active_thinking(state, &waiting_msg);
    let _ = app_handle.emit("ai-thinking", &waiting_msg);
    emit_tool_event(app_handle, event_id, "ask_user", "waiting", args, None, None);

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let question_id = format!("ask_{}", uuid::Uuid::new_v4().simple());
    {
        let mut questions = state.pending_questions.lock().await;
        questions.insert(question_id.clone(), tx);
    }

    let _ = app_handle.emit(
        "ai-ask-user",
        AskUserPayload {
            id: question_id.clone(),
            question: format!("{prefix}{question}"),
            options: options.clone(),
            agent_name: agent_label.map(str::to_string),
            agent_avatar: None,
        },
    );

    // 同时监听: 用户回答 / 5 分钟超时 / 用户中止整个对话
    let answer_result = tokio::select! {
        result = rx => Some(result),
        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => None,
        _ = cancel.cancelled() => {
            let _ = app_handle.emit(
                "ai-ask-user-resolved",
                json!({ "id": question_id, "answered": false }),
            );
            {
                let mut questions = state.pending_questions.lock().await;
                questions.remove(&question_id);
            }
            emit_tool_event(
                app_handle,
                event_id,
                "ask_user",
                "denied",
                args,
                Some("操作已被用户中止。"),
                Some(false),
            );
            return "操作已被用户中止。".to_string();
        }
    };

    {
        let mut questions = state.pending_questions.lock().await;
        questions.remove(&question_id);
    }

    match answer_result {
        Some(Ok(answer)) => {
            let _ = app_handle.emit(
                "ai-ask-user-resolved",
                json!({ "id": question_id, "answered": true }),
            );
            let display_answer = if answer.trim().is_empty() {
                "（跳过，未作答）".to_string()
            } else {
                answer.trim().to_string()
            };
            let answered_msg = format!("[用户已回答] {}\n", display_answer);
            full_thinking.push_str(&answered_msg);
            append_active_thinking(state, &answered_msg);
            let _ = app_handle.emit("ai-thinking", &answered_msg);
            emit_tool_event(
                app_handle,
                event_id,
                "ask_user",
                "completed",
                args,
                Some(&display_answer),
                Some(true),
            );
            if answer.trim().is_empty() {
                "用户跳过了这个问题（未作答）。请根据已有信息按你的最佳判断继续，不必再次询问。"
                    .to_string()
            } else {
                format!("用户的回答：{}", answer.trim())
            }
        }
        Some(Err(_)) => {
            // oneshot 发送端被丢弃 (通道失效)
            let _ = app_handle.emit(
                "ai-ask-user-resolved",
                json!({ "id": question_id, "answered": false }),
            );
            emit_tool_event(
                app_handle,
                event_id,
                "ask_user",
                "denied",
                args,
                Some("询问通道失效。"),
                Some(false),
            );
            "询问通道失效，未能取得用户回答。请根据已有信息按你的最佳判断继续。".to_string()
        }
        None => {
            // 5 分钟超时: 用户不在, 让智能体自主决策而不是无限等待
            let _ = app_handle.emit(
                "ai-ask-user-resolved",
                json!({ "id": question_id, "answered": false }),
            );
            let timeout_msg = "[用户未在 5 分钟内回答，继续自主处理]\n".to_string();
            full_thinking.push_str(&timeout_msg);
            append_active_thinking(state, &timeout_msg);
            let _ = app_handle.emit("ai-thinking", &timeout_msg);
            emit_tool_event(
                app_handle,
                event_id,
                "ask_user",
                "completed",
                args,
                Some("用户未作答（超时）。"),
                Some(true),
            );
            "用户未在 5 分钟内回答这个问题。请根据已有信息按你的最佳判断继续，并在最终回复中说明你的假设，方便用户事后纠正。"
                .to_string()
        }
    }
}

pub(crate) fn tool_output_for_log(output: &str) -> String {
    redact_visual_payload(output).unwrap_or_else(|| output.to_string())
}

pub(crate) fn tool_output_for_message(output: &str) -> String {
    redact_visual_payload(output).unwrap_or_else(|| output.to_string())
}

fn redact_visual_payload(output: &str) -> Option<String> {
    let mut value = serde_json::from_str::<Value>(output).ok()?;
    let image_url = value
        .get("image_url")
        .and_then(|item| item.as_str())
        .filter(|item| item.starts_with("data:image/"))?;

    let image_bytes = image_url.len();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "image_url".to_string(),
            json!(format!(
                "[attached image omitted: {image_bytes} bytes data URL]"
            )),
        );
        object.insert("image_attached".to_string(), json!(true));
    }

    serde_json::to_string_pretty(&value).ok()
}

pub(crate) fn append_visual_tool_message(
    api_messages: &mut Vec<serde_json::Value>,
    tool_name: &str,
    output: &str,
) {
    if !matches!(tool_name, "computer_screenshot" | "browser_snapshot") {
        return;
    }

    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return;
    };
    let Some(image_url) = value.get("image_url").and_then(|item| item.as_str()) else {
        return;
    };
    if !image_url.starts_with("data:image/") {
        return;
    }

    let summary = value
        .get("summary")
        .and_then(|item| item.as_str())
        .unwrap_or("Screenshot captured.");
    let width = value
        .get("width")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    let height = value
        .get("height")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    let image_width = value
        .get("image_width")
        .and_then(|item| item.as_i64())
        .unwrap_or(width);
    let image_height = value
        .get("image_height")
        .and_then(|item| item.as_i64())
        .unwrap_or(height);
    let screen_x = value
        .get("screen_x")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    let screen_y = value
        .get("screen_y")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    let display = value
        .get("display")
        .and_then(|item| item.as_i64())
        .unwrap_or(0);
    let scale_x = value
        .get("scale_x")
        .and_then(|item| item.as_f64())
        .unwrap_or(1.0);
    let scale_y = value
        .get("scale_y")
        .and_then(|item| item.as_f64())
        .unwrap_or(1.0);

    api_messages.push(json!({
        "role": "user",
        "content": [
            {
                "type": "text",
                "text": format!(
                    "{summary}\nUse this screenshot to decide the next UI action. Screenshot image size: {image_width}x{image_height}. Captured display: {display}. Captured screen area: origin ({screen_x},{screen_y}), original size {width}x{height}. Image-to-screen scale: x={scale_x:.4}, y={scale_y:.4}. For mouse actions based on this screenshot, prefer computer_mouse with coordinate_space=\"image\" and image x/y coordinates; the tool maps them to screen coordinates automatically."
                )
            },
            {
                "type": "image_url",
                "image_url": { "url": image_url }
            }
        ]
    }));
}

pub(crate) fn mode_requires_confirmation(
    config: &DirectApiConfig,
    tool_name: &str,
    already_approved: bool,
) -> bool {
    if already_approved {
        return false;
    }
    if config.execution_mode == "unreviewed" {
        return false;
    }
    if matches!(
        tool_name,
        "computer_mouse"
            | "computer_keyboard"
            | "window_focus"
            | "browser_open"
            | "browser_navigate"
            | "browser_click"
            | "browser_type"
            | "browser_press"
            | "browser_scroll"
            | "browser_back"
            | "browser_forward"
            | "browser_close"
    ) {
        return true;
    }
    // read_file、web_search、get_weather 和 list_scheduled_tasks 为安全/只读工具，在任何模式下均不需要弹窗确认；
    // 敏感工具如 run_command 和 open_app 需要等待确认（open_app 启动本地应用，具有敏感性）。
    // ask_user 本身就是询问用户，弹窗即是它的功能，无需再叠加确认流程。
    // code_search 是只读的内容搜索，与 read_file 同级。
    // edit_file 不在此列（写操作需确认一次），但也不在 is_high_risk_tool 中——确认一次后会话内免重复弹窗，保证调试循环流畅。
    if tool_name == "read_file"
        || tool_name == "web_search"
        || tool_name == "get_weather"
        || tool_name == "list_scheduled_tasks"
        || tool_name == "search_memory"
        || tool_name == "save_memory"
        || tool_name == "ask_user"
        || tool_name == "code_search"
    {
        return false;
    }
    // 免确认工具白名单（包含智能体配置的 allowed_tools）：命中时直接放行
    if config
        .auto_approved_tools
        .iter()
        .any(|tool| tool == tool_name)
    {
        return false;
    }
    match config.execution_mode.as_str() {
        "plan" | "unreviewed" => false,
        _ => true,
    }
}

pub(crate) fn emit_tool_event(
    app_handle: &tauri::AppHandle,
    id: &str,
    tool_name: &str,
    status: &str,
    args: &serde_json::Value,
    output: Option<&str>,
    approved: Option<bool>,
) {
    emit_tool_event_labeled(
        app_handle,
        id,
        tool_name,
        status,
        args,
        output,
        approved,
        None,
    );
}

/// 带智能体标签的工具事件：多智能体并行时区分事件归属。
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_tool_event_labeled(
    app_handle: &tauri::AppHandle,
    id: &str,
    tool_name: &str,
    status: &str,
    args: &serde_json::Value,
    output: Option<&str>,
    approved: Option<bool>,
    agent_name: Option<&str>,
) {
    let arguments = serde_json::to_string_pretty(args).unwrap_or_else(|_| "{}".to_string());
    let prefix = agent_name
        .map(|name| format!("【{name}】"))
        .unwrap_or_default();
    let _ = app_handle.emit(
        "ai-tool-event",
        ToolEventPayload {
            id: id.to_string(),
            tool_name: tool_name.to_string(),
            status: status.to_string(),
            summary: format!("{prefix}{}", tool_summary(tool_name, args)),
            arguments,
            command: tool_command(tool_name, args),
            path: tool_path(tool_name, args),
            output: output.map(|value| compact_for_event(value, 3000)),
            approved,
            agent_name: agent_name.map(str::to_string),
        },
    );
}

fn compact_for_event(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut value = input.chars().take(max_chars).collect::<String>();
    value.push_str("\n... [已截断]");
    value
}

/// 工具被批准一次后是否仍要求每次都弹窗确认。
/// 高风险（命令执行、文件写入、外部程序）默认不缓存：每次都让用户看一遍。
/// 低风险（鼠标键盘、读取）可缓存：避免连续操作时弹窗淹没自动化。
pub(crate) fn is_high_risk_tool(tool_name: &str) -> bool {
    matches!(tool_name, "run_command" | "write_file" | "open_app")
}

pub(crate) fn tool_command(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "run_command" => args["command"].as_str().map(|value| value.to_string()),
        "open_app" => {
            let app = args["app"].as_str()?;
            let extra = args["args"].as_str().unwrap_or("").trim();
            if extra.is_empty() {
                Some(app.to_string())
            } else {
                Some(format!("{app} {extra}"))
            }
        }
        "computer_mouse" => args["action"].as_str().map(|action| action.to_string()),
        "computer_keyboard" => args["action"].as_str().map(|action| action.to_string()),
        "browser_open" | "browser_navigate" => args["url"].as_str().map(|url| url.to_string()),
        "browser_type" => args["text"].as_str().map(|text| text.to_string()),
        "browser_press" => args["key"].as_str().map(|key| key.to_string()),
        "browser_click" => args["selector"]
            .as_str()
            .map(|selector| selector.to_string())
            .or_else(|| {
                let x = args["x"].as_f64()?;
                let y = args["y"].as_f64()?;
                Some(format!("坐标 ({x:.0}, {y:.0})"))
            }),
        "browser_scroll" => args["direction"].as_str().map(|d| d.to_string()),
        "window_focus" => args["title"]
            .as_str()
            .map(|value| value.to_string())
            .or_else(|| args["pid"].as_i64().map(|pid| format!("pid {pid}"))),
        _ => None,
    }
}

pub(crate) fn tool_path(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "read_file" | "write_file" | "edit_file" | "list_directory" => {
            args["path"].as_str().map(|value| value.to_string())
        }
        "file_search" => args["directory"].as_str().map(|value| value.to_string()),
        "browser_open" | "browser_navigate" => args["url"].as_str().map(|value| value.to_string()),
        _ => None,
    }
}

pub(crate) fn tool_summary(tool_name: &str, args: &serde_json::Value) -> String {
    match tool_name {
        "read_file" => format!("读取文件 {}", args["path"].as_str().unwrap_or("未知路径")),
        "write_file" => format!("写入文件 {}", args["path"].as_str().unwrap_or("未知路径")),
        "list_directory" => {
            format!("列出目录 {}", args["path"].as_str().unwrap_or("未知路径"))
        }
        "run_command" => {
            format!(
                "执行命令 {}",
                args["command"].as_str().unwrap_or("未知命令")
            )
        }
        "web_search" => format!("搜索网页 {}", args["query"].as_str().unwrap_or("未知查询")),
        "get_weather" => {
            if let Some(location) = args["location"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
            {
                format!("查询天气 {}", location)
            } else {
                "查询当前天气".to_string()
            }
        }
        "open_app" => format!("打开应用 {}", args["app"].as_str().unwrap_or("未知应用")),
        "computer_screenshot" => "查看桌面截图".to_string(),
        "computer_mouse" => format!("控制鼠标 {}", args["action"].as_str().unwrap_or("unknown")),
        "computer_keyboard" => format!("输入键盘 {}", args["action"].as_str().unwrap_or("unknown")),
        "computer_wait" => format!("等待 {} ms", args["ms"].as_i64().unwrap_or(500)),
        "window_list" => "列出桌面窗口".to_string(),
        "window_focus" => {
            if let Some(title) = args["title"].as_str() {
                format!("聚焦窗口 {title}")
            } else {
                format!("聚焦窗口 pid {}", args["pid"].as_i64().unwrap_or(0))
            }
        }
        "browser_open" => format!("打开浏览器 {}", args["url"].as_str().unwrap_or("默认首页")),
        "browser_navigate" => format!("浏览器导航 {}", args["url"].as_str().unwrap_or("未知 URL")),
        "browser_snapshot" => "查看浏览器截图".to_string(),
        "browser_extract" => "读取浏览器页面内容".to_string(),
        "browser_click" => {
            if let Some(selector) = args["selector"].as_str() {
                format!("浏览器点击 {}", selector)
            } else {
                format!(
                    "浏览器点击 ({}, {})",
                    args["x"].as_f64().unwrap_or(0.0) as i64,
                    args["y"].as_f64().unwrap_or(0.0) as i64
                )
            }
        }
        "browser_type" => format!("浏览器输入 {}", args["text"].as_str().unwrap_or("未知文本")),
        "browser_press" => format!("浏览器按键 {}", args["key"].as_str().unwrap_or("未知按键")),
        "browser_scroll" => format!(
            "浏览器滚动 {}",
            args["direction"].as_str().unwrap_or("down")
        ),
        "browser_back" => "浏览器返回上一页".to_string(),
        "browser_forward" => "浏览器前进下一页".to_string(),
        "browser_status" => "查询浏览器状态".to_string(),
        "browser_close" => "关闭浏览器".to_string(),
        "create_scheduled_task" => {
            format!(
                "创建定时任务 {}",
                args["title"].as_str().unwrap_or("未命名任务")
            )
        }
        "list_scheduled_tasks" => "列出定时任务".to_string(),
        "search_memory" => format!("搜索记忆 {}", args["query"].as_str().unwrap_or("未知查询")),
        "save_memory" => format!(
            "保存记忆 [{}] {}",
            args["category"].as_str().unwrap_or("未分类"),
            args["key"].as_str().unwrap_or("无标题")
        ),
        "delete_memory" => format!("删除记忆 #{}", args["id"].as_i64().unwrap_or(0)),
        "code_search" => format!(
            "搜索代码内容 '{}'{}",
            args["query"].as_str().unwrap_or("未知内容"),
            args["directory"]
                .as_str()
                .map(|d| format!(" 在 {}", d))
                .unwrap_or_default()
        ),
        "edit_file" => format!(
            "修改文件 {}",
            args["path"].as_str().unwrap_or("未知路径")
        ),
        "create_docx" => format!(
            "生成 Word 文档 {}",
            args["path"].as_str().unwrap_or("未指定路径")
        ),
        "ask_user" => format!(
            "向用户提问: {}",
            args["question"].as_str().unwrap_or("未指定问题")
        ),
        "file_search" => format!(
            "搜索文件 '{}'{}",
            args["pattern"].as_str().unwrap_or("*"),
            args["directory"]
                .as_str()
                .map(|d| format!(" 在 {}", d))
                .unwrap_or_default()
        ),
        _ => format!("调用工具 {tool_name}"),
    }
}

async fn send_error(app_handle: &tauri::AppHandle, err_msg: String, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(crate::behavior::PetState::Confused);
    drop(behavior);

    {
        let saved_text =
            interrupted_message_with_progress(&format!("[执行中断] {err_msg}"), full_thinking);
        let db = state.db.lock().await;
        let _ = db.save_message_with_thinking(
            "assistant",
            &saved_text,
            if full_thinking.trim().is_empty() {
                None
            } else {
                Some(full_thinking)
            },
        );
    }

    let _ = app_handle.emit(
        "ai-error",
        json!({
            "message": err_msg,
            "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
            "aborted": false
        }),
    );
}

async fn send_aborted(app_handle: &tauri::AppHandle, full_text: &str, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(crate::behavior::PetState::Idle);
    drop(behavior);

    // 有部分内容时保存到数据库，使"继续"时 AI 能接续
    if !full_text.trim().is_empty() || !full_thinking.trim().is_empty() {
        let db = state.db.lock().await;
        let saved_text = if full_text.trim().is_empty() {
            interrupted_message_with_progress("[思考中，已被中止]", full_thinking)
        } else {
            interrupted_message_with_progress(full_text, full_thinking)
        };
        let _ = db.save_message_with_thinking(
            "assistant",
            &saved_text,
            if full_thinking.trim().is_empty() {
                None
            } else {
                Some(full_thinking)
            },
        );
    }

    let _ = app_handle.emit(
        "ai-error",
        json!({
            "message": "已中止",
            "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
            "aborted": true
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_attachment(path: String, extension: &str, is_image: bool) -> UserAttachment {
        UserAttachment {
            path,
            name: format!("sample.{extension}"),
            extension: extension.to_string(),
            is_image,
        }
    }

    fn temp_image_path(extension: &str) -> String {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("ai-desktop-pet-vision-{nonce}.{extension}"))
            .to_string_lossy()
            .into_owned()
    }

    #[tokio::test]
    async fn keeps_plain_text_without_image_attachments() {
        let content = build_user_content("hello", &[]).await;
        assert_eq!(content, Value::String("hello".to_string()));
    }

    #[tokio::test]
    async fn builds_multimodal_content_for_supported_image_attachment() {
        let path = temp_image_path("png");
        tokio::fs::write(&path, b"fake-png-bytes").await.unwrap();

        let content = build_user_content(
            "describe this",
            &[test_attachment(path.clone(), "png", true)],
        )
        .await;
        let parts = content.as_array().expect("multimodal content");

        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "describe this");
        assert_eq!(parts[1]["type"], "image_url");
        assert!(parts[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn warns_without_multimodal_content_for_unsupported_image_format() {
        let path = temp_image_path("svg");
        tokio::fs::write(&path, b"<svg />").await.unwrap();

        let content = build_user_content(
            "describe this",
            &[test_attachment(path.clone(), "svg", true)],
        )
        .await;
        let text = content.as_str().expect("plain text fallback");

        assert!(text.contains("describe this"));
        assert!(text.contains("当前仅支持 PNG、JPEG、WEBP、GIF 图片"));

        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn treats_chunked_unexpected_eof_as_recoverable() {
        assert!(is_recoverable_stream_error(
            "request or response body error: error reading a body from connection: unexpected EOF during chunk size line"
        ));
    }

    #[test]
    fn connection_reset_is_recoverable() {
        assert!(is_recoverable_stream_error("connection reset by peer"));
    }

    #[test]
    fn connection_closed_before_message_completed_is_recoverable() {
        assert!(is_recoverable_stream_error(
            "error sending request for url (https://example.com/v1/chat/completions): connection closed before message completed"
        ));
    }

    #[test]
    fn peer_closed_is_recoverable() {
        assert!(is_recoverable_stream_error(
            "peer closed connection without sending TLS close_notify"
        ));
    }

    #[test]
    fn decode_response_body_error_is_recoverable() {
        assert!(is_recoverable_stream_error(
            "error decoding response body: invalid UTF-8"
        ));
    }

    #[test]
    fn request_canceled_is_recoverable() {
        assert!(is_recoverable_stream_error("request was canceled"));
    }

    #[test]
    fn unrelated_error_is_not_recoverable() {
        assert!(!is_recoverable_stream_error("invalid API key"));
        assert!(!is_recoverable_stream_error("rate limit exceeded"));
    }

    #[test]
    fn unreviewed_mode_does_not_force_computer_use_confirmation() {
        let mut config = DirectApiConfig::default();
        config.execution_mode = "unreviewed".to_string();

        assert!(!mode_requires_confirmation(
            &config,
            "computer_keyboard",
            false
        ));
        assert!(!mode_requires_confirmation(
            &config,
            "computer_mouse",
            false
        ));
    }

    #[test]
    fn approved_computer_use_tool_is_not_reconfirmed() {
        let mut config = DirectApiConfig::default();
        config.execution_mode = "normal".to_string();

        assert!(mode_requires_confirmation(
            &config,
            "computer_keyboard",
            false
        ));
        assert!(!mode_requires_confirmation(
            &config,
            "computer_keyboard",
            true
        ));
    }

    #[test]
    fn weather_tool_does_not_require_confirmation() {
        let mut config = DirectApiConfig::default();
        config.execution_mode = "normal".to_string();

        assert!(!mode_requires_confirmation(&config, "get_weather", false));
    }

    #[test]
    fn history_context_includes_compact_tool_progress() {
        let thinking = r#"
private reasoning that should not be replayed
[调用工具] 打开应用 notepad
{
  "app": "notepad"
}
[执行结果]
{
  "ok": true,
  "summary": "已启动: notepad (notepad.exe)"
}
"#;

        let msg = crate::storage::ChatMessage {
            role: "assistant".to_string(),
            content: "[执行中断] 测试".to_string(),
            thinking: Some(thinking.to_string()),
            created_at: 0,
            quoted_role: None,
            quoted_content: None,
            agent_id: None,
            agent_name: None,
            agent_avatar: None,
        };
        let content = assistant_history_content_for_context(&msg, None);

        assert!(content.contains("[上次工具进度摘要]"));
        assert!(content.contains("[调用工具] 打开应用 notepad"));
        assert!(content.contains("已启动: notepad"));
        assert!(!content.contains("private reasoning"));
    }

    #[test]
    fn turn_limit_message_keeps_progress_for_continue() {
        let thinking =
            "[调用工具] 查看桌面截图\n{}\n[执行结果]\n{\"summary\":\"Screenshot captured\"}";
        let message = turn_limit_message(160, thinking);

        assert!(message.contains("已达到最大迭代限制 160 轮"));
        assert!(message.contains("继续"));
        assert!(message.contains("[当前工具进度摘要]"));
        assert!(message.contains("Screenshot captured"));
    }

    #[test]
    fn is_high_risk_tool_classifies_correctly() {
        // 高风险：每次都要弹窗
        assert!(is_high_risk_tool("run_command"));
        assert!(is_high_risk_tool("write_file"));
        assert!(is_high_risk_tool("open_app"));

        // 低风险：一次批准后可缓存
        assert!(!is_high_risk_tool("computer_mouse"));
        assert!(!is_high_risk_tool("read_file"));
        assert!(!is_high_risk_tool("read_webpage"));
        assert!(!is_high_risk_tool("search_web"));
    }
}
