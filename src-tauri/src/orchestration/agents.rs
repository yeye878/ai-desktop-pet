//! 智能调度候选智能体目录与节点路由。
//! 统一纳管：主助理（main）、内置专职智能体（claude / codex / dsh）以及智能体工坊自定义智能体。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tauri::Manager;

use super::executor::backend_supports_read_only;

pub const MAIN_AGENT_ID: &str = "main";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateAgent {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub backend: String,
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_profile_id: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub is_main: bool,
    #[serde(default)]
    pub is_builtin: bool,
    pub available: bool,
    #[serde(default)]
    pub unavailable_reason: String,
}

impl CandidateAgent {
    pub fn synthetic(id: &str, name: &str, avatar: &str, backend: &str, is_main: bool) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            avatar: avatar.to_string(),
            backend: backend.to_string(),
            description: String::new(),
            system_prompt: String::new(),
            model: String::new(),
            api_profile_id: String::new(),
            allowed_tools: Vec::new(),
            is_main,
            is_builtin: is_main,
            available: true,
            unavailable_reason: String::new(),
        }
    }
}

fn backend_availability(
    backend: &str,
    api_profile_id: &str,
    has_default_direct: bool,
    profile_ids: &HashSet<String>,
    claude_ok: bool,
    codex_ok: bool,
    dsh_ok: bool,
) -> (bool, String) {
    match backend {
        "direct_api" => {
            if api_profile_id.trim().is_empty() {
                if has_default_direct {
                    (true, String::new())
                } else {
                    (false, "未配置默认直连 API".into())
                }
            } else if profile_ids.contains(api_profile_id) {
                (true, String::new())
            } else {
                (false, "绑定的 API 配置不存在".into())
            }
        }
        "claude_code" => {
            if claude_ok {
                (true, String::new())
            } else {
                (false, "本机未检测到 Claude Code CLI".into())
            }
        }
        "codex" => {
            if codex_ok {
                (true, String::new())
            } else {
                (false, "本机未检测到 Codex CLI".into())
            }
        }
        "dsh" => {
            if dsh_ok {
                (true, String::new())
            } else {
                (false, "本机未检测到 DeepSeek Harness CLI".into())
            }
        }
        other => (false, format!("不支持的后端：{other}")),
    }
}

/// 探测当前环境所有可用/不可用的候选智能体：主助理 + 内置 CLI + 自定义智能体。
pub async fn discover_candidates(
    app: &tauri::AppHandle,
    state: &crate::AppState,
) -> Vec<CandidateAgent> {
    let main_backend_raw = state.backend_type.lock().await.clone();
    let main_backend = if main_backend_raw.trim().is_empty() {
        "direct_api".to_string()
    } else {
        main_backend_raw
    };
    let has_default_direct = state.direct_api_config.lock().await.is_some();
    let (custom_agents, profiles) = {
        let db = state.db.lock().await;
        let custom = db.list_agents().unwrap_or_default();
        let profiles = crate::load_api_profiles(&db);
        (custom, profiles)
    };
    let profile_ids: HashSet<String> = profiles.into_iter().map(|p| p.id).collect();
    let claude_ok = crate::openclaw::is_cli_available(app.path().resource_dir().ok().as_deref());
    let codex_ok = crate::codex::is_cli_available();
    let dsh_ok = crate::dsh::is_cli_available();

    let (main_avail, main_reason) = backend_availability(
        &main_backend,
        "",
        has_default_direct,
        &profile_ids,
        claude_ok,
        codex_ok,
        dsh_ok,
    );
    let mut candidates = vec![CandidateAgent {
        id: MAIN_AGENT_ID.into(),
        name: "主助理".into(),
        avatar: "🐾".into(),
        backend: main_backend,
        description: "桌面主助理：具备记忆检索、技能、桌面应用、网页搜索、文档生成与日常通用能力，适合问答、写作、信息检索、桌面任务及简单编程。".into(),
        system_prompt: String::new(),
        model: String::new(),
        api_profile_id: String::new(),
        allowed_tools: Vec::new(),
        is_main: true,
        is_builtin: true,
        available: main_avail,
        unavailable_reason: main_reason,
    }];

    for agent in crate::collaboration::catalog(custom_agents) {
        let (available, reason) = backend_availability(
            &agent.backend,
            &agent.api_profile_id,
            has_default_direct,
            &profile_ids,
            claude_ok,
            codex_ok,
            dsh_ok,
        );
        candidates.push(CandidateAgent {
            id: agent.id,
            name: agent.name,
            avatar: agent.avatar,
            backend: agent.backend,
            description: agent.description,
            system_prompt: agent.system_prompt,
            model: agent.model,
            api_profile_id: agent.api_profile_id,
            allowed_tools: agent.allowed_tools,
            is_main: false,
            is_builtin: agent.is_builtin,
            available,
            unavailable_reason: reason,
        });
    }

    candidates
}

pub fn matches_agent_ref(agent: &CandidateAgent, target: &str) -> bool {
    let needle = target.trim().trim_start_matches('@');
    if needle.is_empty() {
        return false;
    }
    if agent.id == needle {
        return true;
    }
    if agent.is_main && (needle.eq_ignore_ascii_case("main") || needle == "主助理") {
        return true;
    }
    if agent.is_builtin {
        agent.name.eq_ignore_ascii_case(needle)
    } else {
        agent.name == needle
    }
}

pub fn resolve_mentioned_ids(candidates: &[CandidateAgent], mentions: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for mention in mentions {
        if let Some(found) = candidates.iter().find(|c| matches_agent_ref(c, mention)) {
            if seen.insert(found.id.clone()) {
                ids.push(found.id.clone());
            }
        }
    }
    ids
}

/// 生成供 Assessor / Planner 阅读的可用智能体花名册说明。
pub fn format_roster_for_prompt(candidates: &[CandidateAgent], mentioned_ids: &[String]) -> String {
    if candidates.is_empty() {
        return String::new();
    }
    let mut lines = vec!["## 可用执行智能体花名册".to_string()];
    for agent in candidates.iter().filter(|c| c.available) {
        let ro = if backend_supports_read_only(&agent.backend) {
            "支持只读"
        } else {
            "仅执行"
        };
        let desc = agent.description.trim();
        lines.push(format!(
            "- id=`{}` | 名称=`{}` | 后端={}({}) | 专长：{}",
            agent.id,
            agent.name,
            agent.backend,
            ro,
            if desc.is_empty() { "通用执行角色" } else { desc }
        ));
    }
    let unavailable: Vec<String> = candidates
        .iter()
        .filter(|c| !c.available)
        .map(|c| format!("{}({})", c.id, c.unavailable_reason))
        .collect();
    if !unavailable.is_empty() {
        lines.push(format!("（当前未就绪、勿分派：{}）", unavailable.join("；")));
    }
    if !mentioned_ids.is_empty() {
        let names: Vec<String> = mentioned_ids
            .iter()
            .filter_map(|id| candidates.iter().find(|c| &c.id == id))
            .map(|c| format!("{} (`{}`)", c.name, c.id))
            .collect();
        if !names.is_empty() {
            lines.push(format!(
                "【用户显式指定执行者】本任务用户 @ 指定了：{}。请优先把执行节点分配给该智能体（仍须遵守必需的独立审核规则）。",
                names.join("、")
            ));
        }
    }
    lines.join("\n")
}

/// 按节点偏好、用户显式 @ 约束、可用状态与兜底后端确定最终执行者。
pub fn resolve_assigned_agent(
    candidates: &[CandidateAgent],
    preferred_id: Option<&str>,
    mentioned_ids: &[String],
    fallback_backend: &str,
) -> CandidateAgent {
    if !mentioned_ids.is_empty() {
        if let Some(pref) = preferred_id {
            if let Some(found) = candidates
                .iter()
                .find(|c| c.available && mentioned_ids.contains(&c.id) && matches_agent_ref(c, pref))
            {
                return found.clone();
            }
        }
        if let Some(found) = candidates
            .iter()
            .find(|c| c.available && mentioned_ids.contains(&c.id))
        {
            return found.clone();
        }
    }
    if let Some(pref) = preferred_id {
        if let Some(found) = candidates
            .iter()
            .find(|c| c.available && matches_agent_ref(c, pref))
        {
            return found.clone();
        }
    }
    if let Some(main) = candidates.iter().find(|c| c.is_main && c.available) {
        return main.clone();
    }
    if let Some(by_backend) = candidates
        .iter()
        .find(|c| c.available && c.backend == fallback_backend)
    {
        return by_backend.clone();
    }
    if let Some(any) = candidates.iter().find(|c| c.available) {
        return any.clone();
    }
    CandidateAgent::synthetic(MAIN_AGENT_ID, "主助理", "🐾", fallback_backend, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_candidates() -> Vec<CandidateAgent> {
        vec![
            CandidateAgent::synthetic("main", "主助理", "🐾", "direct_api", true),
            CandidateAgent {
                id: "builtin-claude".into(),
                name: "claude".into(),
                avatar: "⌘".into(),
                backend: "claude_code".into(),
                description: "代码重构".into(),
                system_prompt: String::new(),
                model: String::new(),
                api_profile_id: String::new(),
                allowed_tools: vec![],
                is_main: false,
                is_builtin: true,
                available: true,
                unavailable_reason: String::new(),
            },
            CandidateAgent {
                id: "builtin-codex".into(),
                name: "codex".into(),
                avatar: "⌘".into(),
                backend: "codex".into(),
                description: "工程实现".into(),
                system_prompt: String::new(),
                model: String::new(),
                api_profile_id: String::new(),
                allowed_tools: vec![],
                is_main: false,
                is_builtin: true,
                available: false,
                unavailable_reason: "未安装".into(),
            },
            CandidateAgent {
                id: "builtin-dsh".into(),
                name: "dsh".into(),
                avatar: "⌘".into(),
                backend: "dsh".into(),
                description: "深度推理".into(),
                system_prompt: String::new(),
                model: String::new(),
                api_profile_id: String::new(),
                allowed_tools: vec![],
                is_main: false,
                is_builtin: true,
                available: true,
                unavailable_reason: String::new(),
            },
        ]
    }

    #[test]
    fn resolves_preferred_available_and_skips_unavailable() {
        let list = sample_candidates();
        let chosen = resolve_assigned_agent(&list, Some("builtin-dsh"), &[], "direct_api");
        assert_eq!(chosen.id, "builtin-dsh");
        assert_eq!(chosen.backend, "dsh");

        // 不可用的 codex 回退到可用的主助理，不会派给未就绪后端。
        let fallback = resolve_assigned_agent(&list, Some("builtin-codex"), &[], "direct_api");
        assert_eq!(fallback.id, "main");
    }

    #[test]
    fn explicit_mention_overrides_general_recommendation() {
        let list = sample_candidates();
        let mentioned = resolve_mentioned_ids(&list, &["@Claude".into()]);
        assert_eq!(mentioned, vec!["builtin-claude"]);
        let chosen = resolve_assigned_agent(&list, Some("main"), &mentioned, "direct_api");
        assert_eq!(chosen.id, "builtin-claude");
    }

    #[test]
    fn empty_catalog_returns_synthetic_fallback_for_tests() {
        let chosen = resolve_assigned_agent(&[], None, &[], "fake");
        assert_eq!(chosen.backend, "fake");
        assert!(chosen.is_main);
    }
}
