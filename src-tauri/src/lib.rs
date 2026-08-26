mod behavior;
mod computer_use;
mod direct_api;
mod openclaw;
mod skills;
mod storage;
mod subprocess;
mod system;
mod tts;

use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    fs::OpenOptions,
    io::Write,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_avatar: Option<String>,
}

/// 本轮消息被 @ 提及的智能体运行时信息（传给 direct_api 用于保存归属与事件载荷）。
#[derive(Clone, Serialize)]
pub struct AgentContext {
    pub agents: Vec<storage::Agent>,
}

impl AgentContext {
    /// 单智能体时返回该智能体的归属信息，多智能体（面板模式）返回 None。
    pub fn single(&self) -> Option<&storage::Agent> {
        if self.agents.len() == 1 {
            self.agents.first()
        } else {
            None
        }
    }
}

#[derive(Clone, Serialize)]
struct AiErrorPayload {
    message: String,
    thinking: Option<String>,
    aborted: bool,
}

#[derive(Clone, Serialize)]
struct ScheduledTaskTriggeredPayload {
    task: storage::ScheduledTask,
    message: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveCustomPetAssetRequest {
    id: Option<String>,
    name: String,
    kind: Option<String>,
    manifest: String,
    sprite_data_url: String,
    preview_data_url: Option<String>,
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
    pub tts_playback: tts::playback::NativeAudioPlayer,
    pub backend_type: tokio::sync::Mutex<String>,
    pub direct_api_config: tokio::sync::Mutex<Option<direct_api::DirectApiConfig>>,
    pub pending_confirms:
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
    pub pending_questions:
        tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    pub approved_tool_types: tokio::sync::Mutex<std::collections::HashSet<String>>,
    pub abort_token: tokio::sync::Mutex<Option<tokio_util::sync::CancellationToken>>,
    pub chat_start_id: StdMutex<i64>,
    pub http_client: reqwest::Client,
    pub weather_cache: tokio::sync::Mutex<Option<system::WeatherInfo>>,
    pub last_active_skill_id: tokio::sync::Mutex<Option<String>>,
    pub subprocs: subprocess::SubprocessManager,
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
const CUSTOM_PIXEL_PET_SETTING_KEY: &str = "custom_pixel_pet_asset_id";
const MEMORY_CATEGORY_CONVERSATION: &str = "conversation";
const WEATHER_CONFIG_KEY: &str = "weather_config";
const WEATHER_SENT_DATE_KEY: &str = "weather_sent_date";
const WEATHER_CACHE_TTL_SECS: i64 = 600;
const SCHEDULE_REPEAT_ONCE: &str = "once";
const SCHEDULE_REPEAT_DAILY: &str = "daily";
const SCHEDULE_REPEAT_WEEKLY: &str = "weekly";
const SCHEDULE_REPEAT_MONTHLY: &str = "monthly";
const SCHEDULE_MIN_DUE_OFFSET_SECS: i64 = 5;
const SECS_PER_DAY: i64 = 86_400;

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

fn normalize_task_repeat(value: &str) -> storage::TaskRepeat {
    storage::TaskRepeat::from_str(value)
}

fn validate_schedule_input(title: &str, due_at: i64) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("任务标题不能为空".to_string());
    }
    if title.chars().count() > 120 {
        return Err("任务标题不能超过 120 个字符".to_string());
    }
    if due_at < unix_now() + SCHEDULE_MIN_DUE_OFFSET_SECS {
        return Err("提醒时间必须晚于当前时间至少 5 秒".to_string());
    }
    Ok(())
}

fn next_due_at_for_repeat(repeat: &str, previous_due_at: i64, now: i64) -> Option<i64> {
    let interval = match repeat {
        SCHEDULE_REPEAT_DAILY => SECS_PER_DAY,
        SCHEDULE_REPEAT_WEEKLY => SECS_PER_DAY * 7,
        SCHEDULE_REPEAT_MONTHLY => SECS_PER_DAY * 30,
        _ => return None,
    };

    let mut next = previous_due_at.saturating_add(interval);
    while next <= now {
        next = next.saturating_add(interval);
    }
    Some(next)
}

fn scheduled_task_message(task: &storage::ScheduledTask) -> String {
    let note = task.note.trim();
    if note.is_empty() {
        format!("定时任务提醒：{}", task.title)
    } else {
        format!("定时任务提醒：{}\n{}", task.title, note)
    }
}

const MAX_CUSTOM_PET_PNG_BYTES: usize = 4 * 1024 * 1024;
const MAX_CUSTOM_PET_MANIFEST_BYTES: usize = 64 * 1024;

fn is_valid_custom_pet_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn app_data_root(app_handle: &tauri::AppHandle) -> PathBuf {
    app_handle.path().app_data_dir().unwrap_or_else(|_| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ai-desktop-pet")
    })
}

fn weather_debug_log_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("ai-desktop-pet");
    path.push("weather-debug.log");
    path
}

fn log_weather_debug(kind: &str, message: &str) {
    let path = weather_debug_log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{timestamp}] [{kind}] {message}\n");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn weather_proxy_env_summary() -> String {
    let keys = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ];

    keys.iter()
        .map(|key| {
            let value = std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty());
            match value {
                Some(value) => format!("{key}={value}"),
                None => format!("{key}=<unset>"),
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn build_weather_proxy_client() -> Option<reqwest::Client> {
    // 仅在用户显式设置了代理环境变量时才启用代理重试，
    // 不再硬编码 127.0.0.1:7890 兜底——那会在没有 Clash 的机器上
    // 凭空引入 15s 的 connect_timeout 延迟和误导性的「显式代理重试也失败」错误。
    let proxy_url = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("http_proxy"))
        .ok()
        .filter(|value| !value.trim().is_empty())?;

    let proxy = match reqwest::Proxy::all(&proxy_url) {
        Ok(proxy) => proxy,
        Err(error) => {
            log_weather_debug(
                "weather_proxy_client_err",
                &format!("invalid_proxy={proxy_url} error={error}"),
            );
            return None;
        }
    };

    match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .tcp_keepalive(std::time::Duration::from_secs(60))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .proxy(proxy)
        .build()
    {
        Ok(client) => {
            log_weather_debug("weather_proxy_client", &format!("proxy={proxy_url}"));
            Some(client)
        }
        Err(error) => {
            log_weather_debug("weather_proxy_client_err", &format!("error={error}"));
            None
        }
    }
}

async fn get_weather_with_proxy_retry(
    default_client: &reqwest::Client,
    config: &system::weather::WeatherConfig,
) -> Result<system::WeatherInfo, String> {
    match system::weather::get_weather_with_client(default_client, config).await {
        Ok(weather) => Ok(weather),
        Err(primary_error) => {
            log_weather_debug("weather_default_client_err", &primary_error);
            let Some(proxy_client) = build_weather_proxy_client() else {
                return Err(primary_error);
            };

            match system::weather::get_weather_with_client(&proxy_client, config).await {
                Ok(weather) => {
                    log_weather_debug(
                        "weather_proxy_retry_ok",
                        &format!("city={} temp={}", weather.city, weather.temperature),
                    );
                    Ok(weather)
                }
                Err(proxy_error) => {
                    log_weather_debug("weather_proxy_retry_err", &proxy_error);
                    Err(format!(
                        "{}\n显式代理重试也失败: {}",
                        primary_error, proxy_error
                    ))
                }
            }
        }
    }
}

fn custom_pet_assets_root(app_handle: &tauri::AppHandle) -> PathBuf {
    app_data_root(app_handle).join("custom_pets")
}

fn custom_pet_asset_dir(app_handle: &tauri::AppHandle, id: &str) -> Result<PathBuf, String> {
    if !is_valid_custom_pet_id(id) {
        return Err("Invalid custom pet asset id".to_string());
    }
    Ok(custom_pet_assets_root(app_handle).join(id))
}

fn normalize_custom_pet_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "Custom Pixel Pet".to_string();
    }
    trimmed.chars().take(64).collect()
}

fn decode_png_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;

    let (header, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| "Invalid PNG data URL".to_string())?;
    let header = header.to_ascii_lowercase();
    if !header.starts_with("data:image/png") || !header.contains(";base64") {
        return Err("Only PNG data URLs are supported".to_string());
    }
    if encoded.len() > MAX_CUSTOM_PET_PNG_BYTES * 2 {
        return Err("Custom pet image is too large".to_string());
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CUSTOM_PET_PNG_BYTES {
        return Err("Custom pet image is too large".to_string());
    }
    // 校验 PNG 魔数 (\x89PNG\r\n\x1a\n)，避免任意字节以 .png 后缀写入
    if bytes.len() < 8 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("Invalid PNG signature".to_string());
    }
    Ok(bytes)
}

fn validate_pixel_pet_manifest(manifest: &str) -> Result<(), String> {
    if manifest.len() > MAX_CUSTOM_PET_MANIFEST_BYTES {
        return Err("Custom pet manifest is too large".to_string());
    }
    let value: serde_json::Value = serde_json::from_str(manifest).map_err(|e| e.to_string())?;
    if value.get("renderer").and_then(|item| item.as_str()) != Some("pixel-sprite") {
        return Err("Custom pet manifest renderer must be pixel-sprite".to_string());
    }
    let frame_size = value
        .get("frameSize")
        .ok_or_else(|| "Custom pet manifest missing frameSize".to_string())?;
    if frame_size
        .get("width")
        .and_then(|item| item.as_u64())
        .unwrap_or(0)
        == 0
        || frame_size
            .get("height")
            .and_then(|item| item.as_u64())
            .unwrap_or(0)
            == 0
    {
        return Err("Custom pet manifest has invalid frameSize".to_string());
    }
    let width = frame_size
        .get("width")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    let height = frame_size
        .get("height")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    if !matches!(width, 48 | 64 | 96) || width != height {
        return Err(
            "Custom pet manifest frameSize must be 48, 64, or 96 square pixels".to_string(),
        );
    }
    let sheet = value
        .get("sheet")
        .ok_or_else(|| "Custom pet manifest missing sheet".to_string())?;
    let columns = sheet
        .get("columns")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    let rows = sheet
        .get("rows")
        .and_then(|item| item.as_u64())
        .unwrap_or(0);
    if columns == 0 || columns > 20 || rows == 0 || rows > 20 {
        return Err("Custom pet manifest has invalid sheet size".to_string());
    }
    if value
        .get("animations")
        .and_then(|item| item.get("idle"))
        .is_none()
    {
        return Err("Custom pet manifest missing idle animation".to_string());
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ParsedScheduleRequest {
    title: String,
    note: String,
    due_at: i64,
    repeat: storage::TaskRepeat,
}

fn parse_schedule_request(message: &str, now: i64) -> Option<ParsedScheduleRequest> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_lowercase();
    let has_schedule_intent = ["提醒", "叫我", "定时", "闹钟", "remind", "timer", "alarm"]
        .iter()
        .any(|needle| lower.contains(needle));
    if !has_schedule_intent {
        return None;
    }

    let (due_at, matched_time) = parse_relative_due_at(trimmed, now)?;
    let repeat = if lower.contains("每天") || lower.contains("每日") || lower.contains("daily")
    {
        storage::TaskRepeat::Daily
    } else if lower.contains("每周") || lower.contains("weekly") {
        storage::TaskRepeat::Weekly
    } else if lower.contains("每月") || lower.contains("monthly") {
        storage::TaskRepeat::Monthly
    } else {
        storage::TaskRepeat::Once
    };

    let title = extract_schedule_title(trimmed, &matched_time);
    Some(ParsedScheduleRequest {
        title,
        note: trimmed.to_string(),
        due_at,
        repeat,
    })
}

fn parse_relative_due_at(message: &str, now: i64) -> Option<(i64, String)> {
    if let Some(index) = message.find("半小时后") {
        return Some((
            now.saturating_add(30 * 60),
            message[index..index + "半小时后".len()].to_string(),
        ));
    }

    let candidates = [
        ("分钟后", 60.0),
        ("分后", 60.0),
        ("小时后", 3600.0),
        ("个小时后", 3600.0),
        ("天后", SECS_PER_DAY as f64),
    ];

    for (marker, seconds) in candidates {
        if let Some((value, matched)) = number_before_marker(message, marker) {
            if value > 0.0 && value.is_finite() {
                let offset = (value * seconds).round() as i64;
                return Some((now.saturating_add(offset), matched));
            }
        }
    }

    None
}

fn number_before_marker(message: &str, marker: &str) -> Option<(f64, String)> {
    let marker_start = message.find(marker)?;
    let before = &message[..marker_start];
    let mut number_start = before.len();
    for (idx, ch) in before.char_indices().rev() {
        if ch.is_ascii_digit() || ch == '.' || ch.is_whitespace() {
            number_start = idx;
        } else {
            break;
        }
    }

    let raw_number = before[number_start..].trim();
    if !raw_number.is_empty() {
        let value = raw_number.parse::<f64>().ok()?;
        let matched = message[number_start..marker_start + marker.len()]
            .trim()
            .to_string();
        return Some((value, matched));
    }

    let mut chinese_start = before.len();
    for (idx, ch) in before.char_indices().rev() {
        if is_chinese_number_char(ch) {
            chinese_start = idx;
        } else if idx < chinese_start {
            break;
        }
    }
    let raw_chinese = before[chinese_start..].trim();
    let value = parse_simple_chinese_number(raw_chinese)?;
    let matched = message[chinese_start..marker_start + marker.len()]
        .trim()
        .to_string();
    Some((value, matched))
}

fn is_chinese_number_char(ch: char) -> bool {
    matches!(
        ch,
        '零' | '一' | '二' | '两' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百'
    )
}

fn chinese_digit_value(ch: char) -> Option<i64> {
    match ch {
        '零' => Some(0),
        '一' => Some(1),
        '二' | '两' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

fn parse_simple_chinese_number(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some((head, tail)) = value.split_once('百') {
        let hundreds = if head.is_empty() {
            1
        } else {
            parse_simple_chinese_number(head)? as i64
        };
        let rest = if tail.is_empty() {
            0
        } else {
            parse_simple_chinese_number(tail)? as i64
        };
        return Some((hundreds * 100 + rest) as f64);
    }
    if let Some((head, tail)) = value.split_once('十') {
        let tens = if head.is_empty() {
            1
        } else {
            parse_simple_chinese_number(head)? as i64
        };
        let ones = if tail.is_empty() {
            0
        } else {
            parse_simple_chinese_number(tail)? as i64
        };
        return Some((tens * 10 + ones) as f64);
    }

    let mut total = 0i64;
    for ch in value.chars() {
        total = total * 10 + chinese_digit_value(ch)?;
    }
    Some(total as f64)
}

fn extract_schedule_title(message: &str, matched_time: &str) -> String {
    let mut title = message.replace(matched_time, " ");
    for phrase in [
        "请",
        "帮我",
        "给我",
        "设置",
        "创建",
        "一个",
        "定时任务",
        "定时提醒",
        "提醒我",
        "提醒",
        "叫我",
        "闹钟",
        "每天",
        "每日",
        "每周",
        "每月",
        "remind me to",
        "remind me",
        "timer",
        "alarm",
    ] {
        title = title.replace(phrase, " ");
    }

    let title = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['：', ':', ',', '，', '。', '.', '；', ';', ' ', '\n', '\t'])
        .to_string();

    if title.is_empty() {
        "提醒".to_string()
    } else {
        compact_text(&title, 80)
    }
}

fn schedule_delay_label(due_at: i64, now: i64) -> String {
    let delta = due_at.saturating_sub(now);
    if delta < 3600 {
        format!("{} 分钟后", (delta + 59) / 60)
    } else if delta < SECS_PER_DAY {
        format!("{} 小时后", (delta + 3599) / 3600)
    } else {
        format!("{} 天后", (delta + SECS_PER_DAY - 1) / SECS_PER_DAY)
    }
}

fn task_repeat_label(repeat: storage::TaskRepeat) -> &'static str {
    match repeat {
        storage::TaskRepeat::Once => "仅一次",
        storage::TaskRepeat::Daily => "每天",
        storage::TaskRepeat::Weekly => "每周",
        storage::TaskRepeat::Monthly => "每月",
    }
}

fn schedule_confirmation_text(task: &storage::ScheduledTask, now: i64) -> String {
    format!(
        "已设置定时任务「{}」，{}提醒你。重复规则：{}。",
        task.title,
        schedule_delay_label(task.due_at, now),
        task_repeat_label(storage::TaskRepeat::from_str(&task.repeat))
    )
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
        "stream_mode": profile.stream_mode.clone(),
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
    profile.stream_mode = direct_api::api_client::normalize_stream_mode(&profile.stream_mode);
    profile.api_mode = direct_api::api_client::normalize_api_mode(&profile.api_mode);

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
            stream_mode: "auto".to_string(),
            api_mode: "chat_completions".to_string(),
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
            // keyring 失败时拒绝落明文到数据库：日志记录失败，
            // 本会话（调用方持有的 profiles 副本）仍含真 key 可继续工作，
            // 但重启后该 profile 需要用户重新输入。
            if let Err(e) = system::credential::set_api_key(&profile.id, &profile.api_key) {
                eprintln!(
                    "[credential] 写入密钥库失败，拒绝明文持久化 (profile {}): {e}",
                    profile.id
                );
            }
        }
        // 任何情况下都不在 DB 中保存可解出的明文 key
        profile.api_key = String::new();
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

fn attach_computer_use_prompt(system_prompt: String) -> String {
    let note = "Computer Use tools are available only when the user enables them in settings. When using them, inspect the current UI with computer_screenshot or browser_snapshot before the first mouse/keyboard action when the UI state matters. For browser tasks prefer the browser_* family: browser_open starts the AI-managed browser, browser_navigate/browser_back/browser_forward move between pages, browser_extract reads page text and links, browser_click/browser_type/browser_press interact with elements, browser_scroll moves the page, and browser_snapshot captures the page viewport. Use browser_* instead of desktop computer_mouse/computer_keyboard whenever the target is inside a browser tab. Continue the task until it is actually complete: after launching an app, wait/focus/inspect as needed and proceed with the next requested step. Batch safe low-risk actions such as typing a known text string, pressing Enter after typing, or using a common hotkey; do not screenshot after every keystroke or every tiny mouse movement. Wait and inspect again after actions that materially change the UI, when target focus is uncertain, or before irreversible actions. Never type passwords, verification codes, API keys, tokens, payment data, or other secrets. If the user says to continue after an interruption, use the saved tool progress in the conversation and take a fresh screenshot before deciding the next UI action. If the user has not clearly asked for desktop/browser control, explain what you need before requesting an action.";
    format!("{}\n\n{}", system_prompt, note)
}

fn attach_weather_prompt(system_prompt: String) -> String {
    let note = "天气能力：当用户询问当前天气、温度、湿度、出门建议，或要求查询某个城市天气时，优先使用 get_weather 工具。该工具会读取设置页保存的天气配置，也可以临时传入 location 查询指定城市。";
    format!("{}\n\n{}", system_prompt, note)
}

fn attach_search_prompt(system_prompt: String) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let note = format!(
        "联网搜索能力：当前日期是 {today}。当用户询问新闻、日报、最近、最新、今天、本周、价格、法规、版本、人物职位或其他可能随时间变化的信息时，必须先把时间范围想清楚再搜索。\
        对新闻、日报、快讯、最近动态这类任务，优先调用 web_search 并传入 recency_days（今天/日报用 1-2，最近/本周用 7，本月用 31）；搜索词要包含关键实体、地区、主题和日期线索。\
        阅读网页时优先调用 read_webpage 并传入 query，只读取与问题相关的片段。整理结果时比较来源的发布时间，丢弃明显过旧或与时间范围不符的内容；如果搜索结果没有发布时间，要说明不确定性并继续找更可靠来源。\
        做每日时报或新闻简报时，先用 recency_days 搜索最近新闻，再按重要性、地区/主题多样性和来源可靠性筛选，不要把旧新闻当成今日新闻。"
    );
    format!("{}\n\n{}", system_prompt, note)
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

fn attach_memory_prompt(system_prompt: String) -> String {
    format!(
        "{}\n\n长期记忆能力：你拥有 search_memory、save_memory 和 delete_memory 工具来管理长期记忆。\
        当对话中出现需要跨会话记住的信息（用户偏好、习惯、事实等），主动使用 save_memory 保存。\
        当用户询问你是否记得某些事、或你需要回忆之前保存的信息时，使用 search_memory 搜索。\
        不要过度记忆，只保存有价值的、长期有用的信息。",
        system_prompt
    )
}

async fn attach_registered_apps_prompt(
    system_prompt: String,
    state: &tauri::State<'_, AppState>,
) -> String {
    let db = state.db.lock().await;
    if let Ok(memories) = db.get_memories_by_category("app_path") {
        if !memories.is_empty() {
            let mut app_notes = vec![
                "\n\n【重要系统级信息】用户已经在系统中注册并保存了以下自定义应用程序（快捷方式路径）记忆。".to_string()
            ];
            for app in memories {
                app_notes.push(format!(
                    "- 应用别名/名称: \"{}\", 对应执行文件物理路径: \"{}\"",
                    app.key, app.value
                ));
            }
            app_notes.push("当用户要求你“打开”、“运行”、“启动”这些应用时，你拥有完整的预知记忆。请直接调用 `open_app` 工具，并以其对应的应用别名/名称（例如\"微信\"、\"Chrome\" 等）作为 `app` 参数！底层已实现了对注册键名的自动匹配和物理路径运行，你不需要向用户询问其物理路径。".to_string());
            return format!("{}{}", system_prompt, app_notes.join("\n"));
        }
    }
    system_prompt
}

fn attach_custom_prompt(system_prompt: String, custom: &str) -> String {
    if custom.trim().is_empty() {
        return system_prompt;
    }
    format!(
        "{}\n\n【用户自定义系统提示】\n{}",
        system_prompt,
        custom.trim()
    )
}

/// 告知 AI 引用回复机制：用户消息可能带 [引用回复] 前缀；AI 也可用 [QUOTE] 标记引用前文
fn attach_quote_prompt(system_prompt: String) -> String {
    format!(
        "{}\n\n引用回复能力：用户消息若带有「[引用回复]」标记，表示用户引用了之前对话中的某条消息并针对性回复，\
        请优先围绕被引用的内容作答。当你（助手）自己需要明确引用之前对话中的某段原文时，\
        使用引用标记语法：把要引用的原文放在 [QUOTE] 和 [/QUOTE] 之间，例如：\n\
        [QUOTE]用户之前说过的话[/QUOTE]\n\
        针对上面引用的内容，我的回应是……\n\
        前端会把标记内的内容渲染成引用块。注意：只引用对话中真实出现过的原文，不要编造；\
        引用标记之外的部分正常书写；不需要引用时完全不使用该标记。",
        system_prompt
    )
}

/// 告知 AI 文档生成能力与主动询问原则（direct_api 后端专用）
fn attach_word_and_ask_prompt(system_prompt: String) -> String {
    format!(
        "{}

【Word 文档生成能力】
你可以使用 create_docx 工具把内容写到 .docx 文件（Word 文档）。当用户要求生成 Word 文档、\
导出报告、写论文/方案并保存为文档时，优先用 create_docx 而不用 write_file 写纯文本。用法：
- content 参数是 Markdown 内容，支持：# ## ### 多级标题、段落、**粗体** *斜体* `行内代码`、   - 无序列表、 . 有序列表、  引用块、```代码块```、| 表格 | 语法。
- path 参数是保存位置；用户未指定路径时保存到桌面，文件名根据内容主题拟定。
- title 参数可选，作为文档首页大标题。
- 生成成功后在回复中告知用户完整保存路径。
- 若用户要求修改已有 .docx 的内容，先用 read_file 读取，确认需求后用 create_docx 重新生成覆盖。

【主动询问用户】
你有 ask_user 工具，可以在遇到不确定的事情或方向时主动向用户提问，弹窗会让用户选择你给出的选项或自行填写答案。适合使用的场景：
1. 用户的要求存在多种明显不同的理解方向（做 A 还是做 B）；
2. 缺少关键参数且无法从上下文推断（如保存路径、格式、受众、篇幅）；
3. 即将执行的 操作代价较高或不可逆（覆盖重要文件、大批量修改），需用户拍板；
4. 生成的文档内容涉及用户的个人偏好（文风、结构取舍）， 提问时把候选答案放到 options（每个 label 简短、description 说明含义），通常 2-4 个选项为宜。 原则：只在真正影响结果方向的事情上提问；琐碎小事（措辞、常见默认值）按最佳判断直接做，\
不要为小事频繁打断用户；纯闲聊和事实问答不要调用 ask_user。用户跳过或超时未答时，\
按你的最佳判断继续并在回复中说明你的假设。",
        system_prompt
    )
}

/// 告知 AI 代码工具用法与调试方法论（direct_api 后端专用）
fn attach_coding_prompt(system_prompt: String) -> String {
    format!(
        "{}

【编程与代码调试能力】
用户让你处理代码任务（修 bug、加功能、重构、解释项目）时，按下面的方法和工具工作：

【代码工具用法】
- code_search：按内容搜代码，返回 文件:行号:行内容。找函数定义、调用点、报错关键字、配置项都用它；配合 glob（如 \"*.rs;*.ts;*.vue\"）缩小范围。找文件名用 file_search，找内容用 code_search。
- read_file：读文件。代码文件用 offset/limit 分页读（返回带行号的行范围，单次最多 2000 行），大文件不要整读。
- edit_file：改代码首选。把要替换的原文（old_text）和改后内容（new_text）传入，只动这一处、其余内容保持不变。old_text 必须与文件实际内容完全一致（含缩进换行）且在文件中唯一；不唯一就多带几行上下文。新建文件或大范围重写才用 write_file。
- run_command：跑构建/测试/命令验证修改。cargo build、npm install、pytest 等耗时命令必须带 timeout_secs（300-600），默认 30 秒会超时中断。

【调试工作流】
1. 理解：先看项目结构（list_directory）和关键文件，不要凭猜测动手。读报错信息、日志、相关代码，找到问题真正发生的位置（用 code_search 从报错关键字/函数名定位）；
2. 定位修改点：用 read_file 分页读目标文件的相关行范围，确认要改的确切内容；
3. 最小修改：用 edit_file 只改必要的部分，不做无关重构、不顺手改格式。修改与任务无关的代码必须避免。
4. 验证：改完立即用 run_command 跑构建或测试（如 cargo check、npm test）。失败就读完整报错，回到第 2 步迭代——不要没验证就宣称完成，也不要在没读懂报错时盲目重复尝试。
【纪律】
- 修改前必须先读到原文：没读过原 edit_file 大概率匹配失败。
- 一次只改一个逻辑点，改完验证再继续，避免多改动叠加导致难以定位新问题。
- 修改属于不可逆或影响面大的操作（删文件、改配置、批量替换）而方向不明确时，先用 ask_user 跟用户确认。
- 向用户汇报时说明：改了哪些文件哪几处、如何验证的、结果如何。",
        system_prompt
    )
}

/// 把用户引用的前文对话拼装进发送给后端的用户消息（openclaw CLI 模式使用。
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

/// 把被 @ 提及的智能体人格注入系统提示。
/// - 单个智能体：AI 完全以该智能体的身份回复；
/// - 多个智能体：面板模式，AI 依次以每个智能体身份作答。
///   每个回复以「【智能体名】」标题行开头。
fn attach_agent_prompt(system_prompt: String, agents: &[storage::Agent]) -> String {
    if agents.is_empty() {
        return system_prompt;
    }

    if agents.len() == 1 {
        let agent = &agents[0];
        let description = if agent.description.trim().is_empty() {
            "（无描述）".to_string()
        } else {
            agent.description.trim().to_string()
        };
        let persona = if agent.system_prompt.trim().is_empty() {
            "沿用你的基础人格与能力回答。".to_string()
        } else {
            agent.system_prompt.trim().to_string()
        };
        return format!(
            "{}

【当前被 @ 的智能体】
名字：{}
头像符号：{}
描述：{}
角色设定：
{}
你现在就是这个智能体。请完全以「{}」的身份、语气和能力范围回复用户，不要提及你正在扮演智能体，也不要把整段回复用【】括起来。",
            system_prompt, agent.name, agent.avatar, description, persona, agent.name
        );
    }

    let mut catalog = String::new();
    for agent in agents {
        let description = if agent.description.trim().is_empty() {
            "（无描述）".to_string()
        } else {
            agent.description.trim().replace('\n', " ")
        };
        catalog.push_str(&format!("- {}：{}
", agent.name, description));
    }
    let mut personas = String::new();
    for agent in agents {
        let persona = if agent.system_prompt.trim().is_empty() {
            "以通用助手身份回答。".to_string()
        } else {
            agent.system_prompt.trim().to_string()
        };
        personas.push_str(&format!("
### {}
{}
", agent.name, persona));
    }

    format!(
        "{}

【用户 @ 了多个智能体】
用户本轮同时提到了以下智能体：
{catalog}
请依次扮演每个智能体给出各自的回复。每个回复的第一行必须是「【智能体名字】」，智能体名字要与上面列表完全一致；标题行之后换行输出该智能体的回复正文。各智能体按自己的设定独立作答，允许观点不同，不要相互复读、不要互相替对方说话。
各智能体角色设定：
{personas}",
        system_prompt
    )
}

async fn attach_skill_prompt(
    system_prompt: String,
    active: Option<&storage::Skill>,
    all_skills: &[storage::Skill],
    is_direct_api: bool,
) -> String {
    // 1) 把"技能清单（name + description） 注入系统提示，让 AI 知道有哪些可用技能，可以主动调用相关工具。
    //    清单只列名称和用途描述，不追加规则；具体规则由用户激活的那一个技能注入。
    let catalog = build_skill_catalog(all_skills);
    let system_prompt = if catalog.is_empty() {
        system_prompt
    } else {
        format!("{}\n\n{}", system_prompt, catalog)
    };

    // 2) 当前激活技能的追加系统提示。
    let Some(skill) = active else {
        return system_prompt;
    };
    if skill.system_prompt.trim().is_empty() {
        return system_prompt;
    }
    let description = if skill.description.trim().is_empty() {
        "（无描述）".to_string()
    } else {
        skill.description.trim().to_string()
    };

    // 工具白名单仅在 direct_api 后端下有效：openclaw 走的是 Claude CLI 子进程，
    // 不消费 DirectApiConfig，is_high_risk_tool / mode_requires_confirmation 这类
    // 免确认机制对它不起作用。在 openclaw 路径下告诉 AI "可免确认" 等于说谎。
    if is_direct_api {
        let tools = if skill.allowed_tools.is_empty() {
            "（无）".to_string()
        } else {
            skill.allowed_tools.join(", ")
        };
        format!(
            "{}\n\n【当前激活技能】{}\n描述：{}\n追加规则：\n{}\n允许免确认使用的工具：{}",
            system_prompt, skill.name, description, skill.system_prompt, tools
        )
    } else {
        format!(
            "{}\n\n【当前激活技能】{}\n描述：{}\n追加规则：\n{}\n注：当前为 Claude CLI 后端，技能工具白名单不生效，所有工具调用仍由 CLI 自身权限控制。",
            system_prompt, skill.name, description, skill.system_prompt
        )
    }
}

/// 生成"技能清单"段落，所有技能（不论是否激活）都向 AI 公开名称 + 描述，
/// 让 AI 在判断任务时知道哪些技能适用。格式严格，避免被注入：
/// - 每个技能用同一段固定模板
/// - 名称 / 描述按行输出（不解析 Markdown）
/// - 空描述 / 空名称直接跳过
fn build_skill_catalog(skills: &[storage::Skill]) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("【可用技能清单】以下是当前配置的技能名称与用途描述。AI 应根据用户需求自主判断是否需要调用相关技能对应的工具；只有被用户手动激活的技能会追加专属规则并把工具加入免确认白名单。".to_string());
    for skill in skills {
        let name = skill.name.trim();
        if name.is_empty() {
            continue;
        }
        let description = skill.description.trim();
        let description_part = if description.is_empty() {
            "（无描述）".to_string()
        } else {
            description.replace('\n', " ")
        };
        let status = if skill.is_active {
            "（已激活）"
        } else {
            ""
        };
        lines.push(format!("- {name}{status}：{description_part}"));
    }
    lines.join("\n")
}

/// 解析本轮消息应使用的技能：完全由用户手动激活决定。
/// 关键词不再参与匹配；`all_skills` 保留形参仅用于拼接"技能清单"。
fn resolve_skill_for_message(db_active: Option<&storage::Skill>) -> Option<storage::Skill> {
    db_active.cloned()
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
            quoted_role: None,
            quoted_content: None,
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
    fn parses_relative_schedule_requests() {
        let task = parse_schedule_request("10分钟后提醒我喝水", 1_000).expect("task");
        assert_eq!(task.title, "喝水");
        assert_eq!(task.due_at, 1_600);
        assert_eq!(task.repeat, storage::TaskRepeat::Once);

        let task = parse_schedule_request("每天十分钟后提醒我站起来", 1_000).expect("task");
        assert_eq!(task.title, "站起来");
        assert_eq!(task.due_at, 1_600);
        assert_eq!(task.repeat, storage::TaskRepeat::Daily);

        assert!(parse_schedule_request("帮我整理一下计划", 1_000).is_none());
    }

    #[test]
    fn schedule_validation_rejects_oversized_titles() {
        let title = "a".repeat(121);
        let error = validate_schedule_input(&title, unix_now() + 60).expect_err("long title");
        assert!(error.contains("120"));
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
            stream_mode: "auto".to_string(),
            api_mode: "chat_completions".to_string(),
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
            stream_mode: "auto".to_string(),
            api_mode: "chat_completions".to_string(),
        };

        let prompt = attach_runtime_identity_prompt("base prompt".to_string(), &profile);

        assert!(prompt.contains("auto-code-v1"));
        assert!(prompt.contains("Auto Code"));
        assert!(!prompt.contains("sk-secret-value"));
        assert!(!prompt.contains("api.secret.example"));
    }

    #[test]
    fn search_prompt_mentions_recency_and_dates() {
        let prompt = attach_search_prompt("base prompt".to_string());

        assert!(prompt.contains("recency_days"));
        assert!(prompt.contains("当前日期"));
        assert!(prompt.contains("发布时间"));
    }

    fn sample_skill(id: &str, name: &str) -> storage::Skill {
        storage::Skill {
            id: id.to_string(),
            name: name.to_string(),
            description: "测试技能".to_string(),
            system_prompt: "你是测试助手".to_string(),
            allowed_tools: vec!["read_file".to_string(), "search_memory".to_string()],
            keywords: vec![],
            is_active: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn skill_validator_accepts_well_formed_custom() {
        assert!(validate_skill_for_save(&sample_skill("custom-abc", "示例")).is_ok());
    }

    #[test]
    fn skill_validator_rejects_builtin_namespace_for_custom() {
        let mut s = sample_skill("builtin-fake-id", "假冒内置");
        let err = validate_skill_for_save(&s).unwrap_err();
        assert!(err.contains("保留"), "got: {err}");
        // 真实内置 id 应允许（用户可以编辑 builtin 的 prompt）
        s.id = "builtin-translation-polish".to_string();
        assert!(validate_skill_for_save(&s).is_ok());
    }

    #[test]
    fn skill_validator_rejects_oversized_prompt_and_invalid_tools() {
        let mut s = sample_skill("custom-big", "巨型");
        s.system_prompt = "a".repeat(4_001);
        let err = validate_skill_for_save(&s).unwrap_err();
        assert!(err.contains("system_prompt"), "got: {err}");

        let mut s2 = sample_skill("custom-badtool", "非法工具");
        s2.allowed_tools = vec!["read file".to_string()];
        let err = validate_skill_for_save(&s2).unwrap_err();
        assert!(err.contains("工具名"), "got: {err}");

        let mut s3 = sample_skill("custom-badid", "非法 id");
        s3.id = "has space".to_string();
        assert!(validate_skill_for_save(&s3).is_err());
    }

    #[test]
    fn reset_builtin_skills_restores_tombstoned_builtin() {
        let db = storage::Database::open_in_memory().expect("open in-memory db");
        db.delete_skill("builtin-translation-polish").expect("delete");
        assert!(db
            .list_skills()
            .unwrap()
            .iter()
            .all(|s| s.id != "builtin-translation-polish"));

        db.reset_builtin_skills().expect("reset");
        let after = db.list_skills().unwrap();
        assert!(after.iter().any(|s| s.id == "builtin-translation-polish"),
            "reset_builtin_skills 应让被墓碑封存的内置技能回来");
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
    quoted_role: Option<String>,
    quoted_content: Option<String>,
    mention_agents: Option<Vec<String>>,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    if message.trim().is_empty() && quoted_content.as_deref().unwrap_or("").trim().is_empty() {
        return Err("message is empty".to_string());
    }

    {
        let mut active = state
            .active_chat
            .lock()
            .map_err(|_| "active chat state poisoned".to_string())?;
        if active.active.is_some() {
            return Err("AI 正在思考中，请稍等当前回复完成后".to_string());
        }
        active.active = Some(ActiveChatSnapshot {
            message: message.clone(),
            thinking: String::new(),
            started_at: unix_now(),
        });
    }

    {
        let db = state.db.lock().await;
        db.save_message_with_quote(
            "user",
            &message,
            None,
            quoted_role.as_deref(),
            quoted_content.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(parsed_task) = parse_schedule_request(&message, unix_now()) {
        let saved_task = {
            let db = state.db.lock().await;
            let id = db
                .save_scheduled_task(storage::NewScheduledTask {
                    title: &parsed_task.title,
                    note: &parsed_task.note,
                    due_at: parsed_task.due_at,
                    repeat: parsed_task.repeat,
                    enabled: true,
                })
                .map_err(|e| e.to_string())?;
            let task = db
                .get_scheduled_task_by_id(id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "定时任务保存后读取失败".to_string())?;
            let reply = schedule_confirmation_text(&task, unix_now());
            db.save_message("assistant", &reply)
                .map_err(|e| e.to_string())?;
            (task, reply)
        };

        if let Ok(mut active) = state.active_chat.lock() {
            active.active = None;
        }
        {
            let mut behavior = state.behavior.lock().await;
            behavior.set_state(behavior::PetState::Speaking);
            behavior.mood.update(Some(0.05), None);
        }

        let _ = app_handle.emit("scheduled-tasks-changed", serde_json::json!({}));
        let _ = app_handle.emit(
            "ai-finished",
            AiFinishedPayload {
                text: saved_task.1,
                thinking: Some(format!(
                    "已通过本地定时任务解析器创建任务 #{}。",
                    saved_task.0.id
                )),
                agent_id: None,
                agent_name: None,
                agent_avatar: None,
            },
        );
        return Ok(serde_json::json!({ "started": true, "scheduled_task_id": saved_task.0.id }));
    }

    // 解析被 @ 提及的智能体（找不到的忽略，全部找不到则按普通桌宠回复）
    let mentioned_agents = resolve_mentioned_agents(&state, mention_agents).await?;

    // 用户交互 + 切换到思考状态
    {
        let mut behavior = state.behavior.lock().await;
        behavior.on_user_interaction();
        behavior.set_state(behavior::PetState::Thinking);
    }

    let _ = app_handle.emit("ai-started", &message);
    let app_handle_clone = app_handle.clone();
    let msg_clone = message.clone();
    let att_clone = attachments.unwrap_or_default();
    let quote_clone = quoted_role
        .as_deref()
        .zip(quoted_content.as_deref())
        .map(|(role, content)| (role.to_string(), content.to_string()));
    tauri::async_runtime::spawn(async move {
        let result = std::panic::AssertUnwindSafe(run_ai_message(
            app_handle_clone.clone(),
            msg_clone,
            att_clone,
            quote_clone,
            mentioned_agents,
        ))
        .catch_unwind()
        .await;
        if let Err(panic) = result {
            let panic_msg = if let Some(s) = panic.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "未知内部错误".to_string()
            };
            // panic 后清空 active_chat 状态，防止宠物卡在"思考 状态
            let state = app_handle_clone.state::<AppState>();
            if let Ok(mut active) = state.active_chat.lock() {
                active.active = None;
            }
            let _ = app_handle_clone.emit(
                "ai-error",
                serde_json::json!({
                    "message": format!("内部错误: {}", panic_msg),
                    "thinking": null,
                    "aborted": false
                }),
            );
        }
    });

    Ok(serde_json::json!({ "started": true }))
}

async fn resolve_mentioned_agents(
    state: &tauri::State<'_, AppState>,
    names: Option<Vec<String>>,
) -> Result<Vec<storage::Agent>, String> {
    let Some(names) = names else {
        return Ok(Vec::new());
    };
    let db = state.db.lock().await;
    db.get_agents_by_names(&names).map_err(|e| e.to_string())
}

async fn run_ai_message(
    app_handle: tauri::AppHandle,
    message: String,
    attachments: Vec<direct_api::UserAttachment>,
    quote: Option<(String, String)>,
    agents: Vec<storage::Agent>,
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
                    message: "未配置直接 API 后端，请先在设置中配置 API Key。".to_string(),
                    thinking: None,
                    aborted: false,
                },
            );
            return;
        };

        let (system_prompt, all_skills, active_skill) = {
            let personality = state.personality.lock().await.clone();
            let profession = state.profession.lock().await.clone();
            let db = state.db.lock().await;
            let custom_prompt = db
                .get_setting("custom_system_prompt")
                .unwrap_or(None)
                .unwrap_or_default();
            let all_skills = db.list_skills().unwrap_or_default();
            let active_skill = db.get_active_skill().unwrap_or(None);
            drop(db);

            let base = openclaw::build_system_prompt(&personality, &profession);
            let base = attach_runtime_identity_prompt(base, &config);
            let base = attach_execution_mode_prompt(base, &config);
            let base = attach_quote_prompt(base);
            let base = attach_word_and_ask_prompt(base);
            let base = attach_coding_prompt(base);
            let base = attach_computer_use_prompt(base);
            let base = attach_weather_prompt(base);
            let base = attach_search_prompt(base);
            let base = attach_memory_prompt(base);
            let base = attach_registered_apps_prompt(base, &state).await;
            let base = attach_custom_prompt(base, &custom_prompt);
            let base = attach_skill_prompt(base, active_skill.as_ref(), &all_skills, true).await;
            let base = attach_agent_prompt(base, &agents);
            (base, all_skills, active_skill)
        };

        let chat_history = {
            let start_id = *state
                .chat_start_id
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let db = state.db.lock().await;
            let mut history = db
                .get_recent_messages_after(start_id, 50)
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

        // 解析本轮技能（仅手动激活），并把技能白名单合入 config
        let effective_skill = resolve_skill_for_message(active_skill.as_ref());
        let mut effective_config = config;
        if let Some(s) = &effective_skill {
            let mut seen: std::collections::HashSet<String> = effective_config
                .auto_approved_tools
                .iter()
                .cloned()
                .collect();
            for t in &s.allowed_tools {
                if seen.insert(t.clone()) {
                    effective_config.auto_approved_tools.push(t.clone());
                }
            }
            // 仅在激活技能 ID 发生变化时 emit，避免每轮都提示
            let mut last_id = state.last_active_skill_id.lock().await;
            if last_id.as_deref() != Some(s.id.as_str()) {
                *last_id = Some(s.id.clone());
                let _ = app_handle.emit(
                    "ai-skill-activated",
                    serde_json::json!({
                        "skill_id": s.id,
                        "skill_name": s.name,
                        "source": "manual",
                    }),
                );
            }
        } else {
            let mut last_id = state.last_active_skill_id.lock().await;
            *last_id = None;
        }

        // 智能体：合并工具白名单 + 单智能体时允许模型覆盖
        let agent_ctx = AgentContext { agents };
        {
            let mut seen: std::collections::HashSet<String> = effective_config
                .auto_approved_tools
                .iter()
                .cloned()
                .collect();
            for agent in &agent_ctx.agents {
                for tool in &agent.allowed_tools {
                    if seen.insert(tool.clone()) {
                        effective_config.auto_approved_tools.push(tool.clone());
                    }
                }
            }
        }
        if let Some(agent) = agent_ctx.single() {
            let model = agent.model.trim();
            if !model.is_empty() {
                effective_config.model = model.to_string();
            }
        }

        // 创建新的 CancellationToken
        let token = tokio_util::sync::CancellationToken::new();
        {
            let mut abort = state.abort_token.lock().await;
            *abort = Some(token);
        }

        direct_api::run_direct_api_agent(
            app_handle.clone(),
            message,
            effective_config,
            system_prompt,
            chat_history,
            attachments,
            quote,
            Some(agent_ctx),
        )
        .await;

        // 执行结束，清空 abort_token
        {
            let mut abort = state.abort_token.lock().await;
            *abort = None;
        }
        return;
    }

    // 启动 Claude CLI 流式子进程
    let (system_prompt, all_skills, active_skill) = {
        let state = app_handle.state::<AppState>();
        let personality = state.personality.lock().await.clone();
        let profession = state.profession.lock().await.clone();
        let db = state.db.lock().await;
        let custom_prompt = db
            .get_setting("custom_system_prompt")
            .unwrap_or(None)
            .unwrap_or_default();
        let all_skills = db.list_skills().unwrap_or_default();
        let active_skill = db.get_active_skill().unwrap_or(None);
        drop(db);

        let base = openclaw::build_system_prompt(&personality, &profession);
        let base = attach_memory_prompt(base);
        let base = attach_registered_apps_prompt(base, &state).await;
        let base = attach_custom_prompt(base, &custom_prompt);
        let base = attach_skill_prompt(base, active_skill.as_ref(), &all_skills, false).await;
        let base = attach_agent_prompt(base, &agents);
        (base, all_skills, active_skill)
    };

    // openclaw 走的是 CLI 子进程，不消耗 DirectApiConfig，因此只触发事件，不改 auto_approved_tools
    if let Some(effective) = resolve_skill_for_message(active_skill.as_ref()) {
        let state = app_handle.state::<AppState>();
        let mut last_id = state.last_active_skill_id.lock().await;
        if last_id.as_deref() != Some(effective.id.as_str()) {
            *last_id = Some(effective.id.clone());
            let _ = app_handle.emit(
                "ai-skill-activated",
                serde_json::json!({
                    "skill_id": effective.id,
                    "skill_name": effective.name,
                    "source": "manual",
                }),
            );
        }
    } else {
        let state = app_handle.state::<AppState>();
        let mut last_id = state.last_active_skill_id.lock().await;
        *last_id = None;
    }

    let child_result = {
        let state = app_handle.state::<AppState>();
        let ai = state.ai.lock().await;
        let message_for_cli = apply_quote_context(&message, quote.as_ref());
        ai.spawn_streaming(&app_handle, &message_for_cli, &system_prompt)
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
                        message: "已中断".to_string(),
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

            // 保存对话（单智能体时带上归属）
            let state = app_handle.state::<AppState>();
            let single_agent = if agents.len() == 1 {
                agents.first()
            } else {
                None
            };
            {
                let db = state.db.lock().await;
                let _ = db.save_message_with_agent(
                    "assistant",
                    &parsed.full_text,
                    if parsed.full_thinking.is_empty() {
                        None
                    } else {
                        Some(parsed.full_thinking.as_str())
                    },
                    single_agent.map(|agent| agent.id.as_str()),
                    single_agent.map(|agent| agent.name.as_str()),
                    single_agent.map(|agent| agent.avatar.as_str()),
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
                    agent_id: single_agent.map(|agent| agent.id.clone()),
                    agent_name: single_agent.map(|agent| agent.name.clone()),
                    agent_avatar: single_agent.map(|agent| agent.avatar.clone()),
                },
            );
        }
        Err(_) => {
            // 读取流出错（可能是子进程被 kill了
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
                    message: "已中断".to_string(),
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
    stream_mode: Option<String>,
    api_mode: Option<String>,
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
            stream_mode: stream_mode.unwrap_or_else(|| existing_profile.stream_mode.clone()),
            api_mode: api_mode.unwrap_or_else(|| existing_profile.api_mode.clone()),
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
        "stream_mode": profile.stream_mode,
        "api_mode": profile.api_mode,
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
    api_mode: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let resolved_key = resolve_masked_api_key(&*state.db.lock().await, &api_key);
    let effective_api_mode = api_mode.unwrap_or_else(|| "chat_completions".to_string());
    let url = direct_api::api_client::build_chat_completions_url(&base_url, &effective_api_mode);

    let normalized_depth =
        normalize_thinking_depth(&thinking_depth.unwrap_or_else(|| "auto".to_string()));
    let body = if effective_api_mode == "responses" {
        let input = direct_api::api_client::convert_messages_to_responses_input(&[
            serde_json::json!({ "role": "user", "content": "ping" }),
        ]);
        serde_json::json!({
            "model": model,
            "input": input,
            "max_tokens": 5
        })
    } else {
        serde_json::json!({
            "model": model,
            "messages": [
                { "role": "user", "content": "ping" }
            ],
            "max_tokens": 5
        })
    };
    let mut body = body;
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

    let res = tokio::time::timeout(std::time::Duration::from_secs(60), req.send())
        .await
        .map_err(|_| "连接测试超过 60 秒没有响应，请检查网络、Base URL 或代理配置。".to_string())?
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
async fn test_api_compatibility(
    api_key: String,
    base_url: String,
    model: String,
    thinking_depth: Option<String>,
    api_mode: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use direct_api::ContentExtractor;
    use std::time::Instant;

    let resolved_key = resolve_masked_api_key(&*state.db.lock().await, &api_key);
    let effective_api_mode = api_mode.unwrap_or_else(|| "chat_completions".to_string());
    let url = direct_api::api_client::build_chat_completions_url(&base_url, &effective_api_mode);

    let normalized_depth =
        normalize_thinking_depth(&thinking_depth.unwrap_or_else(|| "auto".to_string()));
    let body = if effective_api_mode == "responses" {
        let input = direct_api::api_client::convert_messages_to_responses_input(&[
            serde_json::json!({ "role": "user", "content": "请回复一个简短的问候语" }),
        ]);
        serde_json::json!({
            "model": model,
            "input": input,
            "stream": true,
            "max_tokens": 50
        })
    } else {
        serde_json::json!({
            "model": model,
            "messages": [
                { "role": "user", "content": "请回复一个简短的问候语" }
            ],
            "stream": true,
            "max_tokens": 50
        })
    };
    let mut body = body;
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

    let start_time = Instant::now();
    let res = tokio::time::timeout(std::time::Duration::from_secs(60), req.send())
        .await
        .map_err(|_| "连接测试超过 60 秒没有响应，请检查网络、Base URL 或代理配置。".to_string())?
        .map_err(|e| format!("连接请求发送失败: {e}"))?;

    let elapsed_ms = start_time.elapsed().as_millis();
    let status = res.status().as_u16();
    let http_ok = res.status().is_success();

    if !http_ok {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Ok(serde_json::json!({
            "http_ok": false,
            "http_status": status,
            "response_time_ms": elapsed_ms,
            "is_sse_format": false,
            "is_json_format": false,
            "content_extracted": null,
            "content_field_path": null,
            "has_reasoning": false,
            "has_tool_calls": false,
            "recommended_mode": null,
            "raw_preview": err_text.chars().take(500).collect::<String>(),
            "errors": [format!("HTTP 错误: {}", status)]
        }));
    }

    // 尝试读取响应内容
    let full_text = res.text().await.unwrap_or_default();

    // 分析响应格式
    let is_sse = full_text.contains("data:") || full_text.contains("event:");
    let is_json = full_text.trim().starts_with('{') || full_text.trim().starts_with('[');

    let mut content_extracted: Option<String> = None;
    let mut content_field_path: Option<String> = None;
    let mut has_reasoning = false;
    let mut has_tool_calls = false;
    let mut recommended_mode: Option<String> = None;
    let mut errors = Vec::new();

    if is_json {
        // 尝试解析为JSON
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&full_text) {
            if let Some(content) = ContentExtractor::extract_content(&value) {
                content_extracted = Some(content);
                content_field_path =
                    Some("choices[0].delta.content 或 choices[0].message.content".to_string());
                recommended_mode = Some("non_stream".to_string());
            }

            has_reasoning = ContentExtractor::extract_reasoning(&value).is_some();

            // 检查tool calls
            if let Some(choices) = value.get("choices") {
                if let Some(first) = choices.as_array().and_then(|arr| arr.first()) {
                    has_tool_calls = first
                        .get("delta")
                        .and_then(|d| d.get("tool_calls"))
                        .is_some()
                        || first
                            .get("message")
                            .and_then(|m| m.get("tool_calls"))
                            .is_some();
                }
            }

            if content_extracted.is_none() {
                errors.push("JSON 中未找到 content 字段".to_string());
                recommended_mode = Some("non_stream".to_string());
            }
        } else {
            errors.push("响应以JSON开头但解析失败".to_string());
        }
    } else if is_sse {
        // SSE格式：尝试解析第一个data。
        let mut found_content = false;
        for line in full_text.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data == "[DONE]" {
                    continue;
                }
                if !data.is_empty() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(content) = ContentExtractor::extract_content(&value) {
                            content_extracted = Some(content);
                            content_field_path = Some("SSE data行中的delta.content".to_string());
                            found_content = true;
                            break;
                        }
                        has_reasoning = ContentExtractor::extract_reasoning(&value).is_some();
                    }
                }
            }
        }

        if found_content {
            recommended_mode = Some("stream".to_string());
        } else {
            errors.push("SSE data行中未找到content字段".to_string());
            recommended_mode = Some("non_stream".to_string());
        }
    } else {
        errors.push("响应既不是SSE格式也不是JSON格式".to_string());
        recommended_mode = Some("non_stream".to_string());
    }

    Ok(serde_json::json!({
        "http_ok": true,
        "http_status": status,
        "response_time_ms": elapsed_ms,
        "is_sse_format": is_sse,
        "is_json_format": is_json,
        "content_extracted": content_extracted,
        "content_field_path": content_field_path,
        "has_reasoning": has_reasoning,
        "has_tool_calls": has_tool_calls,
        "recommended_mode": recommended_mode,
        "raw_preview": full_text.chars().take(500).collect::<String>(),
        "errors": errors
    }))
}

/// 系统通知: 工具确认请求 (用于 chat 窗口未获焦时, 替代强制弹窗)
#[tauri::command]
async fn send_tool_confirm_notification(
    app_handle: tauri::AppHandle,
    summary: Option<String>,
    tool_name: Option<String>,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let title = "AI 桌宠需要确认工具调用";
    let body = summary
        .or(tool_name)
        .unwrap_or_else(|| "AI 请求执行一个工具  请点击桌宠查看详情".to_string());
    app_handle
        .notification()
        .builder()
        .title(title)
        .body(&body)
        .show()
        .map_err(|e| format!("发送通知失败: {e}"))?;
    Ok(())
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

/// 回答 ask_user 弹窗提出的问题  把用户的回答 (选项或自定义输入) 发回给等待中的智能体。
#[tauri::command]
async fn answer_question(
    id: String,
    answer: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mut questions = state.pending_questions.lock().await;
    if let Some(tx) = questions.remove(&id) {
        let _ = tx.send(answer);
        Ok(())
    } else {
        Err("无效的问题 ID（可能已超时或被处理）".to_string())
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

fn load_weather_config(db: &storage::Database) -> system::weather::WeatherConfig {
    db.get_setting(WEATHER_CONFIG_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<system::weather::WeatherConfig>(&raw).ok())
        .map(system::weather::normalize_weather_config)
        .unwrap_or_default()
}

fn save_weather_config(
    db: &storage::Database,
    config: system::weather::WeatherConfig,
) -> Result<system::weather::WeatherConfig, String> {
    let config = system::weather::normalize_weather_config(config);
    if config.enabled {
        system::weather::build_weather_url(&config)?;
    }
    let value = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    db.save_setting(WEATHER_CONFIG_KEY, &value)
        .map_err(|e| e.to_string())?;
    Ok(config)
}

async fn fetch_weather_from_settings(
    state: &tauri::State<'_, AppState>,
    force_refresh: bool,
) -> Result<Option<system::WeatherInfo>, String> {
    let config = {
        let db = state.db.lock().await;
        load_weather_config(&db)
    };

    if !config.enabled {
        return Ok(None);
    }

    log_weather_debug(
        "fetch_weather",
        &format!(
            "force_refresh={force_refresh} location={} api_url={} proxy_env={}",
            config.location,
            config.api_url,
            weather_proxy_env_summary()
        ),
    );

    let cached = { state.weather_cache.lock().await.clone() };
    if !force_refresh {
        if let Some(weather) = cached.clone() {
            if unix_now() - weather.timestamp <= WEATHER_CACHE_TTL_SECS {
                return Ok(Some(weather));
            }
        }
    }

    match get_weather_with_proxy_retry(&state.http_client, &config).await {
        Ok(weather) => {
            log_weather_debug(
                "fetch_weather_ok",
                &format!(
                    "city={} temp={} desc={} source={}",
                    weather.city, weather.temperature, weather.description, config.api_url
                ),
            );
            let mut cache = state.weather_cache.lock().await;
            *cache = Some(weather.clone());
            Ok(Some(weather))
        }
        Err(error) => {
            log_weather_debug("fetch_weather_err", &error);
            if !force_refresh {
                if let Some(weather) = cached {
                    return Ok(Some(weather));
                }
            }
            Err(error)
        }
    }
}

async fn send_weather_if_due(
    app_handle: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let today = system::weather::get_today_date();
    let last_sent = {
        let db = state.db.lock().await;
        db.get_setting(WEATHER_SENT_DATE_KEY)
            .map_err(|e| e.to_string())?
            .unwrap_or_default()
    };

    if last_sent == today {
        log_weather_debug("send_weather_skip", &format!("already_sent_today={today}"));
        return Ok(false);
    }

    let Some(weather) = fetch_weather_from_settings(state, true).await? else {
        log_weather_debug("send_weather_skip", "weather_disabled_or_unavailable");
        return Ok(false);
    };

    let message = system::weather::format_weather_message(&weather);
    log_weather_debug(
        "send_weather_ok",
        &format!("city={} sent_today={today}", weather.city),
    );
    app_handle
        .emit(
            "weather-update",
            serde_json::json!({
                "message": message,
                "sent_today": true,
            }),
        )
        .map_err(|e| format!("发送天气事件失败: {e}"))?;

    let db = state.db.lock().await;
    db.save_setting(WEATHER_SENT_DATE_KEY, &today)
        .map_err(|e| e.to_string())?;

    Ok(true)
}

#[tauri::command]
async fn get_weather_config(
    state: tauri::State<'_, AppState>,
) -> Result<system::weather::WeatherConfig, String> {
    let db = state.db.lock().await;
    Ok(load_weather_config(&db))
}

#[tauri::command]
async fn set_weather_config(
    enabled: bool,
    location: String,
    api_url: String,
    state: tauri::State<'_, AppState>,
) -> Result<system::weather::WeatherConfig, String> {
    let config = system::weather::WeatherConfig {
        enabled,
        location,
        api_url,
    };
    let saved = {
        let db = state.db.lock().await;
        save_weather_config(&db, config)?
    };

    let mut cache = state.weather_cache.lock().await;
    *cache = None;
    Ok(saved)
}

#[tauri::command]
async fn test_weather_config(
    location: String,
    api_url: String,
    state: tauri::State<'_, AppState>,
) -> Result<system::WeatherInfo, String> {
    let config = system::weather::WeatherConfig {
        enabled: true,
        location,
        api_url,
    };
    log_weather_debug(
        "test_weather",
        &format!(
            "location={} api_url={} proxy_env={}",
            config.location,
            config.api_url,
            weather_proxy_env_summary()
        ),
    );
    match get_weather_with_proxy_retry(&state.http_client, &config).await {
        Ok(weather) => {
            log_weather_debug(
                "test_weather_ok",
                &format!(
                    "city={} temp={} desc={}",
                    weather.city, weather.temperature, weather.description
                ),
            );
            Ok(weather)
        }
        Err(error) => {
            log_weather_debug("test_weather_err", &error);
            Err(error)
        }
    }
}

#[tauri::command]
async fn get_weather(
    state: tauri::State<'_, AppState>,
) -> Result<Option<system::WeatherInfo>, String> {
    fetch_weather_from_settings(&state, false).await
}

#[tauri::command]
async fn refresh_weather(
    state: tauri::State<'_, AppState>,
) -> Result<Option<system::WeatherInfo>, String> {
    fetch_weather_from_settings(&state, true).await
}

#[tauri::command]
async fn check_and_send_weather(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    send_weather_if_due(&app_handle, &state).await
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

const MAX_MEMORY_VALUE_BYTES: usize = 64 * 1024;
const MAX_MEMORY_KEY_BYTES: usize = 256;
const MAX_MEMORY_CATEGORY_BYTES: usize = 64;

#[tauri::command]
async fn save_memory(
    category: String,
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<storage::MemoryItem, String> {
    if category.len() > MAX_MEMORY_CATEGORY_BYTES {
        return Err(format!(
            "分类名称过长（上限 {} 字节）",
            MAX_MEMORY_CATEGORY_BYTES
        ));
    }
    if key.len() > MAX_MEMORY_KEY_BYTES {
        return Err(format!("键名过长（上限 {} 字节）", MAX_MEMORY_KEY_BYTES));
    }
    if value.len() > MAX_MEMORY_VALUE_BYTES {
        return Err(format!(
            "记忆内容过长：{} 字节（上限 {} 字节 / 64KB）",
            value.len(),
            MAX_MEMORY_VALUE_BYTES
        ));
    }
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
async fn save_scheduled_task(
    title: String,
    note: Option<String>,
    due_at: i64,
    repeat: Option<String>,
    enabled: Option<bool>,
    state: tauri::State<'_, AppState>,
) -> Result<storage::ScheduledTask, String> {
    let title = title.trim().to_string();
    validate_schedule_input(&title, due_at)?;

    let note = note.unwrap_or_default().trim().to_string();
    if note.chars().count() > 2_000 {
        return Err("任务备注不能超过 2000 个字符".to_string());
    }
    let repeat = normalize_task_repeat(repeat.as_deref().unwrap_or(SCHEDULE_REPEAT_ONCE));
    let enabled = enabled.unwrap_or(true);

    let db = state.db.lock().await;
    let id = db
        .save_scheduled_task(storage::NewScheduledTask {
            title: &title,
            note: &note,
            due_at,
            repeat,
            enabled,
        })
        .map_err(|e| e.to_string())?;
    db.get_scheduled_task_by_id(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "定时任务保存后读取失败".to_string())
}

#[tauri::command]
async fn get_scheduled_tasks(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<storage::ScheduledTask>, String> {
    let db = state.db.lock().await;
    db.get_all_scheduled_tasks().map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_scheduled_task_enabled(
    id: i64,
    enabled: bool,
    state: tauri::State<'_, AppState>,
) -> Result<storage::ScheduledTask, String> {
    let db = state.db.lock().await;
    db.set_scheduled_task_enabled(id, enabled)
        .map_err(|e| e.to_string())?;
    db.get_scheduled_task_by_id(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "未找到定时任务".to_string())
}

#[tauri::command]
async fn delete_scheduled_task(id: i64, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_scheduled_task(id).map_err(|e| e.to_string())
}

async fn process_due_scheduled_tasks(app_handle: &tauri::AppHandle) {
    let now = unix_now();
    let state = app_handle.state::<AppState>();
    let due_tasks = {
        let db = state.db.lock().await;
        db.get_due_scheduled_tasks(now).unwrap_or_default()
    };

    if due_tasks.is_empty() {
        return;
    }

    for task in due_tasks {
        let message = scheduled_task_message(&task);
        {
            let db = state.db.lock().await;
            let next_due = next_due_at_for_repeat(&task.repeat, task.due_at, now);
            let _ = db.reschedule_triggered_task(task.id, next_due, now);
            let _ = db.save_message("system", &message);
        }

        let _ = app_handle.emit(
            "scheduled-task-triggered",
            ScheduledTaskTriggeredPayload {
                task: task.clone(),
                message: message.clone(),
            },
        );
        let _ = app_handle.emit(
            "sync-chat-message",
            serde_json::json!({
                "role": "system",
                "content": message,
            }),
        );
    }
}

fn start_scheduled_task_runner(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            interval.tick().await;
            process_due_scheduled_tasks(&app_handle).await;
        }
    });
}

const MAX_CLIPBOARD_BYTES: usize = 64 * 1024;

#[tauri::command]
async fn save_clipboard_item(
    content: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if content.len() > MAX_CLIPBOARD_BYTES {
        return Err(format!(
            "剪贴板内容过长：{} 字节（上限 {} 字节 / 64KB）",
            content.len(),
            MAX_CLIPBOARD_BYTES
        ));
    }
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

const MAX_USER_AVATAR_BYTES: usize = 256 * 1024;

#[tauri::command]
async fn set_user_avatar(avatar: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if avatar.len() > MAX_USER_AVATAR_BYTES {
        return Err(format!(
            "头像数据过大：{} 字节（上限 {} 字节 / 256KB）",
            avatar.len(),
            MAX_USER_AVATAR_BYTES
        ));
    }
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
async fn save_custom_pet_asset(
    request: SaveCustomPetAssetRequest,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<storage::CustomPetAsset, String> {
    validate_pixel_pet_manifest(&request.manifest)?;

    let id = request
        .id
        .filter(|value| is_valid_custom_pet_id(value))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let name = normalize_custom_pet_name(&request.name);
    let kind = request.kind.unwrap_or_else(|| "custom-pixel".to_string());
    if kind != "custom-pixel" {
        return Err("Unsupported custom pet kind".to_string());
    }

    let dir = custom_pet_asset_dir(&app_handle, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let sprite_path = dir.join("spritesheet.png");
    let preview_path = dir.join("preview.png");
    let sprite_bytes = decode_png_data_url(&request.sprite_data_url)?;
    std::fs::write(&sprite_path, sprite_bytes).map_err(|e| e.to_string())?;

    let preview_path_string = if let Some(preview_data_url) = request.preview_data_url {
        let preview_bytes = decode_png_data_url(&preview_data_url)?;
        std::fs::write(&preview_path, preview_bytes).map_err(|e| e.to_string())?;
        preview_path.to_string_lossy().to_string()
    } else {
        String::new()
    };

    let asset = storage::CustomPetAsset {
        id: id.clone(),
        name,
        kind,
        manifest: request.manifest,
        sprite_path: sprite_path.to_string_lossy().to_string(),
        preview_path: preview_path_string,
        created_at: 0,
        updated_at: 0,
    };

    let db = state.db.lock().await;
    db.save_custom_pet_asset(&asset)
        .map_err(|e| e.to_string())?;
    db.get_custom_pet_asset(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Custom pet asset was not saved".to_string())
}

#[tauri::command]
async fn list_custom_pet_assets(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<storage::CustomPetAsset>, String> {
    let db = state.db.lock().await;
    db.list_custom_pet_assets().map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_custom_pet_asset(
    id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Option<storage::CustomPetAsset>, String> {
    let db = state.db.lock().await;
    db.get_custom_pet_asset(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_active_custom_pet_asset(
    state: tauri::State<'_, AppState>,
) -> Result<Option<storage::CustomPetAsset>, String> {
    let db = state.db.lock().await;
    let active_id = db
        .get_setting(CUSTOM_PIXEL_PET_SETTING_KEY)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if active_id.trim().is_empty() {
        return Ok(None);
    }

    let asset = db
        .get_custom_pet_asset(&active_id)
        .map_err(|e| e.to_string())?;
    if asset.is_none() {
        db.save_setting(CUSTOM_PIXEL_PET_SETTING_KEY, "")
            .map_err(|e| e.to_string())?;
        if db
            .get_setting("pet_character")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("custom-pixel")
        {
            db.save_setting("pet_character", "classic")
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(asset)
}

#[tauri::command]
async fn set_active_custom_pet_asset(
    id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<Option<storage::CustomPetAsset>, String> {
    let db = state.db.lock().await;
    let id = id.unwrap_or_default();
    let id = id.trim();

    if id.is_empty() {
        db.save_setting(CUSTOM_PIXEL_PET_SETTING_KEY, "")
            .map_err(|e| e.to_string())?;
        if db
            .get_setting("pet_character")
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some("custom-pixel")
        {
            db.save_setting("pet_character", "classic")
                .map_err(|e| e.to_string())?;
        }
        return Ok(None);
    }

    if !is_valid_custom_pet_id(id) {
        return Err("Invalid custom pet asset id".to_string());
    }

    let asset = db
        .get_custom_pet_asset(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Custom pet asset was not found".to_string())?;
    db.save_setting(CUSTOM_PIXEL_PET_SETTING_KEY, id)
        .map_err(|e| e.to_string())?;
    db.save_setting("pet_character", "custom-pixel")
        .map_err(|e| e.to_string())?;
    Ok(Some(asset))
}

#[tauri::command]
async fn delete_custom_pet_asset(
    id: String,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if !is_valid_custom_pet_id(&id) {
        return Err("Invalid custom pet asset id".to_string());
    }

    let deleted = {
        let db = state.db.lock().await;
        let deleted = db.delete_custom_pet_asset(&id).map_err(|e| e.to_string())?;
        if db
            .get_setting(CUSTOM_PIXEL_PET_SETTING_KEY)
            .map_err(|e| e.to_string())?
            .as_deref()
            == Some(id.as_str())
        {
            db.save_setting(CUSTOM_PIXEL_PET_SETTING_KEY, "")
                .map_err(|e| e.to_string())?;
            if db
                .get_setting("pet_character")
                .map_err(|e| e.to_string())?
                .as_deref()
                == Some("custom-pixel")
            {
                db.save_setting("pet_character", "classic")
                    .map_err(|e| e.to_string())?;
            }
        }
        deleted
    };

    if deleted.is_some() {
        let dir = custom_pet_asset_dir(&app_handle, &id)?;
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
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
    let font_color = font_color.trim().to_string();
    let valid_hex = font_color.is_empty()
        || (matches!(font_color.len(), 4 | 5 | 7 | 9)
            && font_color.starts_with('#')
            && font_color[1..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !valid_hex {
        return Err("字体颜色必须为 #RGB、#RGBA、#RRGGBB 或 #RRGGBBAA 格式".to_string());
    }
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

/// 允许前端通过 set_setting_value 修改的设置键白名单
const ALLOWED_SETTING_KEYS: &[&str] = &[
    "theme",
    "voice_enabled",
    "voice_name",
    "auto_start",
    "font_size",
    "font_color",
    "skin",
    "personality",
    "profession",
    "backend_type",
    "model",
    "confirm_enabled",
    "thinking_depth",
    "execution_mode",
    "search_provider",
    "stream_mode",
    "api_mode",
    "computer_use_enabled",
    "computer_use_scale",
    "pet_character",
    "custom_pixel_pet_asset_id",
    "voice_settings",
    "tts_settings",
];

#[tauri::command]
async fn set_setting_value(
    key: String,
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if !ALLOWED_SETTING_KEYS.contains(&key.as_str()) {
        return Err(format!("不允许修改此设置：  {}", key));
    }
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

// ===== 自定义系统提示 + 技能 =====

#[tauri::command]
async fn get_custom_system_prompt(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let db = state.db.lock().await;
    db.get_setting("custom_system_prompt")
        .map(|opt| opt.unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_custom_system_prompt(
    text: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.save_setting("custom_system_prompt", text.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_skills(state: tauri::State<'_, AppState>) -> Result<Vec<storage::Skill>, String> {
    let db = state.db.lock().await;
    db.list_skills().map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_skill(
    skill: storage::Skill,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    validate_skill_for_save(&skill)?;
    let db = state.db.lock().await;
    db.save_skill(&skill).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_skill(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_skill(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn reset_builtin_skills(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.reset_builtin_skills().map_err(|e| e.to_string())
}

/// 保存技能前的强校验：限制字段长度与字符集，避免巨型 prompt 撑爆上下文
/// 并减小通过 name/description/system_prompt 注入系统提示的攻击面。
/// `builtin-` 命名空间仅允许已存在的内置 id：自定义技能不能用 `builtin-` 前缀。
fn validate_skill_for_save(skill: &storage::Skill) -> Result<(), String> {
    const ID_MAX: usize = 64;
    const NAME_MAX: usize = 80;
    const DESC_MAX: usize = 300;
    const PROMPT_MAX: usize = 4_000;
    const TOOL_MAX: usize = 30;
    const TOOL_NAME_MAX: usize = 64;
    const KEYWORD_MAX: usize = 20;
    const KEYWORD_MAX_LEN: usize = 32;

    let id = skill.id.trim();
    if id.is_empty() {
        return Err("技能 id 不能为空".to_string());
    }
    if id.chars().count() > ID_MAX {
        return Err(format!("技能 id 长度超过 {ID_MAX}"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("技能 id 只能包含字母、数字、 -' 和 '_'".to_string());
    }
    if id.starts_with("builtin-") && !storage::is_builtin_skill_id(id) {
        return Err("`builtin-` 前缀为内置技能保留，自定义技能请换一个 id".to_string());
    }

    let name = skill.name.trim();
    if name.is_empty() {
        return Err("技能名称不能为空".to_string());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("技能名称不能超过 {NAME_MAX} 个字。"));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("技能名称不能包含控制字符".to_string());
    }

    let description = skill.description.trim();
    if description.chars().count() > DESC_MAX {
        return Err(format!("技能描述不能超过 {DESC_MAX} 个字。"));
    }
    if description.chars().any(|c| c.is_control() && c != '\n') {
        return Err("技能描述不能包含控制字符".to_string());
    }

    let system_prompt = skill.system_prompt.trim();
    if system_prompt.chars().count() > PROMPT_MAX {
        return Err(format!("技能 system_prompt 不能超过 {PROMPT_MAX} 个字。"));
    }

    if skill.allowed_tools.len() > TOOL_MAX {
        return Err(format!("单个技能最多 {TOOL_MAX} 个工具"));
    }
    for t in &skill.allowed_tools {
        let t = t.trim();
        if t.is_empty() || t.chars().count() > TOOL_NAME_MAX {
            return Err(format!("工具名长度不合法: {t:?}"));
        }
        if !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("工具名只能包含字母、数字和 '_'：{t:?}"));
        }
    }

    if skill.keywords.len() > KEYWORD_MAX {
        return Err(format!("单个技能最多 {KEYWORD_MAX} 个关键词"));
    }
    for k in &skill.keywords {
        let k = k.trim();
        if k.is_empty() || k.chars().count() > KEYWORD_MAX_LEN {
            return Err("关键词长度不合法".to_string());
        }
    }

    Ok(())
}

#[tauri::command]
async fn set_active_skill(
    id: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.set_active_skill(id.as_deref())
        .map_err(|e| e.to_string())
}

// ===== 智能体（多智能体） =====

#[tauri::command]
async fn list_agents(state: tauri::State<'_, AppState>) -> Result<Vec<storage::Agent>, String> {
    let db = state.db.lock().await;
    db.list_agents().map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_agent(
    agent: storage::Agent,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    validate_agent_for_save(&agent)?;
    let db = state.db.lock().await;
    // 名字唯一：@ 提及按名字匹配，重名会导致歧义。
    if let Some(existing) = db
        .get_agent_by_name(agent.name.trim())
        .map_err(|e| e.to_string())?
    {
        if existing.id != agent.id {
            return Err(format!("已存在名为「{}」的智能体，请换一个名字", agent.name.trim()));
        }
    }
    db.save_agent(&agent).map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_agent(id: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let db = state.db.lock().await;
    db.delete_agent(&id).map_err(|e| e.to_string())
}

/// 让 AI 根据用户的自然语言描述生成智能体定义（name/avatar/description/system_prompt/allowed_tools）。
/// 只做一次非流式调用，返回可保存的 JSON；前端拿到后预填表单供用户确认。
#[tauri::command]
async fn generate_agent_spec(
    description: String,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    const DESC_MAX: usize = 2000;
    if description.trim().is_empty() {
        return Err("请先描述你想创建的智能体".to_string());
    }
    if description.chars().count() > DESC_MAX {
        return Err(format!("描述不能超过 {DESC_MAX} 个字。"));
    }

    let config = {
        let cfg = state.direct_api_config.lock().await;
        cfg.clone()
            .ok_or_else(|| "未配置直接 API，请先在系统设置中配置 API Key".to_string())?
    };

    let tool_names: Vec<String> = direct_api::tool_definitions()
        .iter()
        .filter_map(|def| def.get("function")?.get("name")?.as_str().map(str::to_string))
        .collect();
    let tool_catalog = if tool_names.is_empty() {
        "（无可用工具）".to_string()
    } else {
        tool_names.join(", ")
    };

    let system_prompt = format!(
        "你是「智能体工厂」。用户会描述他想要的智能体，你要生成一个可直接保存的智能体定义， 只输出一个 JSON 对象，不要输出任何解释文字，不要用 Markdown 代码块包裹。 JSON 必须包含这些字段： - name: 简短的名字（2-8 个汉字或字母，不要包含空格和 @），用于对话时 @ 提及
- avatar: 一个最能代表它的 emoji（单个字符）
- description: 一句话描述（20 字以内），说明它擅长什么 - system_prompt: 详细的中文角色设定（120-500 字），用第二人称「你」书写，包含身份、职责、工作方式、输出风格和边界；要具体可执行，不要照抄用户原话
- allowed_tools: 字符串数组，从以下工具列表里只选它完成任务必需的工具，没有就不选：{}
要求：name 不要与用户描述的智能体重名冲突；内容专业、克制，",
        tool_catalog
    );

    let api_messages = vec![
        serde_json::json!({ "role": "system", "content": system_prompt }),
        serde_json::json!({ "role": "user", "content": format!("请为以下需求创建智能体：{description}") }),
    ];

    let result = direct_api::call_chat_completions_non_stream(
        &state.http_client,
        &config,
        &api_messages,
        None,
    )
    .await
    .map_err(|e| format!("生成智能体定义失败  {e}"))?;

    let raw = result.content.unwrap_or_default().trim().to_string();
    let json_text = extract_json_object(&raw)
        .ok_or_else(|| format!("AI 返回的内容不是 JSON，请重试。原始内容：{}", truncate_for_error(&raw)))?;
    let value: serde_json::Value =
        serde_json::from_str(&json_text).map_err(|e| format!("解析智能体定义失败  {e}"))?;

    Ok(normalize_generated_agent_spec(value, &tool_names))
}

/// 从模型输出里抠出第一个 JSON 对象（容忍 markdown 代码围栏和前后废话））
fn extract_json_object(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(raw[start..=end].to_string())
}

fn truncate_for_error(raw: &str) -> String {
    let mut chars = raw.chars();
    let head: String = chars.by_ref().take(80).collect();
    if chars.next().is_some() {
        format!("{head}。")
    } else {
        head
    }
}

/// 把模型生成的智能体 JSON 规范化成安全可保存的结构）
fn normalize_generated_agent_spec(
    value: serde_json::Value,
    tool_names: &[String],
) -> serde_json::Value {
    const NAME_MAX: usize = 16;
    const DESC_MAX: usize = 100;
    const PROMPT_MAX: usize = 2000;
    let allowed: std::collections::HashSet<&String> = tool_names.iter().collect();

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("新智能体")
        .trim()
        .replace([' ', '@', '\t', '\n'], "")
        .chars()
        .take(NAME_MAX)
        .collect::<String>();
    let avatar = value
        .get("avatar")
        .and_then(|v| v.as_str())
        .unwrap_or("🤖")
        .trim()
        .chars()
        .take(2)
        .collect::<String>();
    let description = value
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(DESC_MAX)
        .collect::<String>();
    let system_prompt = value
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(PROMPT_MAX)
        .collect::<String>();
    let allowed_tools: Vec<String> = value
        .get("allowed_tools")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .filter(|tool| allowed.contains(tool))
                .take(30)
                .collect()
        })
        .unwrap_or_default();

    serde_json::json!({
        "name": name,
        "avatar": avatar,
        "description": description,
        "system_prompt": system_prompt,
        "allowed_tools": allowed_tools,
    })
}

/// 保存智能体前的强校验：名字不能含空白或 @（@ 提及解析依赖这一点））
fn validate_agent_for_save(agent: &storage::Agent) -> Result<(), String> {
    const ID_MAX: usize = 64;
    const NAME_MAX: usize = 40;
    const AVATAR_MAX: usize = 8;
    const DESC_MAX: usize = 300;
    const PROMPT_MAX: usize = 4000;
    const MODEL_MAX: usize = 200;
    const TOOL_MAX: usize = 30;
    const TOOL_NAME_MAX: usize = 64;

    let id = agent.id.trim();
    if id.is_empty() {
        return Err("智能体 id 不能为空".to_string());
    }
    if id.chars().count() > ID_MAX {
        return Err(format!("智能体 id 长度超过 {ID_MAX}"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("智能体 id 只能包含字母、数字、'-' 和 '_'".to_string());
    }

    let name = agent.name.trim();
    if name.is_empty() {
        return Err("智能体名字不能为空".to_string());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("智能体名字不能超过 {NAME_MAX} 个字。"));
    }
    if name.chars().any(|c| c.is_whitespace() || c == '@') {
        return Err("智能体名字不能包含空格或 @（名字会用于对话中的 @ 提及）".to_string());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("智能体名字不能包含控制字符".to_string());
    }

    if agent.avatar.chars().count() > AVATAR_MAX {
        return Err(format!("头像符号不能超过 {AVATAR_MAX} 个字。"));
    }
    if agent.avatar.chars().any(|c| c.is_control()) {
        return Err("头像符号不能包含控制字符".to_string());
    }

    if agent.description.chars().count() > DESC_MAX {
        return Err(format!("描述不能超过 {DESC_MAX} 个字。"));
    }
    if agent.description.chars().any(|c| c.is_control() && c != '\n') {
        return Err("描述不能包含控制字符".to_string());
    }

    if agent.system_prompt.chars().count() > PROMPT_MAX {
        return Err(format!("system_prompt 不能超过 {PROMPT_MAX} 个字。"));
    }

    if agent.model.chars().count() > MODEL_MAX {
        return Err(format!("模型名不能超过 {MODEL_MAX} 个字。"));
    }
    if agent.model.chars().any(|c| c.is_control()) {
        return Err("模型名不能包含控制字符".to_string());
    }

    if agent.allowed_tools.len() > TOOL_MAX {
        return Err(format!("单个智能体最多 {TOOL_MAX} 个工具"));
    }
    for tool in &agent.allowed_tools {
        let tool = tool.trim();
        if tool.is_empty() || tool.chars().count() > TOOL_NAME_MAX {
            return Err(format!("工具名长度不合法: {tool:?}"));
        }
        if !tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("工具名只能包含字母、数字和 '_'：{tool:?}"));
        }
    }

    Ok(())
}


#[tauri::command]
async fn spawn_subprocess(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    params: subprocess::SpawnParams,
) -> Result<subprocess::ProcessInfo, String> {
    subprocess::spawn_subprocess(app, &state.subprocs, params).await
}

#[tauri::command]
async fn send_stdin(
    state: tauri::State<'_, AppState>,
    id: String,
    text: String,
    append_newline: Option<bool>,
) -> Result<(), String> {
    subprocess::send_stdin(&state.subprocs, id, text, append_newline.unwrap_or(true)).await
}

#[tauri::command]
async fn kill_subprocess(
    state: tauri::State<'_, AppState>,
    id: String,
    force: Option<bool>,
) -> Result<(), String> {
    subprocess::kill_subprocess(&state.subprocs, id, force.unwrap_or(false)).await
}

#[tauri::command]
async fn list_subprocesses(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<subprocess::ProcessInfo>, String> {
    subprocess::list_subprocesses(&state.subprocs).await
}

#[tauri::command]
async fn eat_files(paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Err("没有提供任何路径".to_string());
    }
    if paths.len() > 100 {
        return Err(format!(
            "一次最多只能移动 100 个文件到回收站，当前传入 {} 个",
            paths.len()
        ));
    }
    for p in &paths {
        if p.is_empty() {
            return Err("路径不能为空".to_string());
        }
        direct_api::tools::is_path_allowed(p)?;
    }
    trash::delete_all(&paths).map_err(|e| e.to_string())
}

async fn get_shortcut_target_path(lnk_path: &str) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // 路径安全验证：必须是绝对路径、以 .lnk 结尾。
        // 真实桌面/开始菜单路径里常含空格、括号、&、`-` 等字符，
        // 不能用"出现特定字符就拒"的简单黑名单。
        let p = std::path::Path::new(lnk_path);
        if !p.is_absolute() {
            return Err("快捷方式路径必须是绝对路径".to_string());
        }
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("lnk") {
            return Err("仅支持 .lnk 快捷方式文件".to_string());
        }

        let script = format!(
            "$sh = New-Object -ComObject WScript.Shell; \
             $sc = $sh.CreateShortcut('{}'); \
             $tp = $sc.TargetPath; \
             if ([string]::IsNullOrEmpty($tp)) {{ Write-Output '__EMPTY__' }} \
             else {{ Write-Output $tp }}",
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
        if target == "__EMPTY__" || target.is_empty() {
            return Err(
                "此快捷方式没有 TargetPath（可能是 UWP 应用、协议链接或网络位置）".to_string(),
            );
        }
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
    if paths.len() > 50 {
        return Err(format!(
            "一次最多只能查询 50 个文件，当前传入 {} 个",
            paths.len()
        ));
    }
    let mut results = Vec::new();
    for p in &paths {
        if p.is_empty() {
            return Err("路径不能为空".to_string());
        }
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
    // 路径安全验证：仅允许访问用户文档、桌面、下载等目录
    let canonical = direct_api::tools::is_path_allowed(&path)?;
    if !canonical.is_file() {
        return Err("文件不存在或不是文件".into());
    }
    let size = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| e.to_string())?
        .len();
    if size > 5 * 1024 * 1024 {
        return Err("文件过大，无法生成预览 (>5MB)".into());
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| e.to_string())?;
    let ext = canonical
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => return Err(format!("不支持的文件类型: .{ext}")),
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

const MAX_TTS_TEXT_BYTES: usize = 4 * 1024;

#[tauri::command]
async fn tts_synthesize(
    text: String,
    voice: String,
    rate: Option<i32>,
    pitch: Option<i32>,
    volume: Option<i32>,
    state: tauri::State<'_, AppState>,
) -> Result<tts::TtsResult, String> {
    if text.len() > MAX_TTS_TEXT_BYTES {
        return Err(format!(
            "TTS 文本过长：{} 字节（上限 {} 字节 / 4KB）",
            text.len(),
            MAX_TTS_TEXT_BYTES
        ));
    }
    if voice.len() > 128 {
        return Err("TTS 声音名称过长".to_string());
    }
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
async fn tts_play(
    text: String,
    voice: String,
    rate: Option<i32>,
    pitch: Option<i32>,
    volume: Option<i32>,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if text.len() > MAX_TTS_TEXT_BYTES {
        return Err(format!(
            "TTS 文本过长：{} 字节（上限 {} 字节 / 4KB）",
            text.len(),
            MAX_TTS_TEXT_BYTES
        ));
    }
    if voice.len() > 128 {
        return Err("TTS 声音名称过长".to_string());
    }
    let req = tts::TtsRequest {
        text,
        voice,
        rate: rate.unwrap_or(0),
        pitch: pitch.unwrap_or(0),
        volume: volume.unwrap_or(100),
    };
    let (audio_path, _) = state.tts.synthesize_to_file(&req).await?;
    let (playback_id, player) = state.tts_playback.play_wav_file(&audio_path)?;
    tokio::task::spawn_blocking(move || player.sleep_until_end())
        .await
        .map_err(|e| format!("TTS 播放任务失败: {e}"))?;
    state.tts_playback.clear_if_current(playback_id);
    Ok(())
}

#[tauri::command]
fn tts_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.tts_playback.stop();
    Ok(())
}

#[tauri::command]
async fn tts_list_voices() -> Result<Vec<tts::TtsVoice>, String> {
    tts::TtsManager::list_voices().await
}

#[tauri::command]
fn tts_cache_stats(state: tauri::State<'_, AppState>) -> Result<tts::TtsCacheStats, String> {
    Ok(state.tts.cache_stats())
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
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            // 主窗口在 tauri.conf.json 中已设为 "create": false。
            // 这里手动创建以便让 debug/release 为 WebView2 分配独立。
            // user data 目录，避免 dev 模式写入到 localhost:1420 历史
            // 污染 release exe，导致快捷方式偶尔出现 "localhost 拒绝连接"。
            if let Some(main_window_config) = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
            {
                let data_directory_subdir = if cfg!(debug_assertions) {
                    "ai-desktop-pet-dev"
                } else {
                    "ai-desktop-pet-release"
                };
                let data_directory = app
                    .path()
                    .local_data_dir()
                    .ok()
                    .unwrap_or_else(std::path::PathBuf::new)
                    .join(data_directory_subdir);
                tauri::WebviewWindowBuilder::from_config(app.handle(), main_window_config)?
                    .data_directory(data_directory)
                    .build()?;
            }

            let http_client = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .pool_idle_timeout(std::time::Duration::from_secs(90))
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
                tts_playback: tts::playback::NativeAudioPlayer::new(),
                backend_type: tokio::sync::Mutex::new(backend_type),
                direct_api_config: tokio::sync::Mutex::new(direct_config),
                pending_confirms: tokio::sync::Mutex::new(std::collections::HashMap::new()),
                pending_questions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
                approved_tool_types: tokio::sync::Mutex::new(std::collections::HashSet::new()),
                abort_token: tokio::sync::Mutex::new(None),
                chat_start_id: StdMutex::new(chat_start_id),
                http_client,
                weather_cache: tokio::sync::Mutex::new(None),
                last_active_skill_id: tokio::sync::Mutex::new(None),
                subprocs: subprocess::SubprocessManager::new(),
            };
            app.manage(app_state);
            start_scheduled_task_runner(app.handle().clone());

            // 启动时检查并发送天气
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 延迟2秒等待前端准备就绪
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                let state = app_handle.state::<AppState>();
                if let Err(e) = send_weather_if_due(&app_handle, &state).await {
                    eprintln!("启动时获取天气失败: {}", e);
                }
            });

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
            save_scheduled_task,
            get_scheduled_tasks,
            set_scheduled_task_enabled,
            delete_scheduled_task,
            save_clipboard_item,
            get_clipboard_items,
            delete_clipboard_item,
            set_clipboard_item_pinned,
            clear_clipboard_items,
            set_user_avatar,
            get_user_avatar,
            save_custom_pet_asset,
            list_custom_pet_assets,
            get_custom_pet_asset,
            get_active_custom_pet_asset,
            set_active_custom_pet_asset,
            delete_custom_pet_asset,
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
            tts_play,
            tts_stop,
            tts_list_voices,
            tts_cache_stats,
            set_backend_type,
            get_backend_type,
            set_api_config,
            get_api_config,
            list_api_profiles,
            set_active_api_profile,
            delete_api_profile,
            test_api_connection,
            test_api_compatibility,
            list_api_models,
            confirm_tool,
            answer_question,
            send_tool_confirm_notification,
            check_claude_status,
            get_weather_config,
            set_weather_config,
            test_weather_config,
            get_weather,
            refresh_weather,
            check_and_send_weather,
            get_custom_system_prompt,
            set_custom_system_prompt,
            list_skills,
            save_skill,
            delete_skill,
            set_active_skill,
reset_builtin_skills,
            list_agents,
            save_agent,
            delete_agent,
            generate_agent_spec,
            spawn_subprocess,
            send_stdin,
            kill_subprocess,
            list_subprocesses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
