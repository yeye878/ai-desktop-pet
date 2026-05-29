use crate::AppState;
use crate::storage::ChatMessage;
use crate::direct_api::api_client::{call_chat_completions_stream, ChatCompletionChunk, DirectApiConfig, SseParser};
use crate::direct_api::tools::{execute_tool, tool_definitions};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
struct ToolConfirmPayload {
    id: String,
    tool_name: String,
    arguments: String,
}

struct AccumulatedToolCall {
    id: String,
    name: String,
    arguments: String,
}

pub async fn run_direct_api_agent(
    app_handle: tauri::AppHandle,
    message: String,
    config: DirectApiConfig,
    system_prompt: String,
    chat_history: Vec<ChatMessage>,
) {
    let state = app_handle.state::<AppState>();

    // 1. 构建初始 messages 数组
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
            let limit_msg = "\n[已达到最大迭代限制 (10轮)]";
            full_text.push_str(limit_msg);
            let _ = app_handle.emit("ai-thinking", limit_msg);
            break;
        }

        // 检查是否已被中止
        {
            let abort = state.abort_token.lock().await;
            if let Some(ref token) = *abort {
                if token.is_cancelled() {
                    send_aborted(&app_handle, &full_thinking);
                    return;
                }
            }
        }

        // 调用 API 获取流式响应
        let tools_list = tool_definitions();
        let res_result = call_chat_completions_stream(&config, &api_messages, Some(tools_list)).await;

        let res = match res_result {
            Ok(res) => res,
            Err(e) => {
                send_error(&app_handle, format!("API 请求失败: {e}"), &full_thinking);
                return;
            }
        };

        // 读取流式响应
        let mut stream = res.bytes_stream();
        let mut sse = SseParser::new();
        let mut accumulated_tool_calls: HashMap<usize, AccumulatedToolCall> = HashMap::new();
        let mut turn_text = String::new();

        while let Some(chunk_res) = stream.next().await {
            // 每次读取块前检查是否被中止
            {
                let abort = state.abort_token.lock().await;
                if let Some(ref token) = *abort {
                    if token.is_cancelled() {
                        send_aborted(&app_handle, &full_thinking);
                        return;
                    }
                }
            }

            let chunk_bytes = match chunk_res {
                Ok(b) => b,
                Err(e) => {
                    send_error(&app_handle, format!("流式读取失败: {e}"), &full_thinking);
                    return;
                }
            };

            let chunk_str = String::from_utf8_lossy(&chunk_bytes);
            let data_lines = sse.push(&chunk_str);

            for data in data_lines {
                if data == "[DONE]" {
                    break;
                }

                if let Ok(parsed) = serde_json::from_str::<ChatCompletionChunk>(&data) {
                    if let Some(choice) = parsed.choices.first() {
                        // 1. 文本内容 delta
                        if let Some(ref text) = choice.delta.content {
                            turn_text.push_str(text);
                            full_text.push_str(text);
                            let _ = app_handle.emit("ai-thinking", text);
                        }

                        // 2. 工具调用 delta
                        if let Some(ref tool_calls) = choice.delta.tool_calls {
                            for tc in tool_calls {
                                let entry = accumulated_tool_calls.entry(tc.index).or_insert(AccumulatedToolCall {
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

        // 如果本轮产生了文本，我们将其累积
        if !turn_text.is_empty() {
            // 如果是在 Agent 循环中产生的非最终文本，我们把它加到 thinking 里方便追溯
            if !accumulated_tool_calls.is_empty() {
                full_thinking.push_str(&turn_text);
                full_thinking.push('\n');
            }
        }

        // 如果没有工具调用，说明 Agent 已经给出了最终回复，退出循环
        if accumulated_tool_calls.is_empty() {
            break;
        }

        // 将助理消息（包含工具调用定义）推入 api_messages
        let mut assistants_tool_calls_json = Vec::new();
        for (_, tc) in &accumulated_tool_calls {
            assistants_tool_calls_json.push(json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": tc.arguments
                }
            }));
        }

        // 构建 assistant 消息
        let assistant_msg = json!({
            "role": "assistant",
            "content": if turn_text.is_empty() { None } else { Some(turn_text.as_str()) },
            "tool_calls": assistants_tool_calls_json
        });
        api_messages.push(assistant_msg);

        // 执行所有的工具调用
        for (_, tc) in accumulated_tool_calls {
            let tc_id = tc.id;
            let tc_name = tc.name;
            let tc_args_str = tc.arguments;

            let tc_args: serde_json::Value = serde_json::from_str(&tc_args_str).unwrap_or(json!({}));

            // 显示在思考框中
            let starting_msg = format!("\n🤖 [调用工具] {}: {}\n", tc_name, tc_args_str);
            full_thinking.push_str(&starting_msg);
            let _ = app_handle.emit("ai-thinking", &starting_msg);

            // 检查是否需要进行交互式确认
            let requires_confirm = config.confirm_enabled && (tc_name == "run_command" || tc_name == "write_file");

            let approved = if requires_confirm {
                let waiting_msg = "⏳ [等待用户授权执行敏感操作...]\n".to_string();
                full_thinking.push_str(&waiting_msg);
                let _ = app_handle.emit("ai-thinking", &waiting_msg);

                // 创建 oneshot channel 并存入 AppState
                let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                let confirm_id = format!("confirm_{}_{}", tc_name, crate::unix_now());
                {
                    let mut confirms = state.pending_confirms.lock().await;
                    confirms.insert(confirm_id.clone(), tx);
                }

                // 向上级 emit 确认请求
                let _ = app_handle.emit("ai-tool-confirm", ToolConfirmPayload {
                    id: confirm_id.clone(),
                    tool_name: tc_name.clone(),
                    arguments: tc_args_str.clone(),
                });

                // 等待用户点击
                let approved_result = rx.await;
                
                // 从 AppState 移除以防泄漏（正常情况下 oneshot 触发时已被 remove，此处为兜底）
                {
                    let mut confirms = state.pending_confirms.lock().await;
                    confirms.remove(&confirm_id);
                }

                match approved_result {
                    Ok(appr) => {
                        if appr {
                            let approved_msg = "✅ [用户已授权，开始执行]\n".to_string();
                            full_thinking.push_str(&approved_msg);
                            let _ = app_handle.emit("ai-thinking", &approved_msg);
                            true
                        } else {
                            let denied_msg = "❌ [用户拒绝执行此敏感操作]\n".to_string();
                            full_thinking.push_str(&denied_msg);
                            let _ = app_handle.emit("ai-thinking", &denied_msg);
                            false
                        }
                    }
                    Err(_) => {
                        let err_msg = "⚠️ [授权通道失效或超时]\n".to_string();
                        full_thinking.push_str(&err_msg);
                        let _ = app_handle.emit("ai-thinking", &err_msg);
                        false
                    }
                }
            } else {
                true
            };

            let tool_output = if approved {
                execute_tool(&tc_name, &tc_args).await
            } else {
                "用户拒绝了此工具的操作权限。请向用户说明为何需要此权限，并礼貌地让用户重新发起或授权。".to_string()
            };

            // 输出显示到思考框
            let output_display = format!("📊 [执行结果]\n{}\n", tool_output);
            full_thinking.push_str(&output_display);
            let _ = app_handle.emit("ai-thinking", &output_display);

            // 推入 api_messages
            api_messages.push(json!({
                "role": "tool",
                "tool_call_id": tc_id,
                "name": tc_name,
                "content": tool_output
            }));
        }
    }

    // 保存最终的 assistant 回复到数据库
    {
        let db = state.db.lock().await;
        let _ = db.save_message_with_thinking(
            "assistant",
            &full_text,
            if full_thinking.is_empty() { None } else { Some(&full_thinking) }
        );
    }

    // 播放语音/设置状态并结束
    {
        let mut behavior = state.behavior.lock().await;
        behavior.set_state(crate::behavior::PetState::Speaking);
        behavior.mood.update(Some(0.05), None);
    }

    // 触发 tts 播放
    {
        let _ = app_handle.emit(
            "ai-finished",
            json!({
                "text": full_text,
                "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) }
            })
        );
    }
}

fn send_error(app_handle: &tauri::AppHandle, err_msg: String, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = tauri::async_runtime::block_on(async { state.behavior.lock().await });
    behavior.set_state(crate::behavior::PetState::Confused);

    let _ = app_handle.emit("ai-error", json!({
        "message": err_msg,
        "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
        "aborted": false
    }));
}

fn send_aborted(app_handle: &tauri::AppHandle, full_thinking: &str) {
    let state = app_handle.state::<AppState>();
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let mut behavior = tauri::async_runtime::block_on(async { state.behavior.lock().await });
    behavior.set_state(crate::behavior::PetState::Idle);

    let _ = app_handle.emit("ai-error", json!({
        "message": "已中止",
        "thinking": if full_thinking.is_empty() { None } else { Some(full_thinking) },
        "aborted": true
    }));
}
