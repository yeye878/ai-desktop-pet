//! Agent 模型目录与思考强度（claude / codex / dsh）。
//!
//! 三个内置 agent 窗口的模型下拉不再依赖前端硬编码列表：这里读取本机 CLI 的
//! 真实模型目录（dsh 走 pet-runner `--list-models`，codex 走 `codex debug models`，
//! claude 读 `~/.claude/settings.json`），统一成前端契约结构，并按 slot 缓存 60 秒。
//!
//! 探测失败不是命令错误：目录照常返回，只是 `error` 非空且 `models` 为空，
//! 由 UI 决定是否提示并允许手输模型 id。

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 目录缓存有效期（前端打开选择器用缓存，点刷新传 force 绕过）。
const CACHE_TTL: Duration = Duration::from_secs(60);
/// 单次目录探测的总预算；超时也返回正常结构（error 文案）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentEffortOption {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentModelOption {
    /// 传给 CLI 的模型 id / 别名。
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 仅 dsh：provider 路由 id。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 仅 dsh：provider 展示名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_label: Option<String>,
    /// 后端默认思考强度 id；空串 = 无 / 未知。
    pub default_effort: String,
    /// 该模型可选的思考强度；空数组 = 不支持选择。
    pub efforts: Vec<AgentEffortOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentModelCatalog {
    pub slot: String,
    pub backend: String,
    /// 本机默认模型所在渠道（仅 dsh 有值；claude / codex 为空串）。
    pub local_provider: String,
    /// 本机配置的默认模型 id；空串 = 未知。
    pub local_model: String,
    /// 本机默认模型展示名；空串 = 未知。
    pub local_model_label: String,
    /// 本机默认思考强度 id；空串 = 未知 / 默认。
    pub local_effort: String,
    pub models: Vec<AgentModelOption>,
    /// 空串 = 正常；非空 = 探测失败提示。
    pub error: String,
    /// unix 毫秒。
    pub fetched_at: u64,
}

/// slot → 后端标识（与 `agents.backend` 一致）。
pub fn slot_backend(slot: &str) -> &'static str {
    match slot {
        "claude" => "claude_code",
        "codex" => "codex",
        "dsh" => "dsh",
        _ => "",
    }
}

impl AgentModelCatalog {
    /// 某个 slot 的空目录（探测失败 / 尚无数据时使用）。
    pub fn empty(slot: &str) -> Self {
        Self {
            slot: slot.to_string(),
            backend: slot_backend(slot).to_string(),
            local_provider: String::new(),
            local_model: String::new(),
            local_model_label: String::new(),
            local_effort: String::new(),
            models: Vec::new(),
            error: String::new(),
            fetched_at: now_ms(),
        }
    }

    /// 探测失败时的正常结构：models 为空、error 带文案。
    pub fn with_error(slot: &str, message: impl Into<String>) -> Self {
        let mut catalog = Self::empty(slot);
        catalog.error = message.into();
        catalog
    }

    /// 在目录里按 id 找模型展示名；找不到时回退 id 本身。
    pub fn label_for(&self, model: &str) -> String {
        self.models
            .iter()
            .find(|option| option.id == model)
            .map(|option| option.name.clone())
            .unwrap_or_else(|| model.to_string())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

struct CacheEntry {
    catalog: AgentModelCatalog,
    stored_at: Instant,
}

/// slot → 最近一次探测结果。
static CACHE: Mutex<Option<HashMap<&'static str, CacheEntry>>> = Mutex::new(None);

fn cache_key(slot: &str) -> Result<&'static str, String> {
    match slot.trim() {
        "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        "dsh" => Ok("dsh"),
        other => Err(format!("不支持的 agent 槽位：{other}")),
    }
}

fn cached(key: &str) -> Option<AgentModelCatalog> {
    let guard = CACHE.lock().ok()?;
    let entry = guard.as_ref()?.get(key)?;
    (entry.stored_at.elapsed() < CACHE_TTL).then(|| entry.catalog.clone())
}

fn store(key: &'static str, catalog: &AgentModelCatalog) {
    if let Ok(mut guard) = CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(
            key,
            CacheEntry {
                catalog: catalog.clone(),
                stored_at: Instant::now(),
            },
        );
    }
}

/// `get_agent_model_catalog` 的实现：60 秒缓存 + force 刷新 + 20 秒总超时。
///
/// slot 非法是调用方错误（返回 Err）；探测失败返回正常结构（error 非空）。
pub async fn get_catalog(slot: &str, force: bool) -> Result<AgentModelCatalog, String> {
    let key = cache_key(slot)?;
    if !force {
        if let Some(catalog) = cached(key) {
            return Ok(catalog);
        }
    }
    let catalog = probe(key).await;
    store(key, &catalog);
    Ok(catalog)
}

async fn probe(slot: &'static str) -> AgentModelCatalog {
    match tokio::time::timeout(PROBE_TIMEOUT, probe_inner(slot)).await {
        Ok(Ok(catalog)) => catalog,
        Ok(Err(message)) => AgentModelCatalog::with_error(slot, message),
        Err(_) => AgentModelCatalog::with_error(
            slot,
            "探测模型目录超时（20 秒），请检查本机 CLI 后重试。",
        ),
    }
}

async fn probe_inner(slot: &'static str) -> Result<AgentModelCatalog, String> {
    match slot {
        "claude" => Ok(claude_catalog()),
        "codex" => crate::codex::model_catalog().await,
        "dsh" => crate::dsh::model_catalog().await,
        other => Err(format!("不支持的 agent 槽位：{other}")),
    }
}

// ===== claude：无模型列表命令，读 ~/.claude/settings.json =====

/// Claude Code CLI 的 `--effort` 档位（固定 5 档）。
const CLAUDE_EFFORTS: [(&str, &str); 5] = [
    ("low", "低"),
    ("medium", "中"),
    ("high", "高"),
    ("xhigh", "极高"),
    ("max", "最高"),
];

/// 四个内置别名：settings 里 env.ANTHROPIC_DEFAULT_<ALIAS>_MODEL 给出本机映射。
const CLAUDE_ALIASES: [(&str, &str); 4] = [
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
    ("fable", "Fable"),
];

pub(crate) fn claude_efforts() -> Vec<AgentEffortOption> {
    CLAUDE_EFFORTS
        .iter()
        .map(|(id, name)| AgentEffortOption {
            id: (*id).to_string(),
            name: (*name).to_string(),
            description: None,
        })
        .collect()
}

fn non_empty_str(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// modelSettings[模型名].effortLevel。
fn effort_level_for(settings: &serde_json::Value, model: &str) -> String {
    settings["modelSettings"][model]["effortLevel"]
        .as_str()
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// 读 `~/.claude/settings.json` 并生成目录；文件缺失时只给别名，不报错。
pub fn claude_catalog() -> AgentModelCatalog {
    let Some(home) = dirs::home_dir() else {
        return AgentModelCatalog::with_error("claude", "无法定位用户主目录，读取不到 ~/.claude/settings.json。");
    };
    let path = home.join(".claude").join("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(settings) => claude_catalog_from_settings(&settings),
            Err(error) => {
                let mut catalog = claude_catalog_from_settings(&serde_json::Value::Null);
                catalog.error = format!("~/.claude/settings.json 不是合法 JSON：{error}");
                catalog
            }
        },
        Err(_) if !path.exists() => claude_catalog_from_settings(&serde_json::Value::Null),
        Err(error) => {
            let mut catalog = claude_catalog_from_settings(&serde_json::Value::Null);
            catalog.error = format!("无法读取 ~/.claude/settings.json：{error}");
            catalog
        }
    }
}

/// claude 的目录生成（纯函数，便于单测）。settings 为空对象/Null 时只返回别名。
pub(crate) fn claude_catalog_from_settings(settings: &serde_json::Value) -> AgentModelCatalog {
    let mut catalog = AgentModelCatalog::empty("claude");
    let efforts = claude_efforts();
    let env = &settings["env"];
    let env_model = non_empty_str(&env["ANTHROPIC_MODEL"]);
    let settings_model = non_empty_str(&settings["model"]);

    let mut models: Vec<AgentModelOption> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    // 1) 固定别名 + 本机映射说明。
    for (alias, name) in CLAUDE_ALIASES {
        let env_key = format!("ANTHROPIC_DEFAULT_{}_MODEL", alias.to_uppercase());
        let mapped = non_empty_str(&env[env_key.as_str()]);
        let default_effort = mapping_effort(settings, alias, mapped.as_deref());
        models.push(AgentModelOption {
            id: alias.to_string(),
            name: name.to_string(),
            description: mapped
                .as_ref()
                .map(|mapped| format!("本机映射到 {mapped}")),
            provider: None,
            provider_label: None,
            default_effort,
            efforts: efforts.clone(),
        });
        seen.push(alias.to_string());
    }

    // 2) modelSettings 里的完整模型名。
    if let Some(entries) = settings["modelSettings"].as_object() {
        let mut names: Vec<&String> = entries.keys().collect();
        names.sort();
        for model in names {
            if seen.iter().any(|seen| seen == model) {
                continue;
            }
            let effort = effort_level_for(settings, model);
            models.push(AgentModelOption {
                id: model.clone(),
                name: model.clone(),
                description: None,
                provider: None,
                provider_label: None,
                default_effort: effort,
                efforts: efforts.clone(),
            });
            seen.push(model.clone());
        }
    }

    // 3) env 里出现的具体模型 id（别名映射同样列出来，方便手选）。
    let env_models = [
        ("ANTHROPIC_MODEL", env_model.clone()),
        (
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            non_empty_str(&env["ANTHROPIC_DEFAULT_OPUS_MODEL"]),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            non_empty_str(&env["ANTHROPIC_DEFAULT_SONNET_MODEL"]),
        ),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            non_empty_str(&env["ANTHROPIC_DEFAULT_HAIKU_MODEL"]),
        ),
        (
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            non_empty_str(&env["ANTHROPIC_DEFAULT_FABLE_MODEL"]),
        ),
    ];
    for (env_key, env_value) in env_models {
        let Some(model) = env_value else {
            continue;
        };
        if seen.iter().any(|seen| *seen == model) {
            continue;
        }
        models.push(AgentModelOption {
            id: model.clone(),
            name: model.clone(),
            description: Some(format!("来自 env.{env_key}")),
            provider: None,
            provider_label: None,
            default_effort: effort_level_for(settings, &model),
            efforts: efforts.clone(),
        });
        seen.push(model);
    }

    // 本机默认模型：env.ANTHROPIC_MODEL 覆盖 settings.model；两者都没有就是空。
    let local_model = env_model
        .clone()
        .or_else(|| settings_model.clone())
        .unwrap_or_default();
    let local_effort = if local_model.is_empty() {
        String::new()
    } else if let Some(alias) = CLAUDE_ALIASES
        .iter()
        .map(|(alias, _)| *alias)
        .find(|alias| *alias == local_model)
    {
        let env_key = format!("ANTHROPIC_DEFAULT_{}_MODEL", alias.to_uppercase());
        let mapped = non_empty_str(&env[env_key.as_str()]);
        mapping_effort(settings, alias, mapped.as_deref())
    } else {
        effort_level_for(settings, &local_model)
    };

    catalog.local_model_label = if local_model.is_empty() {
        String::new()
    } else {
        catalog_label(&models, &local_model)
    };
    catalog.local_model = local_model;
    catalog.local_effort = local_effort;
    catalog.models = models;
    catalog
}

/// 别名 / 映射模型在 modelSettings 里配置的思考强度。
fn mapping_effort(
    settings: &serde_json::Value,
    alias: &str,
    mapped: Option<&str>,
) -> String {
    let direct = effort_level_for(settings, alias);
    if !direct.is_empty() {
        return direct;
    }
    mapped
        .map(|mapped| effort_level_for(settings, mapped))
        .filter(|effort| !effort.is_empty())
        .unwrap_or_default()
}

fn catalog_label(models: &[AgentModelOption], model: &str) -> String {
    models
        .iter()
        .find(|option| option.id == model)
        .map(|option| option.name.clone())
        .unwrap_or_else(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> serde_json::Value {
        serde_json::json!({
            "model": "opus",
            "env": {
                "ANTHROPIC_MODEL": "MiniMax-M3.1-Flash[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "MiniMax-M3[1M]",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "MiniMax-M3.1-Flash[1M]",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "MiniMax-M3.1-Flash[1M]"
            },
            "modelSettings": {
                "claude-opus-5-5": { "effortLevel": "xhigh" },
                "claude-sonnet-5-5": { "effortLevel": "high" }
            }
        })
    }

    #[test]
    fn claude_catalog_exposes_aliases_mapped_models_and_efforts() {
        let catalog = claude_catalog_from_settings(&settings());
        assert_eq!(catalog.slot, "claude");
        assert_eq!(catalog.backend, "claude_code");
        // 别名 + modelSettings 完整模型名 + env 具体模型 id（去重）。
        for id in ["opus", "sonnet", "haiku", "fable", "claude-opus-5-5", "claude-sonnet-5-5", "MiniMax-M3.1-Flash[1M]"] {
            assert!(catalog.models.iter().any(|option| option.id == id), "缺少模型 {id}: {:?}", catalog.models.iter().map(|option| option.id.clone()).collect::<Vec<_>>());
        }
        let opus = catalog.models.iter().find(|option| option.id == "opus").unwrap();
        assert_eq!(opus.name, "Opus");
        assert_eq!(
            opus.description.as_deref(),
            Some("本机映射到 MiniMax-M3[1M]"),
            "别名描述必须标注本机映射"
        );
        assert_eq!(opus.default_effort, "", "modelSettings 没有别名本身时跟随映射模型的设置");
        assert_eq!(
            opus.efforts.iter().map(|effort| effort.id.as_str()).collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(opus.efforts[1].name, "中");
        assert_eq!(opus.efforts[4].name, "最高");
        let opus_model = catalog.models.iter().find(|option| option.id == "claude-opus-5-5").unwrap();
        assert_eq!(opus_model.default_effort, "xhigh");
        // env.ANTHROPIC_MODEL 优先作为本机默认模型。
        assert_eq!(catalog.local_model, "MiniMax-M3.1-Flash[1M]");
        assert_eq!(catalog.local_model_label, "MiniMax-M3.1-Flash[1M]");
        assert_eq!(catalog.error, "");
    }

    #[test]
    fn missing_or_empty_claude_settings_still_offer_aliases() {
        for settings in [serde_json::Value::Null, serde_json::json!({}), serde_json::json!({ "env": {} })] {
            let catalog = claude_catalog_from_settings(&settings);
            let ids: Vec<&str> = catalog.models.iter().map(|option| option.id.as_str()).collect();
            assert_eq!(ids, vec!["opus", "sonnet", "haiku", "fable"]);
            assert_eq!(catalog.local_model, "");
            assert_eq!(catalog.local_provider, "", "claude 没有渠道概念");
            assert_eq!(catalog.local_model_label, "");
            assert_eq!(catalog.local_effort, "");
            assert_eq!(catalog.error, "", "settings 缺失不是错误");
        }
    }

    #[test]
    fn claude_local_effort_comes_from_settings_model_entry() {
        let catalog = claude_catalog_from_settings(&settings());
        // 默认模型是 env 里的 MiniMax-M3.1-Flash[1M]，settings.model 是 opus 别名。
        // 别名 opus 映射模型没有 effortLevel，因此回退到空。
        assert_eq!(catalog.local_effort, "");
        let with_effort = serde_json::json!({
            "env": { "ANTHROPIC_MODEL": "claude-opus-5-5" },
            "modelSettings": { "claude-opus-5-5": { "effortLevel": "max" } }
        });
        let catalog = claude_catalog_from_settings(&with_effort);
        assert_eq!(catalog.local_model, "claude-opus-5-5");
        assert_eq!(catalog.local_effort, "max");
    }

    #[test]
    fn only_known_slots_are_cached_by_name() {
        assert_eq!(cache_key("claude").unwrap(), "claude");
        assert_eq!(cache_key(" codex ").unwrap(), "codex");
        assert_eq!(cache_key("dsh").unwrap(), "dsh");
        assert!(cache_key("main").is_err());
        assert_eq!(slot_backend("claude"), "claude_code");
        assert_eq!(slot_backend("dsh"), "dsh");
        assert_eq!(slot_backend("main"), "");
        let catalog = AgentModelCatalog::with_error("codex", "boom");
        assert_eq!(catalog.error, "boom");
        assert!(catalog.models.is_empty());
        assert!(catalog.fetched_at > 0);
    }
}
