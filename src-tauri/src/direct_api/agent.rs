use crate::direct_api::api_client::{
    api_error_message_from_sse_text, api_error_message_from_value,
    call_chat_completions_non_stream, call_chat_completions_stream, ChatCompletionChunk,
    DirectApiConfig, SseParser, UserAttachment, Utf8ChunkDecoder,
};
use crate::direct_api::tools::{execute_tool, tool_definitions};
use crate::storage::ChatMessage;
use crate::AppState;
use base64::Engine;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use crate::ai_events::AiEmit;
use tauri::Manager;

const MAX_AGENT_TURNS: u32 = 160;
const MAX_VISION_IMAGES: usize = 5;
const MAX_VISION_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

/// 工具授权弹窗的载荷。桌宠的直连 API 与本地 CLI（dsh）后端共用同一份结构与
/// 同一套事件（ai-tool-confirm / ai-tool-confirm-resolved），所以字段是公开的。
#[derive(Clone, Serialize)]
pub struct ToolConfirmPayload {
    pub id: String,
    pub tool_name: String,
    pub arguments: String,
    pub summary: String,
    pub command: Option<String>,
    pub path: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub session_id: String,
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) session_id: String,
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
}

#[derive(Default)]
struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Serialize)]
struct AnswerDeltaPayload {
    text: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn run_direct_api_agent(
    app_handle: tauri::AppHandle,
    message: String,
    config: DirectApiConfig,
    system_prompt: String,
    chat_history: Vec<ChatMessage>,
    attachments: Vec<UserAttachment>,
    quote: Option<(String, String)>,
    agent: Option<crate::AgentContext>,
    turn_tool_grants: HashSet<String>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let state = app_handle.state::<AppState>();
    let single_agent = agent.as_ref().and_then(|ctx| ctx.single()).cloned();
    let agent_meta = single_agent
        .as_ref()
        .map(|a| (a.id.as_str(), a.name.as_str(), a.avatar.as_str()));
    // 取消不再由外层 select 抢先返回：那样内层累计的正文、思考和工具进度会随 future 一起
    // 被丢弃，"已中止"时什么都存不下，"继续"时模型也不知道刚才做过什么。
    // 现在由回合内部每个等待点响应取消，并把已有的部分结果带回来。
    let result = execute_direct_api_turn(
        app_handle.clone(),
        message,
        config,
        system_prompt,
        chat_history,
        attachments,
        quote,
        agent.clone(),
        turn_tool_grants,
        cancel.clone(),
        true,
    )
    .await;
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            if error.aborted {
                send_aborted(&app_handle, &error.partial_text, &error.thinking, agent_meta).await;
            } else {
                send_error(&app_handle, error.message, &error.thinking, &error.partial_text, agent_meta).await;
            }
            return;
        }
    };
    let full_text = output.text;
    let full_thinking = output.thinking;
    crate::save_assistant_message(
        &app_handle,
        &full_text,
        if full_thinking.is_empty() {
            None
        } else {
            Some(full_thinking.as_str())
        },
        agent_meta,
    )
    .await;

    {
        let mut behavior = state.behavior.lock().await;
        behavior.set_state(crate::behavior::PetState::Speaking);
        behavior.mood.update(Some(0.05), None);
    }

    crate::clear_active_chat(&state);

    let _ = app_handle.emit(
        "ai-finished",
        crate::AiFinishedPayload {
            text: full_text,
            thinking: if full_thinking.is_empty() {
                None
            } else {
                Some(full_thinking)
            },
            agent_id: single_agent.as_ref().map(|a| a.id.clone()),
            agent_name: single_agent.as_ref().map(|a| a.name.clone()),
            agent_avatar: single_agent.as_ref().map(|a| a.avatar.clone()),
        },
    );
}

/// 首包超时：推理模型（o 系列、部分中转的 thinking 模型）可能先静默思考很久才吐第一个字节。
const STREAM_FIRST_CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
/// 包间隔超时：已经开始输出后，超过这个时间没有新数据视为断流。
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// 流式请求最多尝试次数（含第一次）：用户的中转服务经常中途断流。
const MAX_STREAM_ATTEMPTS: u32 = 5;
/// 一次尝试里新收到的内容少于这个字节数就断流时，回滚后整轮重试比保留半截更有价值。
const RETRY_CONTENT_THRESHOLD: usize = 100;
/// 送进上下文的历史消息总字符上限（约 6 万 token）：超出时丢弃最早的消息，
/// 否则长会话一旦超过模型上下文，之后每一轮都会失败，只能新建对话。
const HISTORY_CHAR_BUDGET: usize = 120_000;

fn aborted_error(full_text: &str, full_thinking: &str) -> crate::agent_turn::TurnError {
    crate::agent_turn::TurnError::aborted(full_text, full_thinking)
}

/// 两次流式尝试之间的退避等待（2s、4s、8s、16s），期间可以被取消。
async fn wait_before_retry(
    app_handle: &tauri::AppHandle,
    cancel: &tokio_util::sync::CancellationToken,
    attempt: u32,
    reason: &str,
    full_text: &str,
    full_thinking: &str,
) -> Result<(), crate::agent_turn::TurnError> {
    let delay = Duration::from_secs(2u64.pow(attempt.min(4)));
    let notice = format!(
        "\n[连接中断：{}。{} 秒后重试（{}/{}）]\n",
        reason,
        delay.as_secs(),
        attempt,
        MAX_STREAM_ATTEMPTS - 1
    );
    let _ = app_handle.emit("ai-thinking", &notice);
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(aborted_error(full_text, full_thinking)),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

/// 按字符预算保留最近的历史消息（至少保留最后两条），返回 (保留的消息, 被省略的条数)。
pub fn trim_history_for_context(history: Vec<ChatMessage>, budget_chars: usize) -> (Vec<ChatMessage>, usize) {
    let mut used = 0usize;
    let mut keep = 0usize;
    for message in history.iter().rev() {
        let cost = message.content.chars().count() + 32;
        if keep >= 2 && used + cost > budget_chars {
            break;
        }
        used += cost;
        keep += 1;
    }
    let omitted = history.len() - keep;
    (history.into_iter().skip(omitted).collect(), omitted)
}

/// 流式 tool_calls 的槽位：带 index 的照用；不带 index 的服务商，出现新的 id 表示新的一次调用，
/// 否则续写上一次调用。
fn resolve_tool_call_index(
    calls: &HashMap<usize, AccumulatedToolCall>,
    delta: &crate::direct_api::api_client::ToolCallDelta,
    last_index: &mut Option<usize>,
) -> usize {
    let index = match delta.index {
        Some(index) => index,
        None => {
            let incoming_id = delta.id.as_deref().filter(|id| !id.is_empty());
            match (*last_index, incoming_id) {
                (Some(last), Some(id))
                    if calls
                        .get(&last)
                        .is_some_and(|call| !call.id.is_empty() && call.id != id) =>
                {
                    calls.keys().max().map_or(0, |max| max + 1)
                }
                (Some(last), _) => last,
                (None, _) => 0,
            }
        }
    };
    *last_index = Some(index);
    index
}

/// 解析工具参数：空串 / null 视为无参数，其它必须是 JSON 对象。
fn parse_tool_arguments(raw: &str) -> Result<serde_json::Value, String> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Null) => Ok(json!({})),
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err("参数必须是 JSON 对象".to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// 把一个流式 tool_call 增量合并进累积结果。
/// 有的中转每块都重复下发完整的 id / name：直接拼接会得到 `call_xcall_x`、`read_fileread_file`。
fn merge_tool_call_delta(entry: &mut AccumulatedToolCall, delta: &crate::direct_api::api_client::ToolCallDelta) {
    if let Some(id) = delta.id.as_deref().filter(|id| !id.is_empty()) {
        if entry.id.is_empty() {
            entry.id = id.to_string();
        }
    }
    if let Some(function) = &delta.function {
        if let Some(name) = function.name.as_deref().filter(|name| !name.is_empty()) {
            if entry.name != name {
                entry.name.push_str(name);
            }
        }
        if let Some(arguments) = &function.arguments {
            entry.arguments.push_str(arguments);
        }
    }
}

pub async fn execute_direct_api_turn(
    app_handle: tauri::AppHandle,
    message: String,
    config: DirectApiConfig,
    system_prompt: String,
    chat_history: Vec<ChatMessage>,
    attachments: Vec<UserAttachment>,
    quote: Option<(String, String)>,
    agent: Option<crate::AgentContext>,
    turn_tool_grants: HashSet<String>,
    cancel: tokio_util::sync::CancellationToken,
    follow_global_config: bool,
) -> Result<crate::agent_turn::TurnOutput, crate::agent_turn::TurnError> {
    let state = app_handle.state::<AppState>();

    let (chat_history, omitted_history) = trim_history_for_context(chat_history, HISTORY_CHAR_BUDGET);
    let system_prompt = if omitted_history > 0 {
        format!(
            "{system_prompt}\n\n（为控制上下文长度，本会话更早的 {omitted_history} 条消息没有附上；如果需要更早的信息，请让用户补充。）"
        )
    } else {
        system_prompt
    };

    let mut api_messages = Vec::new();
    api_messages.push(json!({
        "role": "system",
        "content": system_prompt
    }));

    for msg in chat_history {
        let (role, content) = match msg.role.as_str() {
            "assistant" => ("assistant", assistant_history_content_for_context(&msg, agent.as_ref())),
            // 定时提醒等系统通知以 system 角色落库；对话中途的 system 消息不少服务商不接受，
            // 转成带标记的用户消息。
            "system" => ("user", format!("[系统通知] {}", msg.content)),
            _ => ("user", msg.content),
        };
        api_messages.push(json!({
            "role": role,
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
    // 整个回合里是否解析到过模型事件：用来区分"接口不是 SSE"与"模型没给正文"。
    let mut saw_model_event = false;

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
            return Err(aborted_error(&full_text, &full_thinking));
        }

        let tools = if config.execution_mode == "plan" {
            None
        } else {
            Some(tool_definitions())
        };

        let mut accumulated_tool_calls: HashMap<usize, AccumulatedToolCall> = HashMap::new();
        let mut turn_text = String::new();
        // 模型给出的结束原因："length" 表示达到输出长度上限，回复被截断。
        let mut finish_reason: Option<String> = None;

        // 根据stream_mode选择不同的调用方式
        match config.stream_mode.as_str() {
            "non_stream" => {
                let result = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(aborted_error(&full_text, &full_thinking)),
                    result = call_chat_completions_non_stream(&state.http_client, &config, &api_messages, tools) => result,
                };
                match result {
                    Ok(result) => {
                        saw_model_event = true;
                        finish_reason = result.finish_reason.clone();
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

                        // 工具调用填进 accumulated_tool_calls，交给下面的统一执行循环
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
                        return Err(crate::agent_turn::TurnError::failed(
                            format!("API 请求失败: {e}"),
                            &full_thinking,
                        )
                        .with_partial_text(&full_text));
                    }
                }
            }
            "auto" | _ => {
                let mut raw_buffer = String::new();
                let mut attempt: u32 = 0;

                loop {
                    attempt += 1;
                    // 回滚点：重试时撤回本次尝试写入的半截正文 / 思考 / 工具调用，
                    // 否则新响应会接在旧内容后面（"好的，好的，我来…"、"read_fileread_file"）。
                    let text_mark = full_text.len();
                    let thinking_mark = full_thinking.len();
                    turn_text.clear();
                    finish_reason = None;
                    accumulated_tool_calls.clear();
                    raw_buffer.clear();

                    let request = tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return Err(aborted_error(&full_text, &full_thinking)),
                        request = call_chat_completions_stream(&state.http_client, &config, &api_messages, tools.clone()) => request,
                    };
                    let res = match request {
                        Ok(res) => res,
                        Err(error) => {
                            // 401 / 400 / 413 这类配置错误重试多少次都一样，直接报错，不再白等 30 秒。
                            if error.retryable && attempt < MAX_STREAM_ATTEMPTS {
                                wait_before_retry(&app_handle, &cancel, attempt, &error.message, &full_text, &full_thinking).await?;
                                continue;
                            }
                            return Err(crate::agent_turn::TurnError::failed(
                                format!("API 请求失败: {}", error.message),
                                &full_thinking,
                            )
                            .with_partial_text(&full_text));
                        }
                    };

                    let mut stream = res.bytes_stream();
                    let mut sse = SseParser::new();
                    // 跨 chunk 增量解码：被分块边界切开的汉字不再变成 "���"。
                    let mut utf8 = Utf8ChunkDecoder::default();
                    let mut done = false;
                    let mut stream_finished = false;
                    let mut received_any = false;
                    let mut interruption: Option<String> = None;
                    let mut last_tool_index: Option<usize> = None;

                    loop {
                        let idle_limit = if received_any {
                            STREAM_IDLE_TIMEOUT
                        } else {
                            STREAM_FIRST_CHUNK_TIMEOUT
                        };
                        let next = tokio::select! {
                            biased;
                            _ = cancel.cancelled() => return Err(aborted_error(&full_text, &full_thinking)),
                            next = tokio::time::timeout(idle_limit, stream.next()) => next,
                        };
                        let events = match next {
                            Ok(Some(Ok(bytes))) => {
                                received_any = true;
                                let text = utf8.push(&bytes);
                                raw_buffer.push_str(&text);
                                sse.push(&text)
                            }
                            Ok(None) => {
                                // 正常结束：冲出残余字节，并用换行收尾最后一行没有换行符的数据。
                                stream_finished = true;
                                let mut text = utf8.finish();
                                raw_buffer.push_str(&text);
                                text.push('\n');
                                sse.push(&text)
                            }
                            Ok(Some(Err(error))) => {
                                let message = error.to_string();
                                eprintln!(
                                    "[stream-error] attempt={} recoverable={} err={}",
                                    attempt,
                                    is_recoverable_stream_error(&message),
                                    message
                                );
                                interruption = Some(format!("流式读取失败: {message}"));
                                break;
                            }
                            Err(_) => {
                                interruption = Some(format!(
                                    "超过 {} 秒没有收到新数据",
                                    idle_limit.as_secs()
                                ));
                                break;
                            }
                        };

                        for (event_type, data) in events {
                            if data == "[DONE]" {
                                done = true;
                                break;
                            }

                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
                                if let Some(message) = api_error_message_from_value(&value) {
                                    return Err(crate::agent_turn::TurnError::failed(
                                        message,
                                        &full_thinking,
                                    )
                                    .with_partial_text(&full_text));
                                }
                            }

                            if config.api_mode == "responses" {
                                // ===== Responses API 流式解析 =====
                                let data_value: serde_json::Value =
                                    match serde_json::from_str(&data) {
                                        Ok(v) => v,
                                        Err(_) => continue,
                                    };
                                saw_model_event = true;
                                match event_type.as_str() {
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
                                            accumulated_tool_calls
                                                .entry(idx)
                                                .or_default()
                                                .arguments
                                                .push_str(delta);
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
                                            // done 事件包含完整参数，覆盖累积值
                                            accumulated_tool_calls.entry(idx).or_default().arguments =
                                                args.to_string();
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
                                                let entry =
                                                    accumulated_tool_calls.entry(idx).or_default();
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
                                    "response.completed" | "response.done" => {
                                        done = true;
                                    }
                                    "response.failed" | "response.incomplete" => {
                                        // 失败 / 被截断的响应不能当成功收尾：带上原因报错。
                                        let reason = data_value
                                            .pointer("/response/error/message")
                                            .or_else(|| data_value.pointer("/response/incomplete_details/reason"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("上游返回 response.failed")
                                            .to_string();
                                        return Err(crate::agent_turn::TurnError::failed(
                                            format!("上游 API 错误：{reason}"),
                                            &full_thinking,
                                        )
                                        .with_partial_text(&full_text));
                                    }
                                    _ => {}
                                }
                            } else {
                                // ===== Chat Completions 流式解析 =====
                                if let Ok(parsed) =
                                    serde_json::from_str::<ChatCompletionChunk>(&data)
                                {
                                    saw_model_event = true;
                                    if let Some(choice) = parsed.choices.first() {
                                        if let Some(reason) = choice.finish_reason.as_deref() {
                                            finish_reason = Some(reason.to_string());
                                        }
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
                                                let index = resolve_tool_call_index(
                                                    &accumulated_tool_calls,
                                                    tc,
                                                    &mut last_tool_index,
                                                );
                                                merge_tool_call_delta(
                                                    accumulated_tool_calls.entry(index).or_default(),
                                                    tc,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if done || stream_finished {
                            break;
                        }
                    }

                    if let Some(reason) = interruption {
                        let attempt_content =
                            (full_text.len() - text_mark) + (full_thinking.len() - thinking_mark);
                        let partial_tool_calls = !accumulated_tool_calls.is_empty();
                        // 内容太少，或者工具调用参数被截断：回滚本次尝试，整轮重来。
                        if (attempt_content < RETRY_CONTENT_THRESHOLD || partial_tool_calls)
                            && attempt < MAX_STREAM_ATTEMPTS
                        {
                            full_text.truncate(text_mark);
                            full_thinking.truncate(thinking_mark);
                            turn_text.clear();
                            accumulated_tool_calls.clear();
                            wait_before_retry(&app_handle, &cancel, attempt, &reason, &full_text, &full_thinking).await?;
                            continue;
                        }
                        if partial_tool_calls {
                            return Err(crate::agent_turn::TurnError::failed(
                                format!("{reason}：工具调用参数不完整，重试 {attempt} 次后仍失败，已停止"),
                                &full_thinking,
                            )
                            .with_partial_text(&full_text));
                        }
                        if turn_text.trim().is_empty() {
                            return Err(crate::agent_turn::TurnError::failed(
                                format!("{reason}，重试 {attempt} 次后仍没有收到回复"),
                                &full_thinking,
                            )
                            .with_partial_text(&full_text));
                        }
                        // 已经收到较多正文：保留下来，并明确标注可能不完整（以前是悄悄当成完整回复）。
                        let note = format!("\n\n[⚠️ {reason}，以上回复可能不完整，可以发送「继续」]");
                        turn_text.push_str(&note);
                        full_text.push_str(&note);
                        let _ = app_handle.emit("ai-answer-delta", AnswerDeltaPayload { text: note });
                    }
                    break;
                }

                // 兜底：SSE 解析没有拿到正文和工具调用时，尝试把原始响应当作其他格式解析
                if turn_text.is_empty()
                    && accumulated_tool_calls.is_empty()
                    && !raw_buffer.is_empty()
                {
                    if let Some(message) = api_error_message_from_sse_text(&raw_buffer) {
                        return Err(crate::agent_turn::TurnError::failed(
                            message,
                            &full_thinking,
                        )
                        .with_partial_text(&full_text));
                    }
                    // HTML 响应检测
                    let buf_trimmed = raw_buffer.trim();
                    if buf_trimmed.starts_with("<!DOCTYPE")
                        || buf_trimmed.starts_with("<!doctype")
                        || buf_trimmed.starts_with("<html")
                        || buf_trimmed.starts_with("<HTML")
                    {
                        return Err(crate::agent_turn::TurnError::failed("API 返回了 HTML 页面而非 JSON 响应。请检查 Base URL 是否指向了正确的 API 端点（而非网关首页），以及 API Key 是否有效。".to_string(), &full_thinking).with_partial_text(&full_text));
                    }

                    // 已经按 SSE 解析到过模型事件（只是这一轮没有正文，比如只有思考）时，
                    // 不能再走下面的兜底：以前会把整段 "data: {...}" 原始协议文本当成回复显示并落库。
                    if !saw_model_event {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw_buffer) {
                            use crate::direct_api::api_client::ContentExtractor;

                            if let Some(content) = ContentExtractor::extract_content(&value) {
                                turn_text.push_str(&content);
                                full_text.push_str(&content);
                                let _ = app_handle
                                    .emit("ai-answer-delta", AnswerDeltaPayload { text: content });
                            }

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

                            if turn_text.is_empty() && accumulated_tool_calls.is_empty() {
                                if let Some(reasoning) = ContentExtractor::extract_reasoning(&value) {
                                    full_thinking.push_str(&reasoning);
                                }
                            }
                        }

                        // 纯文本兜底：只接受明显不是 SSE / JSON / HTML 的响应体。
                        let looks_like_protocol = buf_trimmed.starts_with("data:")
                            || buf_trimmed.starts_with("event:")
                            || buf_trimmed.starts_with(':')
                            || buf_trimmed.starts_with('<')
                            || buf_trimmed.starts_with('{')
                            || buf_trimmed.starts_with('[');
                        if turn_text.is_empty()
                            && accumulated_tool_calls.is_empty()
                            && !buf_trimmed.is_empty()
                            && !looks_like_protocol
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

        // 只有正文、但结束原因是 length：回复被输出长度上限截断了，明确标注而不是当成完整回答。
        if finish_reason.as_deref() == Some("length")
            && accumulated_tool_calls.is_empty()
            && !turn_text.trim().is_empty()
        {
            let note = "\n\n[⚠️ 回复达到模型的输出长度上限被截断，可以发送「继续」]";
            turn_text.push_str(note);
            full_text.push_str(note);
            let _ = app_handle.emit("ai-answer-delta", AnswerDeltaPayload { text: note.to_string() });
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
        // 缺 id 的调用先补上：assistant 消息里的 tool_calls[].id 必须和后面 tool 消息的
        // tool_call_id 一致。以前只在 tool 消息里补，两边对不上，下一轮请求会被接口拒绝。
        let generated_at = crate::unix_now();
        for (index, tc) in ordered_tool_calls.iter_mut() {
            if tc.id.is_empty() {
                tc.id = format!("tool_{}_{}_{}", tc.name, generated_at, index);
            }
        }

        let mut assistant_tool_calls_json = Vec::new();
        for (_, tc) in &ordered_tool_calls {
            // 参数不是合法 JSON（多半被输出长度上限截断）时，历史里记成 "{}"：
            // 原样回传非法 JSON，部分接口会直接以 400 拒绝下一轮请求。
            let arguments = if parse_tool_arguments(&tc.arguments).is_ok() {
                tc.arguments.clone()
            } else {
                "{}".to_string()
            };
            assistant_tool_calls_json.push(json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": arguments
                }
            }));
        }

        api_messages.push(json!({
            "role": "assistant",
            "content": if turn_text.is_empty() { None } else { Some(turn_text.as_str()) },
            "tool_calls": assistant_tool_calls_json
        }));

        // 截图类工具附带的图片消息要等本轮所有 tool 结果都推完再追加：
        // assistant(tool_calls) 之后必须紧跟各个 tool 消息，中间插入 user 消息会被
        // OpenAI / DeepSeek 等接口以 400 拒绝（并行调用"截图 + 列窗口"时必现）。
        let mut visual_messages: Vec<serde_json::Value> = Vec::new();

        for (_, tc) in ordered_tool_calls {
            let tc_id = tc.id;
            let tc_name = tc.name;
            let tc_args_str = tc.arguments;
            let tc_args = match parse_tool_arguments(&tc_args_str) {
                Ok(args) => args,
                Err(error) => {
                    // 以前解析失败就按 {} 执行：截断的参数跑出来只会是错误或半截文件。
                    // 现在不执行，把原因作为工具结果告诉模型，让它补全后重新调用。
                    let tool_output = format!(
                        "工具参数不是完整的 JSON（{error}），没有执行。常见原因是参数太长、被模型的输出长度上限截断：请重新调用并给出完整参数；内容很长时分多次写入，或改用 edit_file 做局部修改。"
                    );
                    emit_tool_event(
                        &app_handle,
                        &tc_id,
                        &tc_name,
                        "skipped",
                        &json!({}),
                        Some(&tool_output),
                        Some(false),
                    );
                    let display = format!("\n[调用工具] {tc_name}（参数不完整，未执行）\n");
                    full_thinking.push_str(&display);
                    append_active_thinking(&state, &display);
                    let _ = app_handle.emit("ai-thinking", &display);
                    push_tool_message(&mut api_messages, &tc_id, &tc_name, &tool_output);
                    continue;
                }
            };
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

            // 授权缓存按会话隔离：只看本会话批准过的工具。
            let already_approved =
                crate::is_tool_approved(&state, &crate::current_turn_session(&state), &tc_name).await;
            let latest_config = if follow_global_config {
                state.direct_api_config.lock().await.clone()
            } else {
                None
            };
            let permission = tool_permission(
                &config,
                latest_config.as_ref(),
                &turn_tool_grants,
                &tc_name,
                already_approved,
            );
            if permission == ToolPermission::Skip {
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

            // ask_user 特殊分支: 不走确认/执行流程, 直接弹内嵌询问弹窗等用户作答。
            // 用户的回答会作为工具结果回传给模型, 让它据此决定后续方向。
            if tc_name == "ask_user" {
                let tool_output =
                    request_user_answer(&state, &app_handle, &mut full_thinking, &tc_id, &tc_args, &cancel)
                        .await;
                if cancel.is_cancelled() {
                    return Err(aborted_error(&full_text, &full_thinking));
                }

                let output_for_log = tool_output_for_log(&tool_output);
                let output_display = format!("[执行结果]\n{}\n", output_for_log);
                full_thinking.push_str(&output_display);
                append_active_thinking(&state, &output_display);
                let _ = app_handle.emit("ai-thinking", &output_display);
                push_tool_message(&mut api_messages, &tc_id, &tc_name, &tool_output);
                continue;
            }

            let requires_confirm = permission == ToolPermission::Confirm;
            let focus_snapshot = if requires_confirm
                && crate::computer_use::should_restore_focus_after_confirmation(&tc_name)
            {
                crate::computer_use::capture_foreground_window().await
            } else {
                None
            };

            let approved = if requires_confirm {
                request_tool_confirmation(
                    &state,
                    &app_handle,
                    &mut full_thinking,
                    &tc_id,
                    &tc_name,
                    &tc_args,
                    &pretty_args,
                    &cancel,
                )
                .await
            } else {
                true
            };

            if cancel.is_cancelled() {
                return Err(aborted_error(&full_text, &full_thinking));
            }
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
                // 工具执行期间也要响应停止：被丢弃的 run_command future 会通过进程树守卫回收子进程。
                let output = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        emit_tool_event(&app_handle, &tc_id, &tc_name, "interrupted", &tc_args, Some("操作已被用户中止。"), Some(false));
                        return Err(aborted_error(&full_text, &full_thinking));
                    }
                    output = execute_tool(&tc_name, &tc_args, &config.search_provider) => output,
                };
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

                // 智能体工坊工具改动了 agents 表：广播后各窗口会重载 @ 提及列表和工坊。
                if tc_name == "create_agent" || tc_name == "update_agent" {
                    let _ = app_handle.emit("agents-changed", json!({}));
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
            append_visual_tool_message(&mut visual_messages, &tc_name, &tool_output);
        }

        api_messages.extend(visual_messages);
    }

    // 保存前再次检查是否已取消
    if cancel.is_cancelled() {
        return Err(aborted_error(&full_text, &full_thinking));
    }

    if full_text.trim().is_empty() {
        // 根据配置和响应情况提供更具体的错误信息
        let error_msg = if config.stream_mode == "non_stream" {
            "API 请求结束了，但没有返回可显示的回复内容。请检查模型配置和接口返回。".to_string()
        } else if saw_model_event && !full_thinking.trim().is_empty() {
            "模型只输出了思考过程，没有给出正文回复（可能是输出长度上限截断了思考）。可以直接发送「继续」。".to_string()
        } else if saw_model_event {
            "模型这一轮没有返回任何正文，可以重试或换个说法。".to_string()
        } else {
            "接口返回了HTTP 200，但响应不是OpenAI SSE格式。如果使用的是Auto Code等非标准接口，请在设置中将流式模式改为non_stream。".to_string()
        };

        return Err(crate::agent_turn::TurnError::failed(
            error_msg,
            &full_thinking,
        ));
    }

    Ok(crate::agent_turn::TurnOutput {
        text: full_text,
        thinking: full_thinking,
    })
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

fn append_active_thinking(state: &tauri::State<'_, AppState>, text: &str) {
    crate::append_session_thinking(state, text);
}

async fn request_tool_confirmation(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    full_thinking: &mut String,
    event_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    pretty_args: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> bool {
    let waiting_msg = format!("[等待授权] {}\n", tool_summary(tool_name, args));
    full_thinking.push_str(&waiting_msg);
    append_active_thinking(state, &waiting_msg);
    let _ = app_handle.emit("ai-thinking", &waiting_msg);
    emit_tool_event(app_handle, event_id, tool_name, "waiting", args, None, None);

    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    let confirm_id = format!("confirm_{}", uuid::Uuid::new_v4());
    let payload = ToolConfirmPayload {
        id: confirm_id.clone(), tool_name: tool_name.to_string(), arguments: pretty_args.to_string(),
        summary: tool_summary(tool_name, args), command: tool_command(tool_name, args), path: tool_path(tool_name, args),
        session_id: crate::current_turn_session(state),
    };
    {
        let mut confirms = state.pending_confirms.lock().await;
        confirms.insert(confirm_id.clone(), crate::interactions::PendingRequest { sender: tx, payload: payload.clone() });
    }
    let mut guard = crate::interactions::RequestGuard::new(app_handle, &confirm_id, crate::interactions::RequestKind::Tool);

    let _ = app_handle.emit(
        "ai-tool-confirm",
        payload,
    );

    let approved_result = crate::interactions::wait_for_response(rx, cancel, Duration::from_secs(120)).await;

    {
        let mut confirms = state.pending_confirms.lock().await;
        confirms.remove(&confirm_id);
    }
    guard.settle();

    let emit_resolved = |approved: bool| {
        let _ = app_handle.emit(
            "ai-tool-confirm-resolved",
            json!({ "id": confirm_id.clone(), "approved": approved }),
        );
    };

    match approved_result {
        crate::interactions::WaitResult::Answered(true) => {
            emit_resolved(true);
            let approved_msg = "[用户已授权，开始执行]\n".to_string();
            full_thinking.push_str(&approved_msg);
            append_active_thinking(state, &approved_msg);
            let _ = app_handle.emit("ai-thinking", &approved_msg);
            if !is_high_risk_tool(tool_name) {
                crate::remember_tool_approval(state, &crate::current_turn_session(state), tool_name).await;
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
        crate::interactions::WaitResult::Answered(false) => {
            emit_resolved(false);
            let denied_msg = "[用户拒绝执行此操作]\n".to_string();
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
        crate::interactions::WaitResult::Closed => {
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
        crate::interactions::WaitResult::Cancelled => {
            emit_resolved(false);
            emit_tool_event(app_handle, event_id, tool_name, "interrupted", args, Some("操作已被用户中止。"), Some(false));
            false
        }
        crate::interactions::WaitResult::TimedOut => {
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

fn push_tool_message(
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
async fn request_user_answer(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    full_thinking: &mut String,
    event_id: &str,
    args: &serde_json::Value,
    cancel: &tokio_util::sync::CancellationToken,
) -> String {
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
    emit_tool_event(
        app_handle, event_id, "ask_user", "waiting", args, None, None,
    );

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let question_id = format!("ask_{}", uuid::Uuid::new_v4());
    let payload = AskUserPayload { id: question_id.clone(), question: question.clone(), options: options.clone(), session_id: crate::current_turn_session(state) };
    {
        let mut questions = state.pending_questions.lock().await;
        questions.insert(question_id.clone(), crate::interactions::PendingRequest { sender: tx, payload: payload.clone() });
    }
    let mut guard = crate::interactions::RequestGuard::new(app_handle, &question_id, crate::interactions::RequestKind::Question);

    let _ = app_handle.emit(
        "ai-ask-user",
        payload,
    );

    let answer_result = crate::interactions::wait_for_response(rx, cancel, Duration::from_secs(300)).await;

    {
        let mut questions = state.pending_questions.lock().await;
        questions.remove(&question_id);
    }
    guard.settle();

    match answer_result {
        crate::interactions::WaitResult::Answered(answer) => {
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
        crate::interactions::WaitResult::Closed => {
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
        crate::interactions::WaitResult::Cancelled => {
            let _ = app_handle.emit("ai-ask-user-resolved", json!({ "id": question_id, "answered": false }));
            emit_tool_event(app_handle, event_id, "ask_user", "interrupted", args, Some("操作已被用户中止。"), Some(false));
            "操作已被用户中止。".to_string()
        }
        crate::interactions::WaitResult::TimedOut => {
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

fn tool_output_for_log(output: &str) -> String {
    redact_visual_payload(output).unwrap_or_else(|| output.to_string())
}

fn tool_output_for_message(output: &str) -> String {
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

fn append_visual_tool_message(
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

#[derive(Debug, PartialEq, Eq)]
enum ToolPermission {
    Skip,
    Confirm,
    Allow,
}

fn tool_permission(
    initial: &DirectApiConfig,
    latest: Option<&DirectApiConfig>,
    turn_grants: &HashSet<String>,
    tool_name: &str,
    already_approved: bool,
) -> ToolPermission {
    let config = latest.unwrap_or(initial);
    // Plan mode must suppress execution even if a role or earlier turn granted it.
    if config.execution_mode == "plan" {
        return ToolPermission::Skip;
    }
    if mode_requires_confirmation(
        config,
        tool_name,
        already_approved || turn_grants.contains(tool_name),
    ) {
        ToolPermission::Confirm
    } else {
        ToolPermission::Allow
    }
}

fn mode_requires_confirmation(
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
    // 本地内容读取默认需确认；凭据路径在执行器中还会被硬拦截。
    // 联网查询和普通记忆写入保留原有策略。显式 custom/unreviewed 授权仍有效。
    if tool_name == "web_search"
        || tool_name == "get_weather"
        || tool_name == "list_scheduled_tasks"
        || tool_name == "search_memory"
        || tool_name == "save_memory"
        || tool_name == "ask_user"
    {
        return false;
    }
    match config.execution_mode.as_str() {
        "plan" | "unreviewed" => false,
        "custom" => !config
            .auto_approved_tools
            .iter()
            .any(|tool| tool == tool_name),
        _ => true,
    }
}

fn emit_tool_event(
    app_handle: &tauri::AppHandle,
    id: &str,
    tool_name: &str,
    status: &str,
    args: &serde_json::Value,
    output: Option<&str>,
    approved: Option<bool>,
) {
    let arguments = serde_json::to_string_pretty(args).unwrap_or_else(|_| "{}".to_string());
    // 工具执行期把桌宠切到 working，跑完切回 thinking（与 CLI 后端同一套映射）
    crate::apply_tool_status_to_pet(app_handle, status);
    let _ = app_handle.emit(
        "ai-tool-event",
        ToolEventPayload {
            id: id.to_string(),
            tool_name: tool_name.to_string(),
            status: status.to_string(),
            summary: tool_summary(tool_name, args),
            arguments,
            command: tool_command(tool_name, args),
            path: tool_path(tool_name, args),
            output: output.map(|value| compact_for_event(value, 3000)),
            approved,
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
fn is_high_risk_tool(tool_name: &str) -> bool {
    // create_agent / update_agent 会改写角色设定和授权名单：每次都要确认，不能批准一次就整个会话放行。
    matches!(
        tool_name,
        "run_command" | "write_file" | "open_app" | "create_agent" | "update_agent" | "read_file" | "code_search"
    )
}

fn tool_command(tool_name: &str, args: &serde_json::Value) -> Option<String> {
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

fn tool_path(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "read_file" | "write_file" | "edit_file" | "list_directory" => {
            args["path"].as_str().map(|value| value.to_string())
        }
        "file_search" => args["directory"].as_str().map(|value| value.to_string()),
        "browser_open" | "browser_navigate" => args["url"].as_str().map(|value| value.to_string()),
        _ => None,
    }
}

fn tool_summary(tool_name: &str, args: &serde_json::Value) -> String {
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
        "edit_file" => format!("修改文件 {}", args["path"].as_str().unwrap_or("未知路径")),
        "create_docx" => format!(
            "生成 Word 文档 {}",
            args["path"].as_str().unwrap_or("未指定路径")
        ),
        "ask_user" => format!(
            "向用户提问: {}",
            args["question"].as_str().unwrap_or("未指定问题")
        ),
        "list_agents" => "查看现有智能体".to_string(),
        "create_agent" => format!(
            "创建智能体 {}",
            args["name"].as_str().unwrap_or("未命名智能体")
        ),
        "update_agent" => format!(
            "更新智能体 {}",
            args["agent"].as_str().unwrap_or("未知智能体")
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

async fn send_error(
    app_handle: &tauri::AppHandle,
    err_msg: String,
    full_thinking: &str,
    partial_text: &str,
    agent: Option<(&str, &str, &str)>,
) {
    let state = app_handle.state::<AppState>();
    crate::clear_active_chat(&state);
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(crate::behavior::PetState::Confused);
    drop(behavior);

    {
        // 出错前已经输出的正文一起落库，切回会话时看得到；错误原因放在末尾。
        let base = if partial_text.trim().is_empty() {
            format!("[执行中断] {err_msg}")
        } else {
            format!("{}\n\n[执行中断] {err_msg}", partial_text.trim_end())
        };
        let saved_text = interrupted_message_with_progress(&base, full_thinking);
        crate::save_assistant_message(
            app_handle,
            &saved_text,
            if full_thinking.trim().is_empty() {
                None
            } else {
                Some(full_thinking)
            },
            agent,
        )
        .await;
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

async fn send_aborted(
    app_handle: &tauri::AppHandle,
    full_text: &str,
    full_thinking: &str,
    agent: Option<(&str, &str, &str)>,
) {
    let state = app_handle.state::<AppState>();
    crate::clear_active_chat(&state);
    if !crate::has_any_active_chat(&state) {
        let mut behavior = state.behavior.lock().await;
        behavior.set_state(crate::behavior::PetState::Idle);
    }
    // 被重置 / 切换后端 / 删除会话取消的回合：会话已被清空或删除，部分输出不落库，
    // 也不再发事件（否则会把"已中止"写进刚清空的界面）。
    if crate::turn_output_discarded(&state) {
        return;
    }

    // 有部分内容时保存到数据库，使"继续"时 AI 能接续
    if !full_text.trim().is_empty() || !full_thinking.trim().is_empty() {
        let saved_text = if full_text.trim().is_empty() {
            interrupted_message_with_progress("[思考中，已被中止]", full_thinking)
        } else {
            // 标注中止，免得重新打开会话时把半截回复当成完整回答。
            interrupted_message_with_progress(
                &format!("{}\n\n[已中止]", full_text.trim_end()),
                full_thinking,
            )
        };
        crate::save_assistant_message(
            app_handle,
            &saved_text,
            if full_thinking.trim().is_empty() {
                None
            } else {
                Some(full_thinking)
            },
            agent,
        )
        .await;
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
    fn local_content_reads_require_consent_unless_explicitly_preapproved() {
        let mut config = DirectApiConfig::default();
        for tool in ["read_file", "code_search"] {
            assert!(mode_requires_confirmation(&config, tool, false));
            assert!(is_high_risk_tool(tool), "approval must not silently authorize another path");
        }
        config.execution_mode = "custom".into();
        config.auto_approved_tools = vec!["read_file".into()];
        assert!(!mode_requires_confirmation(&config, "read_file", false));
        assert!(mode_requires_confirmation(&config, "code_search", false));
        config.execution_mode = "plan".into();
        assert_eq!(tool_permission(&config, None, &HashSet::new(), "read_file", true), ToolPermission::Skip);
    }

    #[test]
    fn role_grants_survive_live_config_refresh_in_normal_and_custom_modes() {
        let initial = DirectApiConfig::default();
        let grants = HashSet::from(["write_file".into(), "browser_click".into()]);
        for mode in ["normal", "custom"] {
            let mut latest = DirectApiConfig::default();
            latest.execution_mode = mode.into();
            for tool in ["write_file", "browser_click"] {
                assert_eq!(
                    tool_permission(&initial, Some(&latest), &grants, tool, false),
                    ToolPermission::Allow
                );
                assert_eq!(
                    tool_permission(&initial, Some(&latest), &HashSet::new(), tool, false),
                    ToolPermission::Confirm
                );
            }
            assert!(
                latest.auto_approved_tools.is_empty(),
                "turn grants must not mutate saved preferences"
            );
        }
    }

    #[test]
    fn live_plan_mode_blocks_even_scoped_or_previously_approved_tools() {
        let initial = DirectApiConfig::default();
        let mut latest = initial.clone();
        latest.execution_mode = "plan".into();
        let grants = HashSet::from(["write_file".into()]);
        assert_eq!(
            tool_permission(&initial, Some(&latest), &grants, "write_file", true),
            ToolPermission::Skip
        );
    }

    #[test]
    fn live_preferences_can_revoke_old_global_grants_without_affecting_scoped_grants() {
        let mut initial = DirectApiConfig::default();
        initial.execution_mode = "custom".into();
        initial.auto_approved_tools = vec!["write_file".into()];
        let latest = DirectApiConfig::default();
        assert_eq!(
            tool_permission(
                &initial,
                Some(&latest),
                &HashSet::new(),
                "write_file",
                false
            ),
            ToolPermission::Confirm
        );
        assert_eq!(
            tool_permission(&initial, None, &HashSet::new(), "write_file", false),
            ToolPermission::Allow
        );
        let mut unreviewed = latest.clone();
        unreviewed.execution_mode = "unreviewed".into();
        assert_eq!(
            tool_permission(
                &latest,
                Some(&unreviewed),
                &HashSet::new(),
                "write_file",
                false
            ),
            ToolPermission::Allow
        );
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
            id: 0,
            session_id: String::new(),
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

    fn tool_delta(
        index: Option<usize>,
        id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) -> crate::direct_api::api_client::ToolCallDelta {
        crate::direct_api::api_client::ToolCallDelta {
            index,
            id: id.map(str::to_string),
            r#type: None,
            function: Some(crate::direct_api::api_client::FunctionCallDelta {
                name: name.map(str::to_string),
                arguments: arguments.map(str::to_string),
            }),
        }
    }

    fn merge_all(deltas: &[crate::direct_api::api_client::ToolCallDelta]) -> Vec<(usize, String, String, String)> {
        let mut calls: HashMap<usize, AccumulatedToolCall> = HashMap::new();
        let mut last = None;
        for delta in deltas {
            let index = resolve_tool_call_index(&calls, delta, &mut last);
            merge_tool_call_delta(calls.entry(index).or_default(), delta);
        }
        let mut merged: Vec<_> = calls
            .into_iter()
            .map(|(index, call)| (index, call.id, call.name, call.arguments))
            .collect();
        merged.sort();
        merged
    }

    #[test]
    fn repeated_tool_call_ids_and_names_are_not_concatenated() {
        // 有的中转每块都重复下发完整 id / name：以前拼成 "call_1call_1"、"read_fileread_file"。
        let merged = merge_all(&[
            tool_delta(Some(0), Some("call_1"), Some("read_file"), Some("{\"pa")),
            tool_delta(Some(0), Some("call_1"), Some("read_file"), Some("th\":\"a.txt\"}")),
        ]);
        assert_eq!(
            merged,
            vec![(0, "call_1".into(), "read_file".into(), "{\"path\":\"a.txt\"}".into())]
        );
    }

    #[test]
    fn standard_split_names_still_concatenate() {
        let merged = merge_all(&[
            tool_delta(Some(0), Some("call_1"), Some("read_"), Some("")),
            tool_delta(Some(0), None, Some("file"), Some("{}")),
        ]);
        assert_eq!(merged[0].2, "read_file");
    }

    #[test]
    fn truncated_tool_arguments_are_rejected_instead_of_running_with_empty_args() {
        assert_eq!(parse_tool_arguments("").unwrap(), json!({}));
        assert_eq!(parse_tool_arguments("null").unwrap(), json!({}));
        assert_eq!(
            parse_tool_arguments("{\"path\":\"a.txt\"}").unwrap()["path"],
            "a.txt"
        );
        // 被输出长度上限截断的参数：以前按 {} 执行。
        assert!(parse_tool_arguments("{\"path\":\"a.txt\",\"content\":\"很长的内").is_err());
        assert!(parse_tool_arguments("[1,2]").is_err());
    }

    #[test]
    fn index_less_parallel_calls_get_separate_slots() {
        let merged = merge_all(&[
            tool_delta(None, Some("call_a"), Some("window_list"), Some("{")),
            tool_delta(None, None, None, Some("}")),
            tool_delta(None, Some("call_b"), Some("computer_screenshot"), Some("{}")),
        ]);
        assert_eq!(merged.len(), 2, "{merged:?}");
        assert_eq!(merged[0].1, "call_a");
        assert_eq!(merged[0].3, "{}");
        assert_eq!(merged[1].1, "call_b");
    }

    fn history_message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: 0,
            session_id: String::new(),
            role: role.to_string(),
            content: content.to_string(),
            thinking: None,
            created_at: 0,
            quoted_role: None,
            quoted_content: None,
            agent_id: None,
            agent_name: None,
            agent_avatar: None,
        }
    }

    #[test]
    fn trimming_history_drops_the_oldest_messages_first() {
        let history: Vec<ChatMessage> = (0..10)
            .map(|i| history_message(if i % 2 == 0 { "user" } else { "assistant" }, &format!("{i}{}", "长".repeat(1000))))
            .collect();
        let (kept, omitted) = trim_history_for_context(history, 3_500);
        assert_eq!(omitted, 7);
        assert_eq!(kept.len(), 3);
        assert!(kept[0].content.starts_with('7'));
        assert!(kept[2].content.starts_with('9'));
    }

    #[test]
    fn trimming_history_always_keeps_the_last_two_messages() {
        let history = vec![
            history_message("user", &"问".repeat(5_000)),
            history_message("assistant", &"答".repeat(5_000)),
        ];
        let (kept, omitted) = trim_history_for_context(history, 10);
        assert_eq!((kept.len(), omitted), (2, 0));
    }

    #[test]
    fn is_high_risk_tool_classifies_correctly() {
        // 高风险：每次都要弹窗
        assert!(is_high_risk_tool("run_command"));
        assert!(is_high_risk_tool("write_file"));
        assert!(is_high_risk_tool("open_app"));

        // 低风险：一次批准后可缓存
        assert!(!is_high_risk_tool("computer_mouse"));
        assert!(is_high_risk_tool("read_file"));
        assert!(!is_high_risk_tool("read_webpage"));
        assert!(!is_high_risk_tool("search_web"));
    }
}
