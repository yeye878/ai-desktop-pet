mod behavior;
mod direct_api;
mod openclaw;
mod storage;
mod system;
mod tts;

use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Mutex as StdMutex,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{Emitter, Manager};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Serialize)]
pub struct ActiveChatSnapshot {
    pub message: String,
    pub thinking: String,
    pub started_at: i64,
}

#[derive(Default)]
pub struct ActiveChatState {
    pub active: Option<ActiveChatSnapshot>,
}

#[derive(Clone, Serialize)]
struct AiFinishedPayload {
    text: String,
    thinking: Option<String>,
}

#[derive(Clone, Serialize)]
struct AiErrorPayload {
    message: String,
    thinking: Option<String>,
    aborted: bool,
}

#[derive(Clone, Serialize)]
struct FileMetadata {
    path: String,
    name: String,
    size: u64,
    extension: String,
    exists: bool,
    is_file: bool,
}

pub struct AppState {
    pub ai: tokio::sync::Mutex<openclaw::ClaudeAdapter>,
    pub behavior: tokio::sync::Mutex<behavior::BehaviorEngine>,
    pub db: tokio::sync::Mutex<storage::Database>,
    pub monitor: tokio::sync::Mutex<system::SystemMonitor>,
    pub current_model: tokio::sync::Mutex<String>,
    pub skin: tokio::sync::Mutex<String>,
    pub font_color: tokio::sync::Mutex<String>,
    pub personality: tokio::sync::Mutex<String>,
    pub profession: tokio::sync::Mutex<String>,
    pub active_ai_pid: tokio::sync::Mutex<Option<u32>>,
    pub active_chat: StdMutex<ActiveChatState>,
    pub tts: tts::TtsManager,
    pub backend_type: tokio::sync::Mutex<String>,
    pub direct_api_config: tokio::sync::Mutex<Option<direct_api::DirectApiConfig>>,
    pub pending_confirms: tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub abort_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("taskkill");
        command.creation_flags(CREATE_NO_WINDOW);
        let _ = command.args(["/F", "/T", "/PID", &pid.to_string()]).output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

#[tauri::command]
async fn send_to_ai(
    message: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if message.trim().is_empty() {
        return Err("message is empty".to_string());
    }

    {
        let mut active = state
            .active_chat
            .lock()
            .map_err(|_| "active chat state poisoned".to_string())?;
        if active.active.is_some() {
            return Err("AI 正在思考中，请稍等当前回复完成。".to_string());
        }
        active.active = Some(ActiveChatSnapshot {
            message: message.clone(),
            thinking: String::new(),
            started_at: unix_now(),
        });
    }

    {
        let db = state.db.lock().await;
        db.save_message("user", &message)
            .map_err(|e| e.to_string())?;
    }

    // 用户交互 + 切换到思考状态
    {
        let mut behavior = state.behavior.lock().await;
        behavior.on_user_interaction();
        behavior.set_state(behavior::PetState::Thinking);
    }

    let _ = app_handle.emit("ai-started", &message);
    tauri::async_runtime::spawn(run_ai_message(app_handle, message));

    Ok(serde_json::json!({ "started": true }))
}

async fn run_ai_message(app_handle: tauri::AppHandle, message: String) {
    let state = app_handle.state::<AppState>();
    let backend = state.backend_type.lock().await.clone();

    if backend == "direct_api" {
        let system_prompt = {
            let personality = state.personality.lock().await;
            let profession = state.profession.lock().await;
            openclaw::build_system_prompt(&personality, &profession)
        };

        let chat_history = {
            let db = state.db.lock().await;
            db.get_recent_messages(10).unwrap_or_default()
        };

        let config_opt = state.direct_api_config.lock().await.clone();
        let Some(config) = config_opt else {
            {
                if let Ok(mut active) = state.active_chat.lock() {
                    active.active = None;
                }
                let mut behavior = state.behavior.lock().await;
                behavior.set_state(behavior::PetState::Confused);
            }
            let _ = app_handle.emit(
                "ai-error",
                AiErrorPayload {
                    message: "未配置直连 API 后端，请先在设置中配置 API Key。".to_string(),
                    thinking: None,
                    aborted: false,
                },
            );
            return;
        };

        // 创建新的 CancellationToken
        let token = tokio_util::sync::CancellationToken::new();
        {
            let mut abort = state.abort_token.lock().await;
            *abort = Some(token);
        }

        direct_api::run_direct_api_agent(
            app_handle.clone(),
            message,
            config,
            system_prompt,
            chat_history,
        )
        .await;

        // 执行结束，清理 abort_token
        {
            let mut abort = state.abort_token.lock().await;
            *abort = None;
        }
        return;
    }

    // 启动 Claude CLI 流式子进程
    let system_prompt = {
        let state = app_handle.state::<AppState>();
        let personality = state.personality.lock().await;
        let profession = state.profession.lock().await;
        openclaw::build_system_prompt(&personality, &profession)
    };

    let child_result = {
        let state = app_handle.state::<AppState>();
        let ai = state.ai.lock().await;
        ai.spawn_streaming(&app_handle, &message, &system_prompt)
    };
    let mut child = match child_result {
        Ok(child) => child,
        Err(err) => {
            {
                let state = app_handle.state::<AppState>();
                if let Ok(mut active) = state.active_chat.lock() {
                    active.active = None;
                }
                let mut behavior = state.behavior.lock().await;
                behavior.set_state(behavior::PetState::Confused);
            }
            let _ = app_handle.emit(
                "ai-error",
                AiErrorPayload {
                    message: format!("启动 Claude 失败: {err}"),
                    thinking: None,
                    aborted: false,
                },
            );
            return;
        }
    };
    let child_pid = child.id();

    // 只把 PID 存入 AppState，避免读取流时长期持有锁导致 abort_ai 卡住。
    if let Some(pid) = child_pid {
        let state = app_handle.state::<AppState>();
        let mut active = state.active_ai_pid.lock().await;
        *active = Some(pid);
    }

    // 从 child 读取流式输出
    let app_for_stream = app_handle.clone();
    let stream_result = openclaw::read_stream(&mut child, |delta| {
        {
            let state = app_for_stream.state::<AppState>();
            if let Ok(mut active) = state.active_chat.lock() {
                if let Some(active) = active.active.as_mut() {
                    active.thinking.push_str(delta);
                }
            };
        }
        let _ = app_for_stream.emit("ai-thinking", delta);
    })
    .await;

    // 等待子进程结束 + 清理
    let _ = child.wait().await;
    if let Some(pid) = child_pid {
        let state = app_handle.state::<AppState>();
        let mut active = state.active_ai_pid.lock().await;
        if *active == Some(pid) {
            *active = None;
        }
    }

    match stream_result {
        Ok(parsed) => {
            // 更新 session_id
            if let Some(ref sid) = parsed.session_id {
                let state = app_handle.state::<AppState>();
                let ai = state.ai.lock().await;
                ai.update_session(sid);
            }

            if parsed.is_error {
                let state = app_handle.state::<AppState>();
                let mut behavior = state.behavior.lock().await;
                behavior.set_state(behavior::PetState::Confused);
                let msg = parsed
                    .error_msg
                    .unwrap_or_else(|| "unknown error".to_string());
                if let Ok(mut active) = state.active_chat.lock() {
                    active.active = None;
                }
                let _ = app_handle.emit(
                    "ai-error",
                    AiErrorPayload {
                        message: format!("Claude error: {msg}"),
                        thinking: if parsed.full_thinking.is_empty() {
                            None
                        } else {
                            Some(parsed.full_thinking)
                        },
                        aborted: false,
                    },
                );
                return;
            }

            if parsed.full_text.is_empty() {
                // 可能是被中止了
                let state = app_handle.state::<AppState>();
                let mut behavior = state.behavior.lock().await;
                behavior.set_state(behavior::PetState::Idle);
                if let Ok(mut active) = state.active_chat.lock() {
                    active.active = None;
                }
                let _ = app_handle.emit(
                    "ai-error",
                    AiErrorPayload {
                        message: "已中止".to_string(),
                        thinking: if parsed.full_thinking.is_empty() {
                            None
                        } else {
                            Some(parsed.full_thinking)
                        },
                        aborted: true,
                    },
                );
                return;
            }

            // 保存对话
            let state = app_handle.state::<AppState>();
            {
                let db = state.db.lock().await;
                let _ = db.save_message_with_thinking(
                    "assistant",
                    &parsed.full_text,
                    if parsed.full_thinking.is_empty() {
                        None
                    } else {
                        Some(parsed.full_thinking.as_str())
                    },
                );
            }

            // 切换到说话状态
            let mut behavior = state.behavior.lock().await;
            behavior.set_state(behavior::PetState::Speaking);
            behavior.mood.update(Some(0.05), None);

            let thinking = if parsed.full_thinking.is_empty() {
                None
            } else {
                Some(parsed.full_thinking)
            };

            if let Ok(mut active) = state.active_chat.lock() {
                active.active = None;
            }

            let _ = app_handle.emit(
                "ai-finished",
                AiFinishedPayload {
                    text: parsed.full_text,
                    thinking,
                },
            );
        }
        Err(_) => {
            // 读取流出错（可能是子进程被 kill）
            let state = app_handle.state::<AppState>();
            let thinking = state
                .active_chat
                .lock()
                .ok()
                .and_then(|active| active.active.as_ref().map(|active| active.thinking.clone()))
                .filter(|thinking| !thinking.is_empty());
            if let Ok(mut active) = state.active_chat.lock() {
                active.active = None;
            }
            let mut behavior = state.behavior.lock().await;
            behavior.set_state(behavior::PetState::Idle);
            let _ = app_handle.emit(
                "ai-error",
                AiErrorPayload {
                    message: "已中止".to_string(),
                    thinking,
                    aborted: true,
                },
            );
        }
    }
}

#[tauri::command]
async fn abort_ai(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // 1. 中止 Direct API 模式
    {
        let abort = state.abort_token.lock().await;
        if let Some(ref token) = *abort {
            token.cancel();
        }
    }

    // 2. 中止 Claude Code 模式
    let pid = {
        let mut active = state.active_ai_pid.lock().await;
        active.take()
    };

    if let Some(pid) = pid {
        // Windows 上需要 kill 整个进程树（claude.cmd → node.exe）
        kill_process_tree(pid);
    }

    let mut behavior = state.behavior.lock().await;
    behavior.set_state(behavior::PetState::Idle);
    Ok(())
}

#[tauri::command]
async fn set_backend_type(backend: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut backend_type = state.backend_type.lock().await;
        *backend_type = backend.clone();
    }
    let db = state.db.lock().await;
    db.save_setting("backend_type", &backend).map_err(|e| e.to_string())?;

    // 切换后端时，清空当前活动会话
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    Ok(())
}

#[tauri::command]
async fn get_backend_type(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let backend_type = state.backend_type.lock().await;
    Ok(backend_type.clone())
}

#[tauri::command]
async fn set_api_config(
    api_key: String,
    base_url: String,
    model: String,
    confirm_enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_setting("api_key", &api_key).map_err(|e| e.to_string())?;
    db.save_setting("api_base_url", &base_url).map_err(|e| e.to_string())?;
    db.save_setting("api_model", &model).map_err(|e| e.to_string())?;
    db.save_setting("api_confirm_enabled", if confirm_enabled { "true" } else { "false" }).map_err(|e| e.to_string())?;

    let mut config = state.direct_api_config.lock().await;
    *config = Some(direct_api::DirectApiConfig {
        api_key,
        base_url,
        model,
        confirm_enabled,
    });

    Ok(())
}

#[tauri::command]
async fn get_api_config(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let api_key = db.get_setting("api_key").unwrap_or_default().unwrap_or_default();
    let base_url = db.get_setting("api_base_url").unwrap_or_default().unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let model = db.get_setting("api_model").unwrap_or_default().unwrap_or_else(|| "gpt-4o-mini".to_string());
    let confirm_enabled = db.get_setting("api_confirm_enabled").unwrap_or_default().unwrap_or_else(|| "true".to_string()) == "true";

    Ok(serde_json::json!({
        "api_key": api_key,
        "base_url": base_url,
        "model": model,
        "confirm_enabled": confirm_enabled,
    }))
}

#[tauri::command]
async fn test_api_connection(
    api_key: String,
    base_url: String,
    model: String,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = if base_url.ends_with("/chat/completions") {
        base_url.clone()
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "user", "content": "ping" }
        ],
        "max_tokens": 5
    });

    let res = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("连接请求发送失败: {e}"))?;

    let status = res.status();
    if status.is_success() {
        Ok("连接成功".to_string())
    } else {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        Err(format!("连接失败 ({}): {}", status, err_text))
    }
}

#[tauri::command]
async fn confirm_tool(
    id: String,
    approved: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut confirms = state.pending_confirms.lock().await;
    if let Some(tx) = confirms.remove(&id) {
        let _ = tx.send(approved);
        Ok(())
    } else {
        Err("无效的确认请求 ID".to_string())
    }
}

#[tauri::command]
async fn get_pet_state(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let behavior = state.behavior.lock().await;
    Ok(behavior.get_state_json())
}

#[tauri::command]
async fn set_pet_state(new_state: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let pet_state = match new_state.as_str() {
        "idle" => behavior::PetState::Idle,
        "listening" => behavior::PetState::Listening,
        "thinking" => behavior::PetState::Thinking,
        "speaking" => behavior::PetState::Speaking,
        "working" => behavior::PetState::Working,
        "happy" => behavior::PetState::Happy,
        "confused" => behavior::PetState::Confused,
        "sleeping" => behavior::PetState::Sleeping,
        "waving" => behavior::PetState::Waving,
        "dragging" => behavior::PetState::Dragging,
        "hungry" => behavior::PetState::Hungry,
        "stuffed" => behavior::PetState::Stuffed,
        "refusing" => behavior::PetState::Refusing,
        _ => behavior::PetState::Idle,
    };
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(pet_state);
    Ok(())
}

#[tauri::command]
async fn get_system_info(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let mut monitor = state.monitor.lock().await;
    monitor.refresh();
    Ok(serde_json::json!({
        "cpu": monitor.cpu_usage(),
        "memory": monitor.memory_usage_percent(),
    }))
}

#[tauri::command]
async fn get_chat_history(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let messages = db.get_recent_messages(50).map_err(|e| e.to_string())?;
    Ok(serde_json::json!(messages))
}

#[tauri::command]
async fn get_active_chat(
    state: tauri::State<'_, AppState>,
) -> Result<Option<ActiveChatSnapshot>, String> {
    let active = state
        .active_chat
        .lock()
        .map_err(|_| "active chat state poisoned".to_string())?;
    Ok(active.active.clone())
}

#[tauri::command]
async fn clear_chat_history(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.clear_messages().map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_memory(
    category: String,
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_memory(&category, &key, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_memories(
    category: String,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let memories = db
        .get_memories_by_category(&category)
        .map_err(|e| e.to_string())?;
    let items: Vec<serde_json::Value> = memories
        .iter()
        .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
        .collect();
    Ok(serde_json::json!(items))
}

#[tauri::command]
async fn save_clipboard_item(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_clipboard_item(&content).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_clipboard_items(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<storage::ClipboardItem>, String> {
    let db = state.db.lock().await;
    db.get_clipboard_items(100).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_clipboard_item(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_clipboard_item(id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_clipboard_item_pinned(
    id: i64,
    pinned: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.set_clipboard_item_pinned(id, pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn clear_clipboard_items(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.clear_clipboard_items().map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_user_avatar(avatar: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_setting("user_avatar", &avatar)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_user_avatar(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().await;
    db.get_setting("user_avatar")
        .map(|value| value.unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn tick(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let mut behavior = state.behavior.lock().await;
    behavior.tick();
    Ok(behavior.get_state_json())
}

#[tauri::command]
async fn switch_model(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut ai = state.ai.lock().await;
    *ai = openclaw::ClaudeAdapter::new();
    let mut current = state.current_model.lock().await;
    *current = "claude:claude-code".to_string();
    Ok(())
}

#[tauri::command]
async fn get_current_model(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let current = state.current_model.lock().await;
    Ok(current.clone())
}

#[tauri::command]
async fn set_skin(skin: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    {
        let mut s = state.skin.lock().await;
        *s = skin.clone();
    }
    let db = state.db.lock().await;
    db.save_setting("skin", &skin).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_skin(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let s = state.skin.lock().await;
    Ok(s.clone())
}

#[tauri::command]
async fn set_font_color(
    font_color: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    {
        let mut fc = state.font_color.lock().await;
        *fc = font_color.clone();
    }
    let db = state.db.lock().await;
    db.save_setting("font_color", &font_color)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_font_color(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let fc = state.font_color.lock().await;
    Ok(fc.clone())
}

#[tauri::command]
async fn set_setting_value(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_setting(&key, &value).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_setting_value(
    key: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let db = state.db.lock().await;
    db.get_setting(&key)
        .map(|value| value.unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_personality(
    personality: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let normalized = openclaw::normalize_personality_id(&personality);
    {
        let mut p = state.personality.lock().await;
        *p = normalized.clone();
    }
    {
        let ai = state.ai.lock().await;
        ai.reset_session();
    }
    let db = state.db.lock().await;
    db.save_setting("personality", &normalized)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_personality(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let p = state.personality.lock().await;
    Ok(p.clone())
}

#[tauri::command]
async fn set_profession(
    profession: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let normalized = openclaw::normalize_profession_id(&profession);
    {
        let mut p = state.profession.lock().await;
        *p = normalized.clone();
    }
    {
        let ai = state.ai.lock().await;
        ai.reset_session();
    }
    let db = state.db.lock().await;
    db.save_setting("profession", &normalized)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_profession(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let p = state.profession.lock().await;
    Ok(p.clone())
}

#[tauri::command]
async fn eat_files(paths: Vec<String>) -> Result<(), String> {
    trash::delete_all(&paths).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_file_metadata(paths: Vec<String>) -> Result<Vec<FileMetadata>, String> {
    let mut results = Vec::new();
    for p in &paths {
        let path = std::path::Path::new(p);
        let exists = path.exists();
        let is_file = exists && path.is_file();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let size = if is_file {
            tokio::fs::metadata(p).await.map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };
        results.push(FileMetadata {
            path: p.clone(),
            name,
            size,
            extension,
            exists,
            is_file,
        });
    }
    Ok(results)
}

#[tauri::command]
async fn read_file_as_data_url(path: String) -> Result<String, String> {
    use base64::Engine;
    let p = std::path::Path::new(&path);
    if !p.exists() || !p.is_file() {
        return Err("文件不存在".into());
    }
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|e| e.to_string())?
        .len();
    if size > 5 * 1024 * 1024 {
        return Err("文件过大，无法生成预览 (>5MB)".into());
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

#[tauri::command]
async fn open_claude_config(app_handle: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::path::PathBuf;
        
        let mut cmd_path = PathBuf::from("claude.cmd");
        let mut node_dir = None;

        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            let bundled_node_dir = resource_dir.join("node");
            let path_options = vec![
                bundled_node_dir.join("claude.cmd"),
                bundled_node_dir.join("node_modules").join(".bin").join("claude.cmd"),
            ];

            for path in path_options {
                if path.exists() {
                    cmd_path = path;
                    node_dir = Some(bundled_node_dir);
                    break;
                }
            }
        }

        let script = if let Some(ref ndir) = node_dir {
            format!(
                "clear; Write-Host '==================================================' -ForegroundColor Cyan; \
                 Write-Host '   智能桌宠 - Claude Code 登录与配置向导' -ForegroundColor Green; \
                 Write-Host '==================================================' -ForegroundColor Cyan; \
                 Write-Host '即将为您拉起内置的 Claude Code CLI 进行认证登录...' -ForegroundColor Yellow; \
                 $env:PATH = '{}' + ';' + $env:PATH; \
                 & '{}'; \
                 Write-Host '配置结束。按任意键关闭此窗口...' -ForegroundColor Cyan; \
                 $null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')",
                ndir.to_string_lossy().replace('\\', "\\\\"),
                cmd_path.to_string_lossy().replace('\\', "\\\\")
            )
        } else {
            "clear; Write-Host '没有找到内置的便携式 Node / Claude Code。将尝试调用全局命令...' -ForegroundColor Yellow; \
             & 'claude.cmd'; \
             Write-Host '配置配置结束。按任意键关闭此窗口...' -ForegroundColor Cyan; \
             $null = $Host.UI.RawUI.ReadKey('NoEcho,IncludeKeyDown')".to_string()
        };

        std::process::Command::new("powershell.exe")
            .args(["-NoExit", "-Command", &script])
            .spawn()
            .map_err(|e| format!("Failed to spawn PowerShell: {}", e))?;
        
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Unsupported operating system".to_string())
    }
}

#[tauri::command]
fn exit_app() {
    std::process::exit(0);
}

#[derive(Serialize)]
pub struct ClaudeStatus {
    pub logged_in: bool,
    pub token_preview: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[tauri::command]
async fn check_claude_status() -> Result<ClaudeStatus, String> {
    let mut path = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    path.push(".claude");
    path.push("settings.json");

    if !path.exists() {
        return Ok(ClaudeStatus {
            logged_in: false,
            token_preview: None,
            model: None,
            base_url: None,
        });
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("无法读取 Claude 配置文件: {e}"))?;

    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 Claude 配置文件失败: {e}"))?;

    let env = json.get("env");
    let token = env.and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN")).and_then(|t| t.as_str());
    let model = env.and_then(|e| e.get("ANTHROPIC_MODEL")).and_then(|m| m.as_str()).map(|s| s.to_string());
    let base_url = env.and_then(|e| e.get("ANTHROPIC_BASE_URL")).and_then(|u| u.as_str()).map(|s| s.to_string());

    if let Some(tok) = token {
        if !tok.trim().is_empty() {
            let preview = if tok.len() > 8 {
                format!("{}...", &tok[..8])
            } else {
                tok.to_string()
            };
            return Ok(ClaudeStatus {
                logged_in: true,
                token_preview: Some(preview),
                model,
                base_url,
            });
        }
    }

    Ok(ClaudeStatus {
        logged_in: false,
        token_preview: None,
        model,
        base_url,
    })
}

#[tauri::command]
async fn tts_synthesize(
    text: String,
    voice: String,
    rate: Option<i32>,
    pitch: Option<i32>,
    volume: Option<i32>,
    state: tauri::State<'_, AppState>,
) -> Result<tts::TtsResult, String> {
    let req = tts::TtsRequest {
        text,
        voice,
        rate: rate.unwrap_or(0),
        pitch: pitch.unwrap_or(0),
        volume: volume.unwrap_or(100),
    };
    state.tts.synthesize(&req).await
}

#[tauri::command]
async fn tts_list_voices() -> Result<Vec<tts::TtsVoice>, String> {
    tts::TtsManager::list_voices().await
}

fn get_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("ai-desktop-pet");
    std::fs::create_dir_all(&path).ok();
    path.push("pet.db");
    path
}

fn tts_cache_dir(app: &tauri::App) -> PathBuf {
    let mut path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("ai-desktop-pet"));
    path.push("tts_cache");
    path
}

pub fn run() {
    let db_path = get_db_path();
    let db = storage::Database::new(&db_path).expect("failed to open database");
    let personality = db
        .get_setting("personality")
        .ok()
        .flatten()
        .map(|value| openclaw::normalize_personality_id(&value))
        .unwrap_or_else(|| openclaw::DEFAULT_PERSONALITY.to_string());
    let profession = db
        .get_setting("profession")
        .ok()
        .flatten()
        .map(|value| openclaw::normalize_profession_id(&value))
        .unwrap_or_else(|| openclaw::DEFAULT_PROFESSION.to_string());
    let skin = db
        .get_setting("skin")
        .ok()
        .flatten()
        .unwrap_or_else(|| "default".to_string());
    let font_color = db
        .get_setting("font_color")
        .ok()
        .flatten()
        .unwrap_or_default();

    let backend_type = db
        .get_setting("backend_type")
        .ok()
        .flatten()
        .unwrap_or_else(|| "claude_code".to_string());

    let api_key = db.get_setting("api_key").ok().flatten().unwrap_or_default();
    let api_base_url = db
        .get_setting("api_base_url")
        .ok()
        .flatten()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_model = db
        .get_setting("api_model")
        .ok()
        .flatten()
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let api_confirm_enabled = db
        .get_setting("api_confirm_enabled")
        .ok()
        .flatten()
        .unwrap_or_else(|| "true".to_string())
        == "true";

    let direct_config = if !api_key.is_empty() {
        Some(direct_api::DirectApiConfig {
            api_key,
            base_url: api_base_url,
            model: api_model,
            confirm_enabled: api_confirm_enabled,
        })
    } else {
        None
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let app_state = AppState {
                ai: tokio::sync::Mutex::new(openclaw::ClaudeAdapter::new()),
                behavior: tokio::sync::Mutex::new(behavior::BehaviorEngine::new()),
                db: tokio::sync::Mutex::new(db),
                monitor: tokio::sync::Mutex::new(system::SystemMonitor::new()),
                current_model: tokio::sync::Mutex::new("claude:claude-code".to_string()),
                skin: tokio::sync::Mutex::new(skin),
                font_color: tokio::sync::Mutex::new(font_color),
                personality: tokio::sync::Mutex::new(personality),
                profession: tokio::sync::Mutex::new(profession),
                active_ai_pid: tokio::sync::Mutex::new(None),
                active_chat: StdMutex::new(ActiveChatState::default()),
                tts: tts::TtsManager::new(tts_cache_dir(app)),
                backend_type: tokio::sync::Mutex::new(backend_type),
                direct_api_config: tokio::sync::Mutex::new(direct_config),
                pending_confirms: tokio::sync::Mutex::new(std::collections::HashMap::new()),
                abort_token: tokio::sync::Mutex::new(None),
            };
            app.manage(app_state);

            // main 窗口现在是控制台，无需置顶

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_to_ai,
            abort_ai,
            get_pet_state,
            set_pet_state,
            get_system_info,
            get_chat_history,
            get_active_chat,
            clear_chat_history,
            save_memory,
            get_memories,
            save_clipboard_item,
            get_clipboard_items,
            delete_clipboard_item,
            set_clipboard_item_pinned,
            clear_clipboard_items,
            set_user_avatar,
            get_user_avatar,
            tick,
            switch_model,
            get_current_model,
            set_skin,
            get_skin,
            set_font_color,
            get_font_color,
            set_setting_value,
            get_setting_value,
            set_personality,
            get_personality,
            set_profession,
            get_profession,
            eat_files,
            get_file_metadata,
            read_file_as_data_url,
            exit_app,
            open_claude_config,
            tts_synthesize,
            tts_list_voices,
            set_backend_type,
            get_backend_type,
            set_api_config,
            get_api_config,
            test_api_connection,
            confirm_tool,
            check_claude_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
