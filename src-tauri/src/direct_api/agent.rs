use crate::direct_api::api_client::{
    call_chat_completions_stream, ChatCompletionChunk, DirectApiConfig, SseParser, UserAttachment,
};
use crate::direct_api::tools::{execute_tool, tool_definitions};
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

const MAX_AGENT_TURNS: u32 = 24;
const MAX_VISION_IMAGES: usize = 5;
const MAX_VISION_IMAGE_BYTES: u64 = 10 * 1024 * 1024;

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
    attachments: Vec<UserAttachment>,
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
        "content": build_user_content(&message, &attachments).await
    }));

    let mut current_turn = 0;
    let mut full_text = String::new();
    let mut full_thinking = String::new();

    loop {
        current_turn += 1;
        if current_turn > MAX_AGENT_TURNS {
            let limit_msg = format!("\n[已达到最大迭代限制 {MAX_AGENT_TURNS} 轮]");
            full_text.push_str(&limit_msg);
            let _ = app_handle.emit("ai-thinking", &limit_msg);
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
        let res =
            match call_chat_completions_stream(&state.http_client, &config, &api_messages, tools)
                .await
            {
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

            let chunk_res = match tokio::time::timeout(Duration::from_secs(25), stream.next()).await
            {
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
                            let _ = app_handle
                                .emit("ai-answer-delta", AnswerDeltaPayload { text: text.clone() });
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
                let output = execute_tool(&tc_name, &tc_args, &config.search_provider).await;
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
                            thinking: if full_thinking.is_empty() {
                                None
                            } else {
                                Some(full_thinking.clone())
                            },
                        },
                    );

                    let db = state.db.lock().await;
                    let _ = db.save_message_with_thinking(
                        "assistant",
                        &reply_text,
                        if full_thinking.is_empty() {
                            None
                        } else {
                            Some(&full_thinking)
                        },
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

    // 保存前再次检查是否已取消
    if is_cancelled(&state).await {
        send_aborted(&app_handle, &full_thinking).await;
        return;
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
            "{} 未作为视觉输入发送：图片超过 10MB",
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

fn append_active_thinking(state: &tauri::State<'_, AppState>, text: &str) {
    if let Ok(mut active) = state.active_chat.lock() {
        if let Some(active) = active.active.as_mut() {
            active.thinking.push_str(text);
        }
    }
}

async fn is_cancelled(state: &tauri::State<'_, AppState>) -> bool {
    let abort = state.abort_token.lock().await;
    abort
        .as_ref()
        .map(|token| token.is_cancelled())
        .unwrap_or(false)
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

    let approved_result = tokio::time::timeout(std::time::Duration::from_secs(120), rx).await;

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
        Ok(Ok(false)) => {
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
            format!(
                "执行命令 {}",
                args["command"].as_str().unwrap_or("未知命令")
            )
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

    let _ = app_handle.emit(
        "ai-error",
        json!({
            "message": err_msg,
            "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
            "aborted": false
        }),
    );
}

async fn send_aborted(app_handle: &tauri::AppHandle, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(crate::behavior::PetState::Idle);
    drop(behavior);

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
}
