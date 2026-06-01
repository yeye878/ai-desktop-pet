mod behavior;
mod direct_api;
mod openclaw;
mod storage;
mod system;
mod tts;

use serde::{Deserialize, Serialize};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    path::PathBuf,
    sync::Mutex as StdMutex,
    time::{SystemTime, UNIX_EPOCH},
};
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
    pub pending_confirms:
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub approved_tool_types: tokio::sync::Mutex<std::collections::HashSet<String>>,
    pub abort_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
    pub chat_start_id: StdMutex<i64>,
    pub http_client: reqwest::Client,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

const API_PROFILES_KEY: &str = "api_model_profiles";
const ACTIVE_API_PROFILE_KEY: &str = "active_api_profile_id";
const API_THINKING_DEPTH_KEY: &str = "api_thinking_depth";
const API_EXECUTION_MODE_KEY: &str = "api_execution_mode";
const API_SEARCH_PROVIDER_KEY: &str = "api_search_provider";
const API_AUTO_APPROVED_TOOLS_KEY: &str = "api_auto_approved_tools";
const MEMORY_CATEGORY_MANUAL: &str = "manual";
const MEMORY_CATEGORY_CONVERSATION: &str = "conversation";

#[derive(Debug, Deserialize)]
struct ApiProfileStore {
    profiles: Vec<direct_api::DirectApiConfig>,
}

fn normalize_thinking_depth(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "low" | "medium" | "high" => value.trim().to_lowercase(),
        _ => "auto".to_string(),
    }
}

fn normalize_execution_mode(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "plan" | "normal" | "unreviewed" | "custom" => value.trim().to_lowercase(),
        _ => "normal".to_string(),
    }
}

fn normalize_search_provider(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "duckduckgo" | "duck" | "ddg" => "duckduckgo".to_string(),
        _ => "bing".to_string(),
    }
}

fn normalize_tool_list(values: &[String]) -> Vec<String> {
    let mut tools = values
        .iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect::<Vec<_>>();
    tools.sort();
    tools.dedup();
    tools
}

fn mask_api_key(api_key: &str) -> String {
    let key = api_key.trim();
    if key.is_empty() {
        return String::new();
    }

    let prefix: String = key.chars().take(4).collect();
    let suffix_chars = key.chars().rev().take(4).collect::<Vec<_>>();
    let suffix: String = suffix_chars.into_iter().rev().collect();
    format!("{prefix}••••{suffix}")
}

fn is_masked_api_key(api_key: &str) -> bool {
    let key = api_key.trim();
    key.contains('•') || key.contains('*')
}

fn public_api_profile(profile: &direct_api::DirectApiConfig) -> serde_json::Value {
    serde_json::json!({
        "id": profile.id.clone(),
        "name": profile.name.clone(),
        "api_key": "",
        "api_key_mask": mask_api_key(&profile.api_key),
        "has_api_key": !profile.api_key.trim().is_empty(),
        "base_url": profile.base_url.clone(),
        "model": profile.model.clone(),
        "confirm_enabled": profile.confirm_enabled,
        "thinking_depth": profile.thinking_depth.clone(),
        "execution_mode": profile.execution_mode.clone(),
        "search_provider": profile.search_provider.clone(),
        "auto_approved_tools": profile.auto_approved_tools.clone(),
    })
}

fn normalize_api_profile(mut profile: direct_api::DirectApiConfig) -> direct_api::DirectApiConfig {
    profile.id = profile.id.trim().to_string();
    if profile.id.is_empty() {
        profile.id = format!("api_{}", uuid::Uuid::new_v4().simple());
    }

    profile.name = profile.name.trim().to_string();
    profile.model = profile.model.trim().to_string();
    profile.base_url = profile.base_url.trim().trim_end_matches('/').to_string();
    profile.api_key = profile.api_key.trim().to_string();
    profile.thinking_depth = normalize_thinking_depth(&profile.thinking_depth);
    profile.execution_mode = normalize_execution_mode(&profile.execution_mode);
    profile.search_provider = normalize_search_provider(&profile.search_provider);
    profile.auto_approved_tools = normalize_tool_list(&profile.auto_approved_tools);

    if profile.model.is_empty() {
        profile.model = "gpt-4o-mini".to_string();
    }
    if profile.name.is_empty() {
        profile.name = profile.model.clone();
    }
    if profile.base_url.is_empty() {
        profile.base_url = "https://api.openai.com/v1".to_string();
    }

    profile
}

fn load_api_profiles(db: &storage::Database) -> Vec<direct_api::DirectApiConfig> {
    let stored = db
        .get_setting(API_PROFILES_KEY)
        .ok()
        .flatten()
        .and_then(|raw| {
            serde_json::from_str::<Vec<direct_api::DirectApiConfig>>(&raw)
                .ok()
                .or_else(|| {
                    serde_json::from_str::<ApiProfileStore>(&raw)
                        .ok()
                        .map(|store| store.profiles)
                })
        })
        .unwrap_or_default();

    let mut profiles: Vec<direct_api::DirectApiConfig> =
        stored.into_iter().map(normalize_api_profile).collect();

    let mut migrated = false;

    if profiles.is_empty() {
        let api_key = db.get_setting("api_key").ok().flatten().unwrap_or_default();
        let base_url = db
            .get_setting("api_base_url")
            .ok()
            .flatten()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = db
            .get_setting("api_model")
            .ok()
            .flatten()
            .unwrap_or_else(|| "gpt-4o-mini".to_string());
        let confirm_enabled = db
            .get_setting("api_confirm_enabled")
            .ok()
            .flatten()
            .unwrap_or_else(|| "true".to_string())
            == "true";
        let thinking_depth = db
            .get_setting(API_THINKING_DEPTH_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| "auto".to_string());
        let execution_mode = db
            .get_setting(API_EXECUTION_MODE_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if confirm_enabled {
                    "normal".to_string()
                } else {
                    "unreviewed".to_string()
                }
            });
        let auto_approved_tools = db
            .get_setting(API_AUTO_APPROVED_TOOLS_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default();
        let search_provider = db
            .get_setting(API_SEARCH_PROVIDER_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| "bing".to_string());

        let default_profile = normalize_api_profile(direct_api::DirectApiConfig {
            id: "default".to_string(),
            name: model.clone(),
            api_key,
            base_url,
            model,
            confirm_enabled,
            thinking_depth,
            execution_mode,
            search_provider,
            auto_approved_tools,
        });

        profiles.push(default_profile);
        migrated = true;
    }

    for profile in &mut profiles {
        if let Some(key) = system::credential::get_api_key(&profile.id) {
            profile.api_key = key;
        } else if !profile.api_key.is_empty() {
            // Migrate plaintext key from DB to system keyring
            if system::credential::set_api_key(&profile.id, &profile.api_key).is_ok() {
                migrated = true;
            }
        }
    }

    if migrated {
        let _ = save_api_profiles(db, &profiles);
    }

    profiles
}

fn save_api_profiles(
    db: &storage::Database,
    profiles: &[direct_api::DirectApiConfig],
) -> Result<(), String> {
    let mut db_profiles = profiles.to_vec();
    for profile in &mut db_profiles {
        if !profile.api_key.is_empty() && !is_masked_api_key(&profile.api_key) {
            if system::credential::set_api_key(&profile.id, &profile.api_key).is_ok() {
                profile.api_key = String::new();
            }
        } else if is_masked_api_key(&profile.api_key) {
            profile.api_key = String::new();
        }
    }
    let json = serde_json::to_string(&db_profiles).map_err(|e| e.to_string())?;
    db.save_setting(API_PROFILES_KEY, &json)
        .map_err(|e| e.to_string())?;

    if db_profiles.iter().all(|profile| profile.api_key.is_empty()) {
        let _ = db.save_setting("api_key", "");
    }
    Ok(())
}

fn active_api_profile(db: &storage::Database) -> direct_api::DirectApiConfig {
    let profiles = load_api_profiles(db);
    let active_id = db.get_setting(ACTIVE_API_PROFILE_KEY).ok().flatten();
    active_id
        .as_deref()
        .and_then(|id| profiles.iter().find(|profile| profile.id == id))
        .cloned()
        .or_else(|| profiles.first().cloned())
        .unwrap_or_default()
}

fn save_legacy_api_settings(
    db: &storage::Database,
    profile: &direct_api::DirectApiConfig,
) -> Result<(), String> {
    db.save_setting("api_key", "").map_err(|e| e.to_string())?;
    db.save_setting("api_base_url", &profile.base_url)
        .map_err(|e| e.to_string())?;
    db.save_setting("api_model", &profile.model)
        .map_err(|e| e.to_string())?;
    db.save_setting(
        "api_confirm_enabled",
        if profile.confirm_enabled {
            "true"
        } else {
            "false"
        },
    )
    .map_err(|e| e.to_string())?;
    db.save_setting(API_THINKING_DEPTH_KEY, &profile.thinking_depth)
        .map_err(|e| e.to_string())?;
    db.save_setting(API_EXECUTION_MODE_KEY, &profile.execution_mode)
        .map_err(|e| e.to_string())?;
    db.save_setting(API_SEARCH_PROVIDER_KEY, &profile.search_provider)
        .map_err(|e| e.to_string())?;
    let tools_json =
        serde_json::to_string(&profile.auto_approved_tools).map_err(|e| e.to_string())?;
    db.save_setting(API_AUTO_APPROVED_TOOLS_KEY, &tools_json)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn attach_execution_mode_prompt(
    system_prompt: String,
    config: &direct_api::DirectApiConfig,
) -> String {
    let mode_note = match config.execution_mode.as_str() {
        "plan" => {
            "当前执行模式：计划模式。你只能制定计划、说明将使用哪些工具和会修改哪些文件；不要调用任何工具，不要执行命令，不要写入文件。"
        }
        "unreviewed" => {
            "当前执行模式：无审查模式。你可以在任务需要时直接调用可用工具，但仍要保持谨慎并清楚说明正在做什么。"
        }
        "custom" => {
            "当前执行模式：自定义模式。部分工具可能免确认，未免确认的操作会先等待用户授权；需要操作时直接发起工具调用。"
        }
        _ => {
            "当前执行模式：普通模式。每类工具在本段对话中首次使用前会等待用户授权；同类工具获得授权后，本段对话内不再重复确认。"
        }
    };
    format!("{}\n\n{}", system_prompt, mode_note)
}

fn attach_runtime_identity_prompt(
    system_prompt: String,
    config: &direct_api::DirectApiConfig,
) -> String {
    let model = config.model.trim();
    if model.is_empty() {
        return system_prompt;
    }

    let profile_name = config.name.trim();
    let profile_note = if !profile_name.is_empty() && profile_name != model {
        format!("，配置名称是「{}」", profile_name)
    } else {
        String::new()
    };

    let identity_note = format!(
        "当前运行环境：你正在通过本应用的直连 API 配置响应用户。请求使用的模型 ID 是「{}」{}。如果用户询问“你是什么模型”或类似问题，请回答这个模型 ID；同时说明这代表应用请求的模型配置，不保证服务商内部没有路由或别名映射。不要主动透露 API Key 或完整接口地址。",
        model, profile_note
    );

    format!("{}\n\n{}", system_prompt, identity_note)
}

fn upsert_api_profile(
    db: &storage::Database,
    profile: direct_api::DirectApiConfig,
    activate: bool,
) -> Result<direct_api::DirectApiConfig, String> {
    let profile = normalize_api_profile(profile);
    let mut profiles = load_api_profiles(db);
    if let Some(existing) = profiles.iter_mut().find(|item| item.id == profile.id) {
        *existing = profile.clone();
    } else {
        profiles.push(profile.clone());
    }

    save_api_profiles(db, &profiles)?;
    if activate {
        db.save_setting(ACTIVE_API_PROFILE_KEY, &profile.id)
            .map_err(|e| e.to_string())?;
        save_legacy_api_settings(db, &profile)?;
    }

    Ok(profile)
}

fn compact_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut value = compact.chars().take(max_chars).collect::<String>();
    value.push_str("...");
    value
}

fn summarize_chat_messages(messages: &[storage::ChatMessage]) -> Option<(String, String)> {
    let meaningful: Vec<&storage::ChatMessage> = messages
        .iter()
        .filter(|msg| !msg.content.trim().is_empty())
        .collect();

    if meaningful.len() < 2 {
        return None;
    }

    let first_time = meaningful
        .first()
        .map(|msg| msg.created_at)
        .unwrap_or_default();
    let last_time = meaningful
        .last()
        .map(|msg| msg.created_at)
        .unwrap_or_default();

    let mut user_points = meaningful
        .iter()
        .filter(|msg| msg.role == "user")
        .rev()
        .take(5)
        .map(|msg| compact_text(&msg.content, 120))
        .collect::<Vec<_>>();
    user_points.reverse();

    let mut assistant_points = meaningful
        .iter()
        .filter(|msg| msg.role == "assistant")
        .rev()
        .take(3)
        .map(|msg| compact_text(&msg.content, 120))
        .collect::<Vec<_>>();
    assistant_points.reverse();

    let key = format!("对话摘要 {}", last_time);
    let value = format!(
        "时间范围 {}-{}，共 {} 条消息。用户重点：{}。AI 回应重点：{}。",
        first_time,
        last_time,
        meaningful.len(),
        if user_points.is_empty() {
            "无明确用户消息".to_string()
        } else {
            user_points.join(" / ")
        },
        if assistant_points.is_empty() {
            "无明确助手回复".to_string()
        } else {
            assistant_points.join(" / ")
        }
    );

    Some((key, value))
}

fn save_chat_summary_if_needed(db: &storage::Database) -> Result<Option<i64>, String> {
    let messages = db.get_recent_messages(50).map_err(|e| e.to_string())?;
    let Some((key, value)) = summarize_chat_messages(&messages) else {
        return Ok(None);
    };
    db.save_memory(MEMORY_CATEGORY_CONVERSATION, &key, &value)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn parse_manual_memory_request(message: &str) -> Option<(String, String)> {
    let trimmed = message.trim();
    let prefixes = [
        "请记住",
        "帮我记住",
        "记住：",
        "记住:",
        "记住 ",
        "记一下：",
        "记一下:",
        "remember that ",
        "remember:",
    ];

    for prefix in prefixes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let value = rest
                .trim()
                .trim_start_matches(['：', ':', ',', '，', '。', ' ']);
            if value.chars().count() >= 2 {
                return Some(("用户指定记忆".to_string(), compact_text(value, 800)));
            }
        }
    }

    None
}

fn should_include_memory_context(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("记忆库")
        || lower.contains("长期记忆")
        || lower.contains("保存的记忆")
        || lower.contains("你记得")
        || lower.contains("你还记得")
        || lower.contains("recall")
        || lower.contains("memory")
        || lower.contains("打开")
        || lower.contains("运行")
        || lower.contains("启动")
        || lower.contains("open")
        || lower.contains("run")
        || lower.contains("start")
}

fn build_memory_context(memories: &[storage::MemoryItem]) -> Option<String> {
    if memories.is_empty() {
        return Some("长期记忆库当前没有保存内容。".to_string());
    }

    let mut lines = Vec::new();
    let mut total_chars = 0usize;
    for memory in memories.iter().take(20) {
        let line = format!(
            "- [{}] {}: {}",
            memory.category,
            compact_text(&memory.key, 80),
            compact_text(&memory.value, 240)
        );
        total_chars += line.chars().count();
        if total_chars > 4000 {
            break;
        }
        lines.push(line);
    }

    Some(lines.join("\n"))
}

fn attach_memory_context(system_prompt: String, memory_context: Option<String>) -> String {
    let base = format!(
        "{}\n\n长期记忆能力：系统有一个由用户管理的长期记忆库。不要在每次回复中主动读取或展开记忆；当用户明确要求记住内容、查看记忆或基于旧信息回答时，再使用长期记忆相关能力。",
        system_prompt
    );

    match memory_context {
        Some(context) => format!("{}\n\n本次按用户请求读取到的长期记忆：\n{}", base, context),
        None => base,
    }
}

async fn attach_registered_apps_prompt(
    system_prompt: String,
    state: &tauri::State<'_, AppState>,
) -> String {
    let db = state.db.lock().await;
    if let Ok(memories) = db.get_memories_by_category("app_path") {
        if !memories.is_empty() {
            let mut app_notes = vec![
                "\n\n【重要系统级信息】用户已经在系统中注册并保存了以下自定义应用程序（快捷方式路径）记忆：".to_string()
            ];
            for app in memories {
                app_notes.push(format!(
                    "- 应用别名/名称: \"{}\", 对应执行文件物理路径: \"{}\"",
                    app.key, app.value
                ));
            }
            app_notes.push("当用户要求你“打开”、“运行”、“启动”这些应用时，你拥有完整的预知记忆。请直接调用 `open_app` 工具，并以其对应的应用别名/名称（例如 \"微信\"、\"Chrome\" 等）作为 `app` 参数！底层已实现了对注册键名的自动匹配和物理路径运行，你不需要向用户询问其物理路径。".to_string());
            return format!("{}{}", system_prompt, app_notes.join("\n"));
        }
    }
    system_prompt
}

async fn memory_context_for_message(
    state: &tauri::State<'_, AppState>,
    message: &str,
) -> Option<String> {
    if !should_include_memory_context(message) {
        return None;
    }

    let db = state.db.lock().await;
    db.get_all_memories()
        .ok()
        .and_then(|memories| build_memory_context(&memories))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_message(role: &str, content: &str, created_at: i64) -> storage::ChatMessage {
        storage::ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            thinking: None,
            created_at,
        }
    }

    #[test]
    fn summarizes_chat_messages_into_memory_text() {
        let messages = vec![
            chat_message("user", "我想给桌宠增加长期记忆。", 10),
            chat_message("assistant", "可以，先拆成存储、命令和界面。", 11),
            chat_message("user", "还要支持直连 API 模型切换。", 12),
        ];

        let (key, value) = summarize_chat_messages(&messages).expect("summary");

        assert_eq!(key, "对话摘要 12");
        assert!(value.contains("共 3 条消息"));
        assert!(value.contains("长期记忆"));
        assert!(value.contains("直连 API"));
    }

    #[test]
    fn ignores_tiny_or_empty_conversations() {
        let messages = vec![chat_message("user", "   ", 10)];
        assert!(summarize_chat_messages(&messages).is_none());
    }

    #[test]
    fn detects_manual_memory_requests() {
        let (_, value) = parse_manual_memory_request("请记住：我喜欢简洁的界面").expect("memory");
        assert_eq!(value, "我喜欢简洁的界面");
        assert!(parse_manual_memory_request("普通聊天").is_none());
    }

    #[test]
    fn memory_context_is_only_used_when_requested() {
        assert!(should_include_memory_context("你还记得我的偏好吗？"));
        assert!(should_include_memory_context("please recall my memory"));
        assert!(!should_include_memory_context("帮我写一段说明"));
    }

    #[test]
    fn normalizes_api_profile_defaults() {
        let profile = normalize_api_profile(direct_api::DirectApiConfig {
            id: "work".to_string(),
            name: "".to_string(),
            api_key: " key ".to_string(),
            base_url: "https://api.example.com/v1/".to_string(),
            model: "gpt-test".to_string(),
            confirm_enabled: true,
            thinking_depth: "HIGH".to_string(),
            execution_mode: "custom".to_string(),
            search_provider: "DuckDuckGo".to_string(),
            auto_approved_tools: vec!["write_file".to_string(), "write_file".to_string()],
        });

        assert_eq!(profile.id, "work");
        assert_eq!(profile.name, "gpt-test");
        assert_eq!(profile.api_key, "key");
        assert_eq!(profile.base_url, "https://api.example.com/v1");
        assert_eq!(profile.thinking_depth, "high");
        assert_eq!(profile.execution_mode, "custom");
        assert_eq!(profile.search_provider, "duckduckgo");
        assert_eq!(profile.auto_approved_tools, vec!["write_file"]);
    }

    #[test]
    fn runtime_identity_prompt_exposes_model_without_secrets() {
        let profile = direct_api::DirectApiConfig {
            id: "auto-code".to_string(),
            name: "Auto Code".to_string(),
            api_key: "sk-secret-value".to_string(),
            base_url: "https://api.secret.example/v1".to_string(),
            model: "auto-code-v1".to_string(),
            confirm_enabled: true,
            thinking_depth: "auto".to_string(),
            execution_mode: "normal".to_string(),
            search_provider: "bing".to_string(),
            auto_approved_tools: Vec::new(),
        };

        let prompt = attach_runtime_identity_prompt("base prompt".to_string(), &profile);

        assert!(prompt.contains("auto-code-v1"));
        assert!(prompt.contains("Auto Code"));
        assert!(!prompt.contains("sk-secret-value"));
        assert!(!prompt.contains("api.secret.example"));
    }
}

fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("taskkill");
        command.creation_flags(CREATE_NO_WINDOW);
        let _ = command
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

async fn stop_active_response(state: &tauri::State<'_, AppState>) {
    {
        let abort = state.abort_token.lock().await;
        if let Some(ref token) = *abort {
            token.cancel();
        }
    }

    let pid = {
        let mut active = state.active_ai_pid.lock().await;
        active.take()
    };
    if let Some(pid) = pid {
        kill_process_tree(pid);
    }

    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
}

async fn reset_conversation(
    state: &tauri::State<'_, AppState>,
    reset_cli_session: bool,
) -> Result<Option<i64>, String> {
    stop_active_response(state).await;

    let saved_memory_id = {
        let db = state.db.lock().await;
        let saved_memory_id = save_chat_summary_if_needed(&db)?;
        db.clear_messages().map_err(|e| e.to_string())?;
        // 持久化 chat_start_id 到数据库，清空对话时重置为 0
        let _ = db.save_setting("chat_start_id", "0");
        saved_memory_id
    };

    // 清空对话后重置对话起点
    if let Ok(mut start_id) = state.chat_start_id.lock() {
        *start_id = 0;
    }

    state.approved_tool_types.lock().await.clear();

    if reset_cli_session {
        let ai = state.ai.lock().await;
        ai.reset_session();
    }

    let mut behavior = state.behavior.lock().await;
    behavior.set_state(behavior::PetState::Idle);

    Ok(saved_memory_id)
}

#[tauri::command]
async fn send_to_ai(
    message: String,
    attachments: Option<Vec<direct_api::UserAttachment>>,
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
        if let Some((key, value)) = parse_manual_memory_request(&message) {
            db.save_memory(MEMORY_CATEGORY_MANUAL, &key, &value)
                .map_err(|e| e.to_string())?;
        }
    }

    // 用户交互 + 切换到思考状态
    {
        let mut behavior = state.behavior.lock().await;
        behavior.on_user_interaction();
        behavior.set_state(behavior::PetState::Thinking);
    }

    let _ = app_handle.emit("ai-started", &message);
    tauri::async_runtime::spawn(run_ai_message(
        app_handle,
        message,
        attachments.unwrap_or_default(),
    ));

    Ok(serde_json::json!({ "started": true }))
}

async fn run_ai_message(
    app_handle: tauri::AppHandle,
    message: String,
    attachments: Vec<direct_api::UserAttachment>,
) {
    let state = app_handle.state::<AppState>();
    let backend = state.backend_type.lock().await.clone();

    if backend == "direct_api" {
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

        let system_prompt = {
            let personality = state.personality.lock().await.clone();
            let profession = state.profession.lock().await.clone();
            let base = openclaw::build_system_prompt(&personality, &profession);
            let base = attach_runtime_identity_prompt(base, &config);
            let base = attach_execution_mode_prompt(base, &config);
            let memory_context = memory_context_for_message(&state, &message).await;
            let base = attach_memory_context(base, memory_context);
            attach_registered_apps_prompt(base, &state).await
        };

        let chat_history = {
            let start_id = *state
                .chat_start_id
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let db = state.db.lock().await;
            let mut history = db
                .get_recent_messages_after(start_id, 10)
                .unwrap_or_default();
            if history
                .last()
                .map(|msg| msg.role == "user" && msg.content == message)
                .unwrap_or(false)
            {
                history.pop();
            }
            history
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
            attachments,
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
        let personality = state.personality.lock().await.clone();
        let profession = state.profession.lock().await.clone();
        let base = openclaw::build_system_prompt(&personality, &profession);
        let memory_context = memory_context_for_message(&state, &message).await;
        let base = attach_memory_context(base, memory_context);
        attach_registered_apps_prompt(base, &state).await
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
    stop_active_response(&state).await;
    let mut behavior = state.behavior.lock().await;
    behavior.set_state(behavior::PetState::Idle);
    Ok(())
}

#[tauri::command]
async fn set_backend_type(
    backend: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    stop_active_response(&state).await;

    {
        let mut backend_type = state.backend_type.lock().await;
        *backend_type = backend.clone();
    }
    let db = state.db.lock().await;
    db.save_setting("backend_type", &backend)
        .map_err(|e| e.to_string())?;

    state.approved_tool_types.lock().await.clear();
    Ok(())
}

#[tauri::command]
async fn get_backend_type(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let backend_type = state.backend_type.lock().await;
    Ok(backend_type.clone())
}

fn resolve_masked_api_key(db: &storage::Database, api_key: &str) -> String {
    if is_masked_api_key(api_key) {
        let profiles = load_api_profiles(db);
        for profile in profiles {
            if mask_api_key(&profile.api_key) == api_key {
                return profile.api_key;
            }
        }
        active_api_profile(db).api_key
    } else {
        api_key.to_string()
    }
}

#[tauri::command]
async fn set_api_config(
    api_key: String,
    base_url: String,
    model: String,
    confirm_enabled: bool,
    thinking_depth: Option<String>,
    execution_mode: Option<String>,
    search_provider: Option<String>,
    auto_approved_tools: Option<Vec<String>>,
    profile_id: Option<String>,
    profile_name: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<direct_api::DirectApiConfig, String> {
    let db = state.db.lock().await;
    let resolved_profile_id = match profile_id {
        Some(id) => id,
        None => db
            .get_setting(ACTIVE_API_PROFILE_KEY)
            .ok()
            .flatten()
            .unwrap_or_default(),
    };
    let existing_profile = if resolved_profile_id.is_empty() {
        active_api_profile(&db)
    } else {
        load_api_profiles(&db)
            .into_iter()
            .find(|profile| profile.id == resolved_profile_id)
            .unwrap_or_else(|| active_api_profile(&db))
    };

    let final_api_key = if api_key.trim().is_empty() {
        existing_profile.api_key.clone()
    } else if is_masked_api_key(&api_key) {
        existing_profile.api_key.clone()
    } else {
        api_key
    };

    let profile = upsert_api_profile(
        &db,
        direct_api::DirectApiConfig {
            id: resolved_profile_id,
            name: profile_name.unwrap_or_else(|| model.clone()),
            api_key: final_api_key,
            base_url,
            model,
            confirm_enabled,
            thinking_depth: thinking_depth.unwrap_or_else(|| "auto".to_string()),
            execution_mode: execution_mode
                .unwrap_or_else(|| existing_profile.execution_mode.clone()),
            search_provider: search_provider
                .unwrap_or_else(|| existing_profile.search_provider.clone()),
            auto_approved_tools: auto_approved_tools
                .unwrap_or_else(|| existing_profile.auto_approved_tools.clone()),
        },
        true,
    )?;

    let mut config = state.direct_api_config.lock().await;
    *config = Some(profile.clone());

    let mut returned_profile = profile;
    returned_profile.api_key = mask_api_key(&returned_profile.api_key);
    Ok(returned_profile)
}

#[tauri::command]
async fn get_api_config(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let profile = active_api_profile(&db);

    Ok(serde_json::json!({
        "id": profile.id,
        "name": profile.name,
        "api_key": "",
        "api_key_mask": mask_api_key(&profile.api_key),
        "has_api_key": !profile.api_key.trim().is_empty(),
        "base_url": profile.base_url,
        "model": profile.model,
        "confirm_enabled": profile.confirm_enabled,
        "thinking_depth": profile.thinking_depth,
        "execution_mode": profile.execution_mode,
        "search_provider": profile.search_provider,
        "auto_approved_tools": profile.auto_approved_tools,
    }))
}

#[tauri::command]
async fn list_api_profiles(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let db = state.db.lock().await;
    let active_id = db.get_setting(ACTIVE_API_PROFILE_KEY).ok().flatten();
    let profiles = load_api_profiles(&db);
    let public_profiles = profiles.iter().map(public_api_profile).collect::<Vec<_>>();
    Ok(serde_json::json!({
        "active_id": active_id.unwrap_or_else(|| profiles.first().map(|profile| profile.id.clone()).unwrap_or_default()),
        "profiles": public_profiles,
    }))
}

#[tauri::command]
async fn set_active_api_profile(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<direct_api::DirectApiConfig, String> {
    let db = state.db.lock().await;
    let profiles = load_api_profiles(&db);
    let profile = profiles
        .iter()
        .find(|profile| profile.id == id)
        .cloned()
        .ok_or_else(|| "未找到指定的 API 模型配置".to_string())?;

    db.save_setting(ACTIVE_API_PROFILE_KEY, &profile.id)
        .map_err(|e| e.to_string())?;
    save_legacy_api_settings(&db, &profile)?;

    let mut config = state.direct_api_config.lock().await;
    *config = Some(profile.clone());

    let mut returned_profile = profile;
    returned_profile.api_key = mask_api_key(&returned_profile.api_key);
    Ok(returned_profile)
}

#[tauri::command]
async fn delete_api_profile(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    let mut profiles = load_api_profiles(&db);
    if profiles.len() <= 1 {
        return Err("至少保留一个 API 模型配置".to_string());
    }

    let original_len = profiles.len();
    profiles.retain(|profile| profile.id != id);
    if profiles.len() == original_len {
        return Err("未找到指定的 API 模型配置".to_string());
    }

    let _ = system::credential::delete_api_key(&id);

    save_api_profiles(&db, &profiles)?;

    let active_id = db.get_setting(ACTIVE_API_PROFILE_KEY).ok().flatten();
    if active_id.as_deref() == Some(id.as_str()) {
        if let Some(next_profile) = profiles.first().cloned() {
            db.save_setting(ACTIVE_API_PROFILE_KEY, &next_profile.id)
                .map_err(|e| e.to_string())?;
            save_legacy_api_settings(&db, &next_profile)?;
            let mut config = state.direct_api_config.lock().await;
            *config = Some(next_profile);
        }
    }

    Ok(())
}

#[tauri::command]
async fn test_api_connection(
    api_key: String,
    base_url: String,
    model: String,
    thinking_depth: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let resolved_key = resolve_masked_api_key(&*state.db.lock().await, &api_key);
    let url = if base_url.ends_with("/chat/completions") {
        base_url.clone()
    } else {
        format!("{}/chat/completions", base_url.trim_end_matches('/'))
    };

    let normalized_depth =
        normalize_thinking_depth(&thinking_depth.unwrap_or_else(|| "auto".to_string()));
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "user", "content": "ping" }
        ],
        "max_tokens": 5
    });
    if matches!(normalized_depth.as_str(), "low" | "medium" | "high") {
        body.as_object_mut().unwrap().insert(
            "reasoning_effort".to_string(),
            serde_json::json!(normalized_depth),
        );
    }

    let mut req = state
        .http_client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body);
    if !resolved_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", resolved_key.trim()));
    }

    let res = tokio::time::timeout(std::time::Duration::from_secs(45), req.send())
        .await
        .map_err(|_| "连接测试超过 45 秒没有响应，请检查网络、Base URL 或代理配置。".to_string())?
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
async fn list_api_models(
    api_key: String,
    base_url: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let resolved_key = resolve_masked_api_key(&*state.db.lock().await, &api_key);
    direct_api::list_models(&state.http_client, &resolved_key, &base_url).await
}

#[tauri::command]
async fn confirm_tool(
    id: String,
    approved: bool,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut confirms = state.pending_confirms.lock().await;
    if let Some(tx) = confirms.remove(&id) {
        let _ = tx.send(approved);
        let _ = app_handle.emit(
            "ai-tool-confirm-resolved",
            serde_json::json!({ "id": id, "approved": approved }),
        );
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
async fn clear_chat_history(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let saved_memory_id = reset_conversation(&state, false).await?;
    Ok(serde_json::json!({ "saved_memory_id": saved_memory_id }))
}

#[tauri::command]
async fn start_new_conversation(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    stop_active_response(&state).await;

    let saved_memory_id = {
        let db = state.db.lock().await;
        let saved_memory_id = save_chat_summary_if_needed(&db)?;
        // 记录当前最大消息 ID 作为新对话起点，旧消息不再进入 AI 上下文
        let max_id = db.get_max_message_id().unwrap_or(0);
        if let Ok(mut start_id) = state.chat_start_id.lock() {
            *start_id = max_id;
        }
        // 持久化 chat_start_id 到数据库，确保重启后对话上下文不丢失
        let _ = db.save_setting("chat_start_id", &max_id.to_string());
        saved_memory_id
    };

    state.approved_tool_types.lock().await.clear();

    {
        let ai = state.ai.lock().await;
        ai.reset_session();
    }

    let mut behavior = state.behavior.lock().await;
    behavior.set_state(behavior::PetState::Idle);

    Ok(serde_json::json!({ "saved_memory_id": saved_memory_id }))
}

#[tauri::command]
async fn save_memory(
    category: String,
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<storage::MemoryItem, String> {
    let db = state.db.lock().await;
    let id = db
        .save_memory(&category, &key, &value)
        .map_err(|e| e.to_string())?;
    db.get_memory_by_id(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "记忆保存后读取失败".to_string())
}

#[tauri::command]
async fn get_memories(
    category: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<storage::MemoryItem>, String> {
    let db = state.db.lock().await;
    match category {
        Some(category) if !category.trim().is_empty() => db
            .get_memories_by_category(&category)
            .map_err(|e| e.to_string()),
        _ => db.get_all_memories().map_err(|e| e.to_string()),
    }
}

#[tauri::command]
async fn delete_memory(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_memory(id).map_err(|e| e.to_string())
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
    {
        let db = state.db.lock().await;
        let _ = save_chat_summary_if_needed(&db)?;
        db.clear_messages().map_err(|e| e.to_string())?;
    }
    state.approved_tool_types.lock().await.clear();
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

async fn get_shortcut_target_path(lnk_path: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "$sh = New-Object -ComObject WScript.Shell; \
             $target = $sh.CreateShortcut('{}').TargetPath; \
             Write-Output $target",
            lnk_path.replace('\'', "''")
        );

        let output = tokio::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .await
            .map_err(|e| format!("执行 PowerShell 失败: {e}"))?;

        if !output.status.success() {
            let err_text = String::from_utf8_lossy(&output.stderr);
            return Err(format!("PowerShell 错误: {}", err_text.trim()));
        }

        let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = lnk_path;
        Err("该功能仅在 Windows 系统中可用".to_string())
    }
}

#[tauri::command]
async fn register_shortcut_file(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let p = std::path::Path::new(&path);
    if !p.exists() || !p.is_file() {
        return Err("快捷方式文件不存在".to_string());
    }

    let app_name = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "未知应用".to_string());

    let extension = p
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if extension != "lnk" {
        return Err("仅支持 Windows 快捷方式 (.lnk) 文件".to_string());
    }

    let target_path = get_shortcut_target_path(&path).await?;
    if target_path.trim().is_empty() {
        return Err("解析快捷方式目标路径失败或目标路径为空".to_string());
    }

    let db = state.db.lock().await;
    db.save_memory("app_path", &app_name, &target_path)
        .map_err(|e| format!("保存到数据库失败: {e}"))?;

    Ok(serde_json::json!({
        "name": app_name,
        "path": target_path,
    }))
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
                bundled_node_dir
                    .join("node_modules")
                    .join(".bin")
                    .join("claude.cmd"),
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

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("无法读取 Claude 配置文件: {e}"))?;

    let json: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 Claude 配置文件失败: {e}"))?;

    let env = json.get("env");
    let token = env
        .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
        .and_then(|t| t.as_str());
    let model = env
        .and_then(|e| e.get("ANTHROPIC_MODEL"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string());
    let base_url = env
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

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
    let mut path = app.path().app_data_dir().unwrap_or_else(|_| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ai-desktop-pet")
    });
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

    let direct_config = Some(active_api_profile(&db));

    // 从数据库读取 chat_start_id，确保重启后对话上下文不丢失
    let chat_start_id = db
        .get_setting("chat_start_id")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let http_client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Failed to create HTTP client");

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
                approved_tool_types: tokio::sync::Mutex::new(std::collections::HashSet::new()),
                abort_token: tokio::sync::Mutex::new(None),
                chat_start_id: StdMutex::new(chat_start_id),
                http_client,
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
            start_new_conversation,
            save_memory,
            get_memories,
            delete_memory,
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
            register_shortcut_file,
            exit_app,
            open_claude_config,
            tts_synthesize,
            tts_list_voices,
            set_backend_type,
            get_backend_type,
            set_api_config,
            get_api_config,
            list_api_profiles,
            set_active_api_profile,
            delete_api_profile,
            test_api_connection,
            list_api_models,
            confirm_tool,
            check_claude_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
