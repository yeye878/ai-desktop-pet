use crate::direct_api::api_client::{
    call_chat_completions_stream, ChatCompletionChunk, DirectApiConfig, SseParser,
};
use crate::direct_api::tools::{execute_tool, tool_definitions};
use crate::storage::ChatMessage;
use crate::AppState;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
struct ToolConfirmPayload {
    id: String,
    tool_name: String,
    arguments: String,
    summary: String,
    command: Option<String>,
    path: Option<String>,
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
) {
    let state = app_handle.state::<AppState>();

    let mut api_messages = Vec::new();
    api_messages.push(json!({
        "role": "system",
        "content": system_prompt
    }));

    for msg in chat_history {
        api_messages.push(json!({
            "role": msg.role,
            "content": msg.content
        }));
    }

    api_messages.push(json!({
        "role": "user",
        "content": message
    }));

    let mut current_turn = 0;
    let max_turns = 10;
    let mut full_text = String::new();
    let mut full_thinking = String::new();

    loop {
        current_turn += 1;
        if current_turn > max_turns {
            let limit_msg = "\n[已达到最大迭代限制 10 轮]";
            full_text.push_str(limit_msg);
            let _ = app_handle.emit("ai-thinking", limit_msg);
            break;
        }

        if is_cancelled(&state).await {
            send_aborted(&app_handle, &full_thinking).await;
            return;
        }

        let tools = if config.execution_mode == "plan" {
            None
        } else {
            Some(tool_definitions())
        };
        let res = match call_chat_completions_stream(&config, &api_messages, tools).await {
            Ok(res) => res,
            Err(e) => {
                send_error(&app_handle, format!("API 请求失败: {e}"), &full_thinking).await;
                return;
            }
        };

        let mut stream = res.bytes_stream();
        let mut sse = SseParser::new();
        let mut accumulated_tool_calls: HashMap<usize, AccumulatedToolCall> = HashMap::new();
        let mut turn_text = String::new();

        let mut done = false;
        loop {
            if is_cancelled(&state).await {
                send_aborted(&app_handle, &full_thinking).await;
                return;
            }

            let chunk_res = match tokio::time::timeout(Duration::from_secs(25), stream.next()).await {
                Ok(Some(chunk_res)) => chunk_res,
                Ok(None) => break,
                Err(_) => {
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
                    send_error(&app_handle, format!("流式读取失败: {e}"), &full_thinking).await;
                    return;
                }
            };

            let chunk_str = String::from_utf8_lossy(&chunk_bytes);
            let data_lines = sse.push(&chunk_str);

            for data in data_lines {
                if data == "[DONE]" {
                    done = true;
                    break;
                }

                if let Ok(parsed) = serde_json::from_str::<ChatCompletionChunk>(&data) {
                    if let Some(choice) = parsed.choices.first() {
                        if let Some(reasoning) = reasoning_delta_text(&choice.delta) {
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
                                let entry = accumulated_tool_calls.entry(tc.index).or_insert(
                                    AccumulatedToolCall {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                    },
                                );

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

            if done {
                break;
            }
        }

        if !turn_text.is_empty() && !accumulated_tool_calls.is_empty() {
            full_thinking.push_str(&turn_text);
            full_thinking.push('\n');
        }

        if accumulated_tool_calls.is_empty() {
            break;
        }

        let mut assistant_tool_calls_json = Vec::new();
        for (_, tc) in &accumulated_tool_calls {
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

        for (_, tc) in accumulated_tool_calls {
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

            emit_tool_event(&app_handle, &tc_id, &tc_name, "requested", &tc_args, None, None);
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

            let already_approved = {
                let approved_tool_types = state.approved_tool_types.lock().await;
                approved_tool_types.contains(&tc_name)
            };
            let requires_confirm = mode_requires_confirmation(&config, &tc_name, already_approved);

            let approved = if requires_confirm {
                request_tool_confirmation(
                    &state,
                    &app_handle,
                    &mut full_thinking,
                    &tc_id,
                    &tc_name,
                    &tc_args,
                    &pretty_args,
                )
                .await
            } else {
                true
            };

            let tool_output = if approved {
                emit_tool_event(
                    &app_handle,
                    &tc_id,
                    &tc_name,
                    "running",
                    &tc_args,
                    None,
                    Some(true),
                );
                let output = execute_tool(&tc_name, &tc_args).await;
                emit_tool_event(
                    &app_handle,
                    &tc_id,
                    &tc_name,
                    "completed",
                    &tc_args,
                    Some(&output),
                    Some(true),
                );

                if tc_name == "open_app" {
                    let success = output.contains("已启动");
                    let app_name = tc_args["app"].as_str().unwrap_or("应用");
                    let reply_text = if success {
                        format!("已成功为您打开了：{}！🐾", app_name)
                    } else {
                        format!("未能打开 {}：{}", app_name, output)
                    };
                    
                    let _ = app_handle.emit(
                        "ai-finished",
                        crate::AiFinishedPayload {
                            text: reply_text.clone(),
                            thinking: if full_thinking.is_empty() { None } else { Some(full_thinking.clone()) },
                        },
                    );
                    
                    let db = state.db.lock().await;
                    let _ = db.save_message_with_thinking(
                        "assistant",
                        &reply_text,
                        if full_thinking.is_empty() { None } else { Some(&full_thinking) },
                    );
                    drop(db);

                    {
                        let mut behavior = state.behavior.lock().await;
                        behavior.set_state(crate::behavior::PetState::Speaking);
                        behavior.mood.update(Some(0.05), None);
                    }
                    if let Ok(mut active) = state.active_chat.lock() {
                        active.active = None;
                    }
                    return;
                }

                output
            } else {
                "用户拒绝了此工具的操作权限。请说明为什么需要此权限，并让用户重新发起或授权。"
                    .to_string()
            };

            let output_display = format!("[执行结果]\n{}\n", tool_output);
            full_thinking.push_str(&output_display);
            append_active_thinking(&state, &output_display);
            let _ = app_handle.emit("ai-thinking", &output_display);
            push_tool_message(&mut api_messages, &tc_id, &tc_name, &tool_output);
        }
    }

    if full_text.trim().is_empty() {
        send_error(
            &app_handle,
            "API 请求结束了，但没有返回可显示的回复内容。".to_string(),
            &full_thinking,
        )
        .await;
        return;
    }

    {
        let db = state.db.lock().await;
        let _ = db.save_message_with_thinking(
            "assistant",
            &full_text,
            if full_thinking.is_empty() {
                None
            } else {
                Some(&full_thinking)
            },
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
        json!({
            "text": full_text,
            "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) }
        }),
    );
}

fn reasoning_delta_text(delta: &crate::direct_api::api_client::Delta) -> Option<String> {
    if let Some(text) = delta.reasoning_content.as_deref() {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }

    value_to_text(delta.reasoning.as_ref())
        .or_else(|| value_to_text(delta.thinking.as_ref()))
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

fn append_active_thinking(state: &tauri::State<'_, AppState>, text: &str) {
    if let Ok(mut active) = state.active_chat.lock() {
        if let Some(active) = active.active.as_mut() {
            active.thinking.push_str(text);
        }
    }
}

async fn is_cancelled(state: &tauri::State<'_, AppState>) -> bool {
    let abort = state.abort_token.lock().await;
    abort.as_ref().map(|token| token.is_cancelled()).unwrap_or(false)
}

async fn request_tool_confirmation(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    full_thinking: &mut String,
    event_id: &str,
    tool_name: &str,
    args: &serde_json::Value,
    pretty_args: &str,
) -> bool {
    let waiting_msg = format!("[等待授权] {}\n", tool_summary(tool_name, args));
    full_thinking.push_str(&waiting_msg);
    append_active_thinking(state, &waiting_msg);
    let _ = app_handle.emit("ai-thinking", &waiting_msg);
    emit_tool_event(app_handle, event_id, tool_name, "waiting", args, None, None);

    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
    let confirm_id = format!("confirm_{}_{}", tool_name, crate::unix_now());
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
            summary: tool_summary(tool_name, args),
            command: tool_command(tool_name, args),
            path: tool_path(tool_name, args),
        },
    );

    let approved_result = rx.await;

    {
        let mut confirms = state.pending_confirms.lock().await;
        confirms.remove(&confirm_id);
    }

    match approved_result {
        Ok(true) => {
            let approved_msg = "[用户已授权，开始执行]\n".to_string();
            full_thinking.push_str(&approved_msg);
            append_active_thinking(state, &approved_msg);
            let _ = app_handle.emit("ai-thinking", &approved_msg);
            {
                let mut approved_tool_types = state.approved_tool_types.lock().await;
                approved_tool_types.insert(tool_name.to_string());
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
        Ok(false) => {
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
        Err(_) => {
            let err_msg = "[授权通道失效或超时]\n".to_string();
            full_thinking.push_str(&err_msg);
            append_active_thinking(state, &err_msg);
            let _ = app_handle.emit("ai-thinking", &err_msg);
            emit_tool_event(
                app_handle,
                event_id,
                tool_name,
                "denied",
                args,
                Some("授权通道失效或超时。"),
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

fn mode_requires_confirmation(
    config: &DirectApiConfig,
    tool_name: &str,
    already_approved: bool,
) -> bool {
    if already_approved {
        return false;
    }
    // read_file 和 web_search 为安全/只读或已明确授权的工具，在任何模式下均不需要弹窗确认；
    // 敏感工具如 run_command 和 open_app 需要等待确认（open_app 启动本地应用，具有敏感性）。
    if tool_name == "read_file" || tool_name == "web_search" {
        return false;
    }
    match config.execution_mode.as_str() {
        "plan" | "unreviewed" => false,
        "custom" => !config.auto_approved_tools.iter().any(|tool| tool == tool_name),
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
        _ => None,
    }
}

fn tool_path(tool_name: &str, args: &serde_json::Value) -> Option<String> {
    match tool_name {
        "read_file" | "write_file" | "list_directory" => {
            args["path"].as_str().map(|value| value.to_string())
        }
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
            format!("执行命令 {}", args["command"].as_str().unwrap_or("未知命令"))
        }
        "web_search" => format!("搜索网页 {}", args["query"].as_str().unwrap_or("未知查询")),
        "open_app" => format!("打开应用 {}", args["app"].as_str().unwrap_or("未知应用")),
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

    let _ = app_handle.emit("ai-error", json!({
        "message": err_msg,
        "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
        "aborted": false
    }));
}

async fn send_aborted(app_handle: &tauri::AppHandle, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(crate::behavior::PetState::Idle);
    drop(behavior);

    let _ = app_handle.emit("ai-error", json!({
        "message": "已中止",
        "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
        "aborted": true
    }));
}
