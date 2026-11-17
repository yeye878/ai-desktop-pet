//! 统一执行适配器（第 10.1 节）：AgentExecutor::execute(InvocationContext, TaskContract)。
//! 复用现有 API/CLI 实际请求与解析，剥离共享聊天与全局权限；每次调用显式配置。
//! 只读限制在调用端（工具定义/CLI --tools）与执行端（execute_tool guard）同时强制。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

use super::contracts::{
    BackendCapabilities, Capability, EvidenceRecord, InvocationResult, NodeKind, Usage,
};
use super::persistence::{new_id, unix_now};
use super::prompts;
use super::registry::InvocationEntry;

tokio::task_local! {
    static ORCH_READ_ONLY: bool;
    static ORCH_INVOCATION_ID: String;
    static ORCH_EVIDENCE: Option<EvidenceSink>;
}

pub type EvidenceSink = Arc<Mutex<Vec<EvidenceRecord>>>;

/// execute_tool 执行端只读强制查询。
pub fn orch_read_only() -> bool {
    ORCH_READ_ONLY.try_with(|value| *value).unwrap_or(false)
}

/// 字符边界安全截断，避免中文/多字节 UTF-8 截断 panic。
pub fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// 工具运行器证据：命令/参数摘要、输出、归属 invocation。执行者自述不算证据。
pub fn record_tool_evidence(name: &str, args: &serde_json::Value, output: &str) {
    let _ = ORCH_EVIDENCE.try_with(|sink| {
        let Some(sink) = sink else { return };
        let invocation_id = ORCH_INVOCATION_ID.try_with(|id| id.clone()).unwrap_or_default();
        let summary = truncate_utf8(
            &format!("{name} {}", serde_json::to_string(args).unwrap_or_default()),
            300,
        );
        let log_ref = truncate_utf8(output, 500);
        let record = EvidenceRecord {
            evidence_id: new_id("ev"),
            source_invocation_id: invocation_id,
            kind: if name == "run_command" { "command".into() } else { "tool".into() },
            summary,
            cwd: String::new(),
            exit_code: None,
            artifact_refs: vec![],
            log_ref,
            created_at: unix_now(),
        };
        if let Ok(mut list) = sink.lock() {
            list.push(record);
        }
    });
}

/// 运行时过程增量：供 `TurnPolicy` 与 CLI 适配器实时流式推送思考、正文、工具轨迹。
#[derive(Clone, Debug)]
pub enum ActivityDelta {
    Thinking(String),
    Text(String),
    Tool(crate::cli_activity::ToolEvent),
}

pub type ActivitySink = Arc<dyn Fn(ActivityDelta) + Send + Sync>;

/// 每次调用的显式策略：不依赖“最近一次会话”的全局兜底。
#[derive(Clone)]
pub struct TurnPolicy {
    pub read_only: bool,
    pub invocation_id: String,
    pub evidence: Option<EvidenceSink>,
    pub session_id: Option<String>,
    /// 智能调度内部节点调用屏蔽全局聊天窗流式事件，避免覆盖主输入框 loading 状态。
    pub emit_chat_events: bool,
    /// 可选过程流回调：将思考/过程文字/工具调用事件转发到智能调度节点流。
    pub activity_sink: Option<ActivitySink>,
}

impl Default for TurnPolicy {
    fn default() -> Self {
        Self {
            read_only: false,
            invocation_id: String::new(),
            evidence: None,
            session_id: None,
            emit_chat_events: true,
            activity_sink: None,
        }
    }
}

#[derive(Clone)]
pub struct InvocationContext {
    pub session_id: String,
    pub task_id: String,
    pub run_id: String,
    pub node_id: String,
    pub invocation_id: String,
    pub attempt: u32,
    pub role: NodeKind,
    /// direct_api / claude_code / codex / dsh
    pub backend: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub workspace: PathBuf,
    /// 权限上限：与用户授权/会话权限/角色能力取交集后的档位。
    pub permission: String,
    pub read_only: bool,
    /// 单调用防挂起护栏（技术超时，不是预算上限）。
    pub timeout_secs: u64,
    pub cancel: CancellationToken,
    pub entry: Option<Arc<InvocationEntry>>,
    pub policy_version: u32,
    pub prompt_version: u32,
    /// 分配给该节点的智能体身份（主助理 / 内置 CLI / 自定义角色）。
    pub agent: Option<super::agents::CandidateAgent>,
}

#[derive(Clone, Default)]
pub struct TaskContract {
    pub objective: String,
    /// 独立构建的上下文包：冻结需求/约束/输入快照，不含完整聊天与桌宠人设。
    pub context_pack: String,
    pub criteria: Vec<super::contracts::AcceptanceCriterion>,
    pub inputs: Vec<String>,
}

impl TaskContract {
    pub fn message(&self) -> String {
        let mut text = String::new();
        if !self.context_pack.is_empty() {
            text.push_str(&self.context_pack);
            text.push_str("\n\n");
        }
        text.push_str("## 任务\n");
        text.push_str(&self.objective);
        if !self.inputs.is_empty() {
            text.push_str("\n\n## 输入材料\n");
            for input in &self.inputs {
                text.push_str(input);
                text.push('\n');
            }
        }
        if !self.criteria.is_empty() {
            text.push_str("\n\n## 验收标准（冻结，不得删减必需项）\n");
            for criterion in &self.criteria {
                text.push_str(&format!(
                    "- [{}] {}（{}，验证方式 {:?}）\n",
                    criterion.id,
                    criterion.requirement,
                    if criterion.required { "必需" } else { "建议" },
                    criterion.verification
                ));
            }
        }
        text
    }
}

#[derive(Debug, Clone)]
pub struct ExecError {
    pub message: String,
    pub aborted: bool,
    /// 瞬时故障（网络/超时）：与质量返工分别计数，每节点最多重试 1 次。
    pub transient: bool,
}

impl ExecError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self { message: message.into(), aborted: false, transient: false }
    }
    pub fn transient(message: impl Into<String>) -> Self {
        Self { message: message.into(), aborted: false, transient: true }
    }
    pub fn aborted(message: impl Into<String>) -> Self {
        Self { message: message.into(), aborted: true, transient: false }
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self { message: message.into(), aborted: false, transient: false }
    }
}

pub type ExecFuture<'a> = futures_util::future::BoxFuture<'a, Result<InvocationResult, ExecError>>;

pub trait AgentExecutor: Send + Sync {
    fn execute<'a>(&'a self, ctx: &'a InvocationContext, contract: &'a TaskContract) -> ExecFuture<'a>;
}

/// 四类后端能力矩阵（第 10.1 节）。未知标 unknown；路由前过滤不满足能力的候选。
pub fn capability_matrix() -> Vec<BackendCapabilities> {
    vec![
        BackendCapabilities {
            backend: "direct_api".into(),
            structured_output: Capability::Supported,
            workspace_override: Capability::Unsupported,
            read_only_enforcement: Capability::Supported,
            cancel: Capability::Supported,
            usage_reporting: Capability::Unknown,
            resume: Capability::Unsupported,
        },
        BackendCapabilities {
            backend: "claude_code".into(),
            structured_output: Capability::Supported,
            workspace_override: Capability::Supported,
            read_only_enforcement: Capability::Supported,
            cancel: Capability::Supported,
            usage_reporting: Capability::Unknown,
            resume: Capability::Supported,
        },
        BackendCapabilities {
            backend: "codex".into(),
            structured_output: Capability::Unknown,
            workspace_override: Capability::Supported,
            read_only_enforcement: Capability::Unsupported,
            cancel: Capability::Supported,
            usage_reporting: Capability::Unknown,
            resume: Capability::Supported,
        },
        BackendCapabilities {
            backend: "dsh".into(),
            structured_output: Capability::Unknown,
            workspace_override: Capability::Supported,
            read_only_enforcement: Capability::Unsupported,
            cancel: Capability::Supported,
            usage_reporting: Capability::Unknown,
            resume: Capability::Supported,
        },
    ]
}

pub fn backend_supports_read_only(backend: &str) -> bool {
    capability_matrix()
        .into_iter()
        .find(|c| c.backend == backend)
        .is_some_and(|c| c.read_only_enforcement == Capability::Supported)
}

/// 提取最后一个有效围栏 JSON 对象（支持 ```json / ```JSON / 无标签 ```）的起止字节区间及值。
fn extract_last_fenced_json_span(text: &str) -> Option<(usize, usize, serde_json::Value)> {
    let mut last = None;
    let mut offset = 0usize;
    while let Some(rel_start) = text[offset..].find("```") {
        let start = offset + rel_start;
        let after_ticks = &text[start + 3..];
        let Some(newline_rel) = after_ticks.find('\n') else {
            break;
        };
        let lang = after_ticks[..newline_rel].trim().to_ascii_lowercase();
        let body_start = start + 3 + newline_rel + 1;
        let Some(end_rel) = text[body_start..].find("```") else {
            break;
        };
        let body = text[body_start..body_start + end_rel].trim();
        let end = body_start + end_rel + 3;
        if lang.is_empty() || lang == "json" {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
                if value.is_object() {
                    last = Some((start, end, value));
                }
            }
        }
        offset = end;
    }
    last
}

/// 约定结果封套：最后一个有效 JSON 围栏；无围栏时尝试整段或末尾平衡 JSON 对象。解析失败交给上层受限修复。
pub fn extract_json_envelope(text: &str) -> Option<serde_json::Value> {
    if let Some((_, _, value)) = extract_last_fenced_json_span(text) {
        return Some(value);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        if value.is_object() {
            return Some(value);
        }
    }
    // 容忍“说明文字 + 裸 JSON 对象”末尾输出，但不对纯文本关键字（如 PASS）放行。
    let trimmed = text.trim_end();
    if trimmed.ends_with('}') {
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        let mut start_candidates = Vec::new();
        for (idx, ch) in trimmed.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '{' => {
                    if depth == 0 {
                        start_candidates.push(idx);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        depth = 0;
                    }
                }
                _ => {}
            }
        }
        if let Some(&start) = start_candidates.last() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..]) {
                if value.is_object() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// 剥离 worker/integrator 末尾附加的结构化元信息封套，保留面向用户的答复正文；不误删普通 JSON 代码示例。
pub fn strip_worker_envelope(text: &str) -> String {
    let Some((start, end, value)) = extract_last_fenced_json_span(text) else {
        return text.trim().to_string();
    };
    let is_envelope = value.as_object().is_some_and(|map| {
        (map.contains_key("summary") || map.contains_key("artifacts"))
            && (map.contains_key("evidence_notes")
                || map.contains_key("limitations")
                || map.contains_key("request_subtask")
                || map.contains_key("artifacts"))
    });
    if !is_envelope {
        return text.trim().to_string();
    }
    let before = text[..start].trim_end();
    let after = text[end..].trim_start();
    let combined = match (before.is_empty(), after.is_empty()) {
        (false, false) => format!("{before}\n\n{after}"),
        (false, true) => before.to_string(),
        (true, false) => after.to_string(),
        (true, true) => String::new(),
    };
    if !combined.trim().is_empty() {
        return combined;
    }
    value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| text.trim().to_string())
}

pub struct RuntimeExecutor {
    pub app: tauri::AppHandle,
}

impl AgentExecutor for RuntimeExecutor {
    fn execute<'a>(&'a self, ctx: &'a InvocationContext, contract: &'a TaskContract) -> ExecFuture<'a> {
        Box::pin(async move {
            let duration = std::time::Duration::from_secs(ctx.timeout_secs.max(30));
            match tokio::time::timeout(duration, self.execute_inner(ctx, contract)).await {
                Ok(result) => result,
                Err(_) => Err(ExecError::transient(format!(
                    "节点 {} 超过 {}s 无完成，按瞬时故障处理",
                    ctx.node_id, ctx.timeout_secs
                ))),
            }
        })
    }
}

fn emit_activity_event(
    app: &tauri::AppHandle,
    ctx: &InvocationContext,
    kind: &str,
    text: Option<&str>,
    tool: Option<&crate::cli_activity::ToolEvent>,
) {
    if ctx.entry.as_ref().is_some_and(|entry| !entry.is_open()) {
        return;
    }
    let agent = ctx.agent.as_ref();
    let payload = serde_json::json!({
        "session_id": ctx.session_id,
        "run_id": ctx.run_id,
        "node_id": ctx.node_id,
        "role_kind": ctx.role.as_str(),
        "invocation_id": ctx.invocation_id,
        "attempt": ctx.attempt,
        "agent_id": agent.map(|a| a.id.as_str()).unwrap_or(super::agents::MAIN_AGENT_ID),
        "agent_name": agent.map(|a| a.name.as_str()).unwrap_or("主助理"),
        "agent_avatar": agent.map(|a| a.avatar.as_str()).unwrap_or("🐾"),
        "backend": ctx.backend,
        "kind": kind,
        "text": text,
        "tool": tool,
    });
    let _ = app.emit("orchestration-activity", payload);
}

fn should_stream_text(role: NodeKind) -> bool {
    !role.read_only() && role != NodeKind::Plan
}

fn to_storage_agent(agent: &super::agents::CandidateAgent) -> crate::storage::Agent {
    crate::storage::Agent {
        id: agent.id.clone(),
        name: agent.name.clone(),
        avatar: agent.avatar.clone(),
        description: agent.description.clone(),
        system_prompt: agent.system_prompt.clone(),
        model: agent.model.clone(),
        backend: agent.backend.clone(),
        api_profile_id: agent.api_profile_id.clone(),
        is_builtin: agent.is_builtin,
        allowed_tools: agent.allowed_tools.clone(),
        created_at: 0,
        updated_at: 0,
    }
}

fn push_cli_tool_evidence(evidence: &EvidenceSink, tool: &crate::cli_activity::ToolEvent) {
    if tool.status != "completed" && tool.status != "failed" {
        return;
    }
    let is_cmd = matches!(
        tool.tool_name.as_str(),
        "Bash" | "run_command" | "command_execution" | "exec_command"
    );
    if let Ok(mut list) = evidence.lock() {
        list.push(EvidenceRecord {
            evidence_id: new_id("ev"),
            source_invocation_id: String::new(),
            kind: if is_cmd { "command".into() } else { "tool".into() },
            summary: tool.summary.clone(),
            cwd: String::new(),
            exit_code: tool.exit_code.map(|code| code as i32),
            artifact_refs: vec![],
            log_ref: tool.output.clone().unwrap_or_default(),
            created_at: unix_now(),
        });
    }
}

impl RuntimeExecutor {
    async fn execute_inner(
        &self,
        ctx: &InvocationContext,
        contract: &TaskContract,
    ) -> Result<InvocationResult, ExecError> {
        if ctx.read_only && !backend_supports_read_only(&ctx.backend) {
            return Err(ExecError::unsupported(format!(
                "后端 {} 不支持只读强制，不能承载 {:?}",
                ctx.backend, ctx.role
            )));
        }
        emit_activity_event(&self.app, ctx, "started", None, None);
        let outcome = match ctx.backend.as_str() {
            "direct_api" => self.execute_direct_api(ctx, contract).await,
            "claude_code" => self.execute_claude(ctx, contract).await,
            "codex" => self.execute_codex(ctx, contract).await,
            "dsh" => self.execute_dsh(ctx, contract).await,
            other => Err(ExecError::unsupported(format!(
                "后端 {other} 暂不承载智能调度调用（能力矩阵见 executor::capability_matrix）"
            ))),
        };
        match &outcome {
            Ok(_) => emit_activity_event(&self.app, ctx, "completed", None, None),
            Err(err) => emit_activity_event(&self.app, ctx, "failed", Some(&err.message), None),
        }
        outcome
    }

    async fn build_direct_api_context(
        &self,
        ctx: &InvocationContext,
    ) -> Result<(crate::direct_api::DirectApiConfig, String, std::collections::HashSet<String>), ExecError> {
        let state = self.app.state::<crate::AppState>();
        let custom_profile_id = ctx
            .agent
            .as_ref()
            .filter(|a| !a.is_main && !a.api_profile_id.trim().is_empty())
            .map(|a| a.api_profile_id.clone());
        let mut config = if let Some(profile_id) = custom_profile_id {
            let db = state.db.lock().await;
            crate::load_api_profiles(&db)
                .into_iter()
                .find(|p| p.id == profile_id)
                .ok_or_else(|| ExecError::unsupported("该智能体绑定的 API 配置已不存在"))?
        } else {
            state
                .direct_api_config
                .lock()
                .await
                .clone()
                .ok_or_else(|| ExecError::unsupported("直连 API 未配置，无法承载该节点"))?
        };
        let agent_model = ctx
            .agent
            .as_ref()
            .map(|a| a.model.trim())
            .filter(|m| !m.is_empty());
        if let Some(model) = agent_model {
            config.model = model.to_string();
        } else if let Some(model) = ctx.model.as_ref().filter(|m| !m.trim().is_empty()) {
            config.model = model.clone();
        }

        // 只读阶段（评估/审核）不继承执行者临时授权，也不注入桌宠人设或聊天历史。
        if ctx.read_only || ctx.role == NodeKind::Plan {
            config.auto_approved_tools = Vec::new();
            return Ok((config, role_prompt(ctx.role), std::collections::HashSet::new()));
        }

        let personality = state.personality.lock().await.clone();
        let profession = state.profession.lock().await.clone();
        let (all_skills, active_skill, custom_prompt, memory_block) = {
            let db = state.db.lock().await;
            (
                db.list_skills().unwrap_or_default(),
                db.get_active_skill().unwrap_or(None),
                db.get_setting("custom_system_prompt")
                    .unwrap_or(None)
                    .unwrap_or_default(),
                crate::memory_context_block(&db),
            )
        };
        let effective_skill = crate::resolve_skill_for_message(active_skill.as_ref());
        let mut system = crate::openclaw::build_system_prompt(&personality, &profession);
        system = crate::attach_runtime_identity_prompt(system, &config);
        system = crate::attach_execution_mode_prompt(system, &config);
        system = crate::attach_word_and_ask_prompt(system);
        system = crate::attach_agent_factory_prompt(system);
        system = crate::attach_coding_prompt(system);
        system = crate::attach_computer_use_prompt(system);
        system = crate::attach_weather_prompt(system);
        system = crate::attach_search_prompt(system);
        system = crate::attach_memory_prompt(system, &memory_block);
        system = crate::attach_registered_apps_prompt(system, &state).await;
        system = crate::attach_custom_prompt(system, &custom_prompt);
        system = crate::attach_skill_prompt(system, active_skill.as_ref(), &all_skills, true).await;

        let mut agents_for_grants = Vec::new();
        if let Some(agent) = ctx.agent.as_ref().filter(|a| !a.is_main) {
            let storage_agent = to_storage_agent(agent);
            system.push_str("\n\n");
            system.push_str(&crate::collaboration::role_section(&storage_agent));
            agents_for_grants.push(storage_agent);
        }
        system.push_str("\n\n");
        system.push_str(&role_prompt(ctx.role));

        let grants = crate::collect_turn_tool_grants(&agents_for_grants, effective_skill.as_ref());
        Ok((config, system, grants))
    }

    async fn build_cli_system_prompt(&self, ctx: &InvocationContext) -> String {
        if ctx.read_only || ctx.role == NodeKind::Plan {
            return role_prompt(ctx.role);
        }
        let state = self.app.state::<crate::AppState>();
        let personality = state.personality.lock().await.clone();
        let profession = state.profession.lock().await.clone();
        let (all_skills, active_skill, custom_prompt, memory_block) = {
            let db = state.db.lock().await;
            (
                db.list_skills().unwrap_or_default(),
                db.get_active_skill().unwrap_or(None),
                db.get_setting("custom_system_prompt")
                    .unwrap_or(None)
                    .unwrap_or_default(),
                crate::memory_context_block(&db),
            )
        };
        let mut system = crate::openclaw::build_system_prompt(&personality, &profession);
        system = crate::attach_cli_memory_prompt(system, &memory_block, &state);
        system = crate::attach_custom_prompt(system, &custom_prompt);
        system = crate::attach_skill_prompt(system, active_skill.as_ref(), &all_skills, false).await;
        if let Some(agent) = ctx.agent.as_ref().filter(|a| !a.is_main) {
            let storage_agent = to_storage_agent(agent);
            system.push_str("\n\n");
            system.push_str(&crate::collaboration::role_section(&storage_agent));
        }
        system.push_str("\n\n");
        system.push_str(&role_prompt(ctx.role));
        system
    }

    fn make_activity_sink(&self, ctx: &InvocationContext) -> ActivitySink {
        let app = self.app.clone();
        let ctx_clone = ctx.clone();
        let stream_text = should_stream_text(ctx.role);
        Arc::new(move |delta: ActivityDelta| match delta {
            ActivityDelta::Thinking(t) => emit_activity_event(&app, &ctx_clone, "thinking", Some(&t), None),
            ActivityDelta::Text(t) => {
                if stream_text {
                    emit_activity_event(&app, &ctx_clone, "text", Some(&t), None);
                }
            }
            ActivityDelta::Tool(tool) => emit_activity_event(&app, &ctx_clone, "tool", None, Some(&tool)),
        })
    }

    async fn execute_direct_api(
        &self,
        ctx: &InvocationContext,
        contract: &TaskContract,
    ) -> Result<InvocationResult, ExecError> {
        let (config, system, grants) = self.build_direct_api_context(ctx).await?;
        let evidence: EvidenceSink = Arc::new(Mutex::new(Vec::new()));
        let policy = TurnPolicy {
            read_only: ctx.read_only,
            invocation_id: ctx.invocation_id.clone(),
            evidence: Some(evidence.clone()),
            session_id: Some(ctx.session_id.clone()),
            emit_chat_events: false,
            activity_sink: Some(self.make_activity_sink(ctx)),
        };
        let result = ORCH_READ_ONLY
            .scope(
                ctx.read_only,
                ORCH_INVOCATION_ID.scope(
                    ctx.invocation_id.clone(),
                    ORCH_EVIDENCE.scope(
                        Some(evidence.clone()),
                        crate::direct_api::execute_direct_api_turn_with_policy(
                            self.app.clone(),
                            contract.message(),
                            config,
                            system,
                            Vec::new(), // 独立上下文：不读共享聊天
                            Vec::new(),
                            None,
                            None,
                            grants,
                            ctx.cancel.clone(),
                            false,
                            false, // 智能调度内禁止自由 delegate_to_agent
                            &policy,
                        ),
                    ),
                ),
            )
            .await;
        let output = match result {
            Ok(output) => output,
            Err(error) if error.aborted => return Err(ExecError::aborted(error.message)),
            Err(error) => return Err(ExecError::transient(error.message)),
        };
        let evidence = evidence.lock().map(|list| list.clone()).unwrap_or_default();
        Ok(InvocationResult {
            invocation_id: ctx.invocation_id.clone(),
            node_id: ctx.node_id.clone(),
            attempt: ctx.attempt,
            text: output.text.clone(),
            structured: extract_json_envelope(&output.text),
            usage: Usage { model_requests: None, input_tokens: None, output_tokens: None, tool_calls: evidence.len() as u64 },
            evidence,
            error: None,
            aborted: false,
        })
    }

    fn resolved_model<'a>(&'a self, ctx: &'a InvocationContext) -> Option<&'a str> {
        ctx.agent
            .as_ref()
            .map(|a| a.model.trim())
            .filter(|m| !m.is_empty())
            .or_else(|| ctx.model.as_deref().map(str::trim).filter(|m| !m.is_empty()))
    }

    async fn resolve_cli_permission(&self, ctx: &InvocationContext, backend: &str) -> String {
        if !ctx.permission.trim().is_empty() {
            return ctx.permission.clone();
        }
        if ctx.read_only && backend == "claude_code" {
            return "plan".to_string();
        }
        let key = match backend {
            "codex" => crate::cli_permission::CODEX_SANDBOX_MODE_KEY,
            "dsh" => crate::cli_permission::DSH_PERMISSION_MODE_KEY,
            _ => crate::cli_permission::CLAUDE_PERMISSION_MODE_KEY,
        };
        crate::cli_permission::read_setting(&self.app, key).await
    }

    async fn execute_claude(
        &self,
        ctx: &InvocationContext,
        contract: &TaskContract,
    ) -> Result<InvocationResult, ExecError> {
        let read_only_tools = vec![
            "Read".to_string(),
            "Glob".to_string(),
            "Grep".to_string(),
            "WebSearch".to_string(),
            "WebFetch".to_string(),
        ];
        let permission = self.resolve_cli_permission(ctx, "claude_code").await;
        let system = self.build_cli_system_prompt(ctx).await;
        let options = crate::openclaw::SpawnOptions {
            session: None, // 每次调用独立上下文，不续接旧会话
            owner_session_id: Some(ctx.session_id.clone()),
            model: self.resolved_model(ctx).map(str::to_string),
            effort: ctx.effort.clone(),
            permission,
            workspace: Some(ctx.workspace.clone()),
            tools_whitelist: if ctx.read_only { Some(read_only_tools) } else { None },
            use_permission_bridge: !ctx.read_only,
        };
        let mut child = crate::openclaw::ClaudeAdapter::new()
            .spawn_streaming_with_options(&self.app, &contract.message(), &system, &options)
            .await
            .map_err(|e| ExecError::transient(format!("启动 Claude CLI 失败：{e}")))?;
        self.drive_cli_stream(ctx, &mut child, "claude_code", "").await
    }

    async fn execute_codex(
        &self,
        ctx: &InvocationContext,
        contract: &TaskContract,
    ) -> Result<InvocationResult, ExecError> {
        let permission = self.resolve_cli_permission(ctx, "codex").await;
        let system = self.build_cli_system_prompt(ctx).await;
        let mut child = crate::codex::CodexAdapter::default()
            .spawn_streaming_in_workspace(
                &self.app,
                &contract.message(),
                &system,
                Some(&ctx.workspace),
                Some(&ctx.session_id),
                None,
                self.resolved_model(ctx),
                ctx.effort.as_deref(),
                &permission,
            )
            .await
            .map_err(|e| ExecError::transient(format!("启动 Codex CLI 失败：{e}")))?;
        self.drive_cli_stream(ctx, &mut child, "codex", &permission).await
    }

    async fn execute_dsh(
        &self,
        ctx: &InvocationContext,
        contract: &TaskContract,
    ) -> Result<InvocationResult, ExecError> {
        let permission = self.resolve_cli_permission(ctx, "dsh").await;
        let system = self.build_cli_system_prompt(ctx).await;
        let mut child = crate::dsh::DshAdapter::default()
            .spawn_streaming_in_workspace(
                &self.app,
                &contract.message(),
                &[],
                &system,
                Some(&ctx.workspace),
                Some(&ctx.session_id),
                None,
                self.resolved_model(ctx),
                None,
                ctx.effort.as_deref(),
                &permission,
            )
            .await
            .map_err(|e| ExecError::transient(format!("启动 DeepSeek Harness CLI 失败：{e}")))?;
        self.drive_cli_stream(ctx, &mut child, "dsh", &permission).await
    }

    async fn drive_cli_stream(
        &self,
        ctx: &InvocationContext,
        child: &mut tokio::process::Child,
        backend: &str,
        permission: &str,
    ) -> Result<InvocationResult, ExecError> {
        let pid = child.id();
        if ctx.cancel.is_cancelled() {
            if let Some(pid) = pid {
                crate::kill_process_tree(pid);
            }
            let _ = child.wait().await;
            return Err(ExecError::aborted("已停止"));
        }
        if let (Some(pid), Some(entry)) = (pid, ctx.entry.as_ref()) {
            entry.add_pid(pid);
        }
        let evidence: EvidenceSink = Arc::new(Mutex::new(Vec::new()));
        let text = Arc::new(Mutex::new(String::new()));
        let sink = self.make_activity_sink(ctx);

        let think_sink = sink.clone();
        let text_sink = sink.clone();
        let text_buf = text.clone();
        let tool_sink = sink;
        let ev_sink = evidence.clone();
        let cancel = ctx.cancel.clone();

        let on_thinking = move |delta: &str| {
            if !cancel.is_cancelled() {
                think_sink(ActivityDelta::Thinking(delta.to_string()));
            }
        };
        let cancel_text = ctx.cancel.clone();
        let on_text = move |delta: &str| {
            if let Ok(mut buf) = text_buf.lock() {
                buf.push_str(delta);
            }
            if !cancel_text.is_cancelled() {
                text_sink(ActivityDelta::Text(delta.to_string()));
            }
        };
        let cancel_tool = ctx.cancel.clone();
        let on_tool = move |tool: &crate::cli_activity::ToolEvent| {
            if !cancel_tool.is_cancelled() {
                tool_sink(ActivityDelta::Tool(tool.clone()));
            }
            push_cli_tool_evidence(&ev_sink, tool);
        };

        let read = async {
            match backend {
                "codex" => {
                    crate::codex::read_stream_with_activity(child, on_thinking, on_text, on_tool).await
                }
                "dsh" => {
                    let approvals: Option<Box<dyn crate::dsh::ApprovalAnswerer>> = Some(Box::new(
                        crate::dsh::UiApproval::for_session(
                            self.app.clone(),
                            ctx.cancel.clone(),
                            ctx.session_id.clone(),
                            crate::cli_permission::dsh_permission(permission).auto_approve,
                        ),
                    ));
                    crate::dsh::read_stream_with_activity(
                        child,
                        on_thinking,
                        on_text,
                        on_tool,
                        approvals,
                    )
                    .await
                }
                _ => {
                    crate::openclaw::read_stream_with_activity(child, on_thinking, on_text, on_tool)
                        .await
                }
            }
        };
        let parsed = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => None,
            result = read => Some(result),
        };
        if parsed.is_none() {
            if let Some(pid) = pid {
                crate::kill_process_tree(pid);
            }
        }
        let _ = child.wait().await;
        if ctx.cancel.is_cancelled() {
            return Err(ExecError::aborted("已停止"));
        }
        let state = parsed
            .ok_or_else(|| ExecError::aborted("已停止"))?
            .map_err(|e| ExecError::transient(format!("读取 CLI 流失败：{e}")))?;
        if state.is_error {
            return Err(ExecError::failed(
                state.error_msg.unwrap_or_else(|| "CLI 未正常完成".into()),
            ));
        }
        let mut evidence = evidence.lock().map(|list| list.clone()).unwrap_or_default();
        for record in &mut evidence {
            record.source_invocation_id = ctx.invocation_id.clone();
        }
        let text = text.lock().map(|buf| buf.clone()).unwrap_or_default();
        let full_text = if text.is_empty() { state.full_text.clone() } else { text };
        Ok(InvocationResult {
            invocation_id: ctx.invocation_id.clone(),
            node_id: ctx.node_id.clone(),
            attempt: ctx.attempt,
            text: full_text.clone(),
            structured: extract_json_envelope(&full_text),
            usage: Usage {
                model_requests: None,
                input_tokens: None,
                output_tokens: None,
                tool_calls: evidence.len() as u64,
            },
            evidence,
            error: None,
            aborted: false,
        })
    }
}

pub fn role_prompt(role: NodeKind) -> String {
    match role {
        NodeKind::Assess => prompts::assessor(),
        NodeKind::Plan => prompts::planner(),
        NodeKind::Integrate => prompts::integrator(),
        NodeKind::Review | NodeKind::ReviewAction => prompts::reviewer(),
        _ => prompts::worker(),
    }
}

/// 假执行器：状态机/门禁测试用（第 15 节：先用假模型测状态机，不烧真实模型）。
pub struct FakeExecutor {
    queue: Mutex<VecDeque<(Option<NodeKind>, Result<InvocationResult, ExecError>)>>,
    pub calls: Mutex<Vec<(NodeKind, String)>>,
    /// 上下文感知响应：模拟真实模型从上下文包读取版本/哈希后填写封套。
    /// 返回 None 表示不接管该调用，回退队列。
    pub generative:
        Mutex<Option<Box<dyn Fn(&InvocationContext, &TaskContract) -> Option<Result<InvocationResult, ExecError>> + Send>>>,
    /// 异步版本：可模拟耗时（并发时序测试）。
    pub generative_async: Mutex<
        Option<
            Box<
                dyn Fn(InvocationContext, TaskContract) -> futures_util::future::BoxFuture<'static, Option<Result<InvocationResult, ExecError>>>
                    + Send,
            >,
        >,
    >,
}

impl FakeExecutor {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            generative: Mutex::new(None),
            generative_async: Mutex::new(None),
        }
    }

    pub fn push(&self, role: Option<NodeKind>, result: Result<InvocationResult, ExecError>) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push_back((role, result));
        }
    }

    pub fn push_text(&self, role: NodeKind, text: &str, structured: Option<serde_json::Value>) {
        self.push(
            Some(role),
            Ok(InvocationResult {
                invocation_id: String::new(),
                node_id: String::new(),
                attempt: 1,
                text: text.to_string(),
                structured,
                usage: Usage::default(),
                evidence: vec![],
                error: None,
                aborted: false,
            }),
        );
    }
}

impl Default for FakeExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentExecutor for FakeExecutor {
    fn execute<'a>(&'a self, ctx: &'a InvocationContext, contract: &'a TaskContract) -> ExecFuture<'a> {
        Box::pin(async move {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push((ctx.role, contract.objective.clone()));
            }
            let async_hook = self
                .generative_async
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(|f| f(ctx.clone(), contract.clone())));
            if let Some(future) = async_hook {
                if let Some(result) = future.await {
                    return result.map(|mut r| {
                        r.invocation_id = ctx.invocation_id.clone();
                        r.node_id = ctx.node_id.clone();
                        r.attempt = ctx.attempt;
                        r
                    });
                }
            }
            let hook = self.generative.lock().ok().and_then(|guard| guard.as_ref().and_then(|f| f(ctx, contract)));
            if let Some(result) = hook {
                return result.map(|mut r| {
                    r.invocation_id = ctx.invocation_id.clone();
                    r.node_id = ctx.node_id.clone();
                    r.attempt = ctx.attempt;
                    r
                });
            }
            let next = self.queue.lock().ok().and_then(|mut queue| {
                let position = queue
                    .iter()
                    .position(|(role, _)| role.is_none() || *role == Some(ctx.role));
                position.map(|index| queue.remove(index).unwrap().1)
            });
            match next {
                Some(Ok(mut result)) => {
                    result.invocation_id = ctx.invocation_id.clone();
                    result.node_id = ctx.node_id.clone();
                    result.attempt = ctx.attempt;
                    Ok(result)
                }
                Some(Err(error)) => Err(error),
                None => Err(ExecError::failed(format!("FakeExecutor 没有为 {:?} 准备响应", ctx.role))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_extracts_last_json_fence_not_scanned_pass() {
        let text = "前面说 PASS 也没用\n```json\n{\"verdict\":\"rework\"}\n```\n补充\n```json\n{\"verdict\":\"pass\",\"checks\":[]}\n```";
        let value = extract_json_envelope(text).unwrap();
        assert_eq!(value["verdict"], "pass");
        assert!(extract_json_envelope("全部测试通过 PASS").is_none());
        assert!(extract_json_envelope("```json\n{invalid}\n```").is_none());
    }

    #[test]
    fn read_only_backends_filtered_by_matrix() {
        assert!(backend_supports_read_only("direct_api"));
        assert!(backend_supports_read_only("claude_code"));
        assert!(!backend_supports_read_only("codex"));
        assert!(!backend_supports_read_only("dsh"));
    }

    #[test]
    fn fake_executor_matches_role_and_records_calls() {
        let executor = FakeExecutor::new();
        executor.push_text(NodeKind::Assess, "评估", Some(serde_json::json!({"schema_version":1})));
        executor.push_text(NodeKind::Work, "交付", None);
        let ctx = InvocationContext {
            session_id: "s".into(),
            task_id: "t".into(),
            run_id: "r".into(),
            node_id: "n".into(),
            invocation_id: "inv-1".into(),
            attempt: 1,
            role: NodeKind::Work,
            backend: "fake".into(),
            model: None,
            effort: None,
            workspace: PathBuf::from("."),
            permission: String::new(),
            read_only: false,
            timeout_secs: 60,
            cancel: CancellationToken::new(),
            entry: None,
            policy_version: 1,
            prompt_version: 1,
            agent: None,
        };
        let contract = TaskContract { objective: "做".into(), ..Default::default() };
        let result = futures_util::FutureExt::now_or_never(executor.execute(&ctx, &contract)).unwrap().unwrap();
        assert_eq!(result.text, "交付");
        assert_eq!(result.invocation_id, "inv-1");
        assert_eq!(executor.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn utf8_truncation_never_panics_on_chinese_boundary() {
        let text = "中文字符截断安全测试".repeat(20);
        let cut = truncate_utf8(&text, 300);
        assert!(cut.len() <= 300);
        assert!(text.starts_with(&cut));
    }

    #[test]
    fn envelope_handles_uppercase_untagged_and_trailing_prose_json() {
        let upper = "说明\n```JSON\n{\"ok\":1}\n```";
        assert_eq!(extract_json_envelope(upper).unwrap()["ok"], 1);
        let untagged = "说明\n```\n{\"ok\":2}\n```";
        assert_eq!(extract_json_envelope(untagged).unwrap()["ok"], 2);
        let prose = "评估完成：\n{\"schema_version\":1,\"task_type\":\"qa\"}";
        assert_eq!(extract_json_envelope(prose).unwrap()["task_type"], "qa");
    }

    #[test]
    fn strip_worker_envelope_removes_metadata_keeps_normal_json_examples() {
        let reply = "推荐日本和泰国。\n\n```json\n{\"summary\":\"推荐两国\",\"artifacts\":[],\"evidence_notes\":\"\",\"limitations\":[]}\n```";
        assert_eq!(strip_worker_envelope(reply), "推荐日本和泰国。");
        let sample = "示例 JSON：\n```json\n{\"hello\":\"world\"}\n```";
        assert_eq!(strip_worker_envelope(sample), sample);
    }
}