//! 并行闭环调度器（P3，第 7/9/10.3 节）：DAG 就绪节点并行、资源锁、worktree 隔离、
//! 多执行者集成重审、定向返工。无预算上限：调用数仅统计；返工/重试上限保留。
//! 成功谓词由代码统一计算；“执行者说已完成”“队列跑完了”都不等于验收通过。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio_util::sync::CancellationToken;

use super::agents::{self, CandidateAgent};
use super::artifacts::ArtifactStore;
use super::contracts::{
    AcceptanceCriterion, ArtifactManifest, CheckStatus, Difficulty, EffectivePolicy,
    InvocationResult, NodeKind, NodeState, OrchestrationEvent, ReviewIssue, ReviewReport,
    ReviewVerdict, RunState, TaskNode, VerificationKind,
};
use super::executor::{
    backend_supports_read_only, truncate_utf8, AgentExecutor, ExecError, InvocationContext,
    TaskContract,
};
use super::persistence::{new_id, Store};
use super::planner;
use super::policy::{
    compute_effective_policy, enforce_criteria_review, has_external_writes, PolicyConfig,
};
use super::registry::InvocationRegistry;
use super::review::{run_review, ReviewError};
use super::triage::{assess_with_roster, TriageError};
use super::validation::{validate, ValidationOutcome};
use super::workspace;

#[derive(Clone)]
pub struct Routing {
    pub backend: String,
    /// 评估/审核后端：必须支持只读强制，否则受阻。
    pub review_backend: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub workspace: PathBuf,
    pub permission: String,
    pub candidates: Vec<CandidateAgent>,
    pub mentioned_agent_ids: Vec<String>,
}

pub struct Deps<'a> {
    pub executor: &'a dyn AgentExecutor,
    pub store: &'a Store,
    pub registry: &'a InvocationRegistry,
    pub artifacts: &'a ArtifactStore,
    pub routing: Routing,
    pub policy_config: PolicyConfig,
    pub cancel: CancellationToken,
    pub sink: Option<Box<dyn Fn(OrchestrationEvent) + Send + Sync>>,
}

/// 成功谓词（第 7 节）：全部必需项 pass、无 blocker/major、版本当前、
/// 必需审核存在且匹配、无取消/阻塞/未核实外部动作。
pub fn success_predicate(
    required_checks: &[(String, CheckStatus)],
    force_review: bool,
    review: Option<&ReviewReport>,
    versions_current: bool,
    no_cancel_or_block: bool,
) -> bool {
    required_checks.iter().all(|(_, status)| *status == CheckStatus::Pass)
        && versions_current
        && no_cancel_or_block
        && review.map(|r| r.blocking_issues().is_empty()).unwrap_or(true)
        && (!force_review
            || review.is_some_and(|r| r.verdict == ReviewVerdict::Pass && r.blocking_issues().is_empty()))
}

struct Run<'a> {
    deps: &'a Deps<'a>,
    task_id: String,
    run_id: String,
    session_id: String,
    revision: u64,
}

impl<'a> Run<'a> {
    fn emit(&self, kind: &str, payload: serde_json::Value, node: &str, invocation: &str) {
        if let Ok(seq) = self.deps.store.append_event(&self.run_id, kind, &payload, node, invocation) {
            if let Some(sink) = &self.deps.sink {
                if let Ok(events) = self.deps.store.events_after(&self.run_id, seq - 1) {
                    for event in events {
                        sink(event);
                    }
                }
            }
        }
    }

    fn transition(&mut self, state: RunState, message: &str, kind: &str, payload: serde_json::Value) -> Result<(), String> {
        let ok = self
            .deps
            .store
            .transition(&self.run_id, self.revision, state, message, kind, &payload, "", "")
            .map_err(|e| e.to_string())?;
        if !ok {
            return Err("状态 revision 过期或已终态".into());
        }
        self.revision += 1;
        if let Some(sink) = &self.deps.sink {
            if let Ok(Some(last)) = self.deps.store.latest_event(&self.run_id) {
                sink(last);
            }
        }
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.deps.cancel.is_cancelled()
    }

    fn finish_cancelled(&mut self) {
        if let Some((state, revision)) = self.deps.store.run_state(&self.run_id).ok().flatten() {
            self.revision = revision;
            if state.is_terminal() {
                return;
            }
            if state != RunState::Cancelling {
                let _ = self.transition(RunState::Cancelling, "停止中", "run.cancelling", serde_json::json!({}));
            }
            let _ = self.transition(RunState::Cancelled, "已取消", "run.cancelled", serde_json::json!({}));
        }
    }

    fn next_attempt(&self, node_id: &str) -> u32 {
        self.deps.store.next_attempt(&self.run_id, node_id).unwrap_or(1)
    }

    fn make_ctx(
        &self,
        role: NodeKind,
        node_id: &str,
        read_only: bool,
        backend: &str,
        attempt: u32,
        agent: Option<CandidateAgent>,
    ) -> InvocationContext {
        let invocation_id = new_id("inv");
        let entry = self.deps.registry.register(&invocation_id, &self.session_id, &self.run_id, node_id);
        let resolved_backend = agent
            .as_ref()
            .map(|a| a.backend.clone())
            .unwrap_or_else(|| backend.to_string());
        InvocationContext {
            session_id: self.session_id.clone(),
            task_id: self.task_id.clone(),
            run_id: self.run_id.clone(),
            node_id: node_id.to_string(),
            invocation_id,
            attempt,
            role,
            backend: resolved_backend,
            model: self.deps.routing.model.clone(),
            effort: self.deps.routing.effort.clone(),
            workspace: self.deps.routing.workspace.clone(),
            permission: self.deps.routing.permission.clone(),
            read_only,
            timeout_secs: 900,
            cancel: entry.cancel.clone(),
            entry: Some(entry),
            policy_version: super::policy::POLICY_VERSION,
            prompt_version: super::prompts::PROMPT_VERSION,
            agent,
        }
    }
}

/// 调用台账 + 统计（仅观测，不做上限）。
fn record(deps: &Deps<'_>, ctx: &InvocationContext, result: Option<&InvocationResult>, error: Option<&str>) {
    let _ = deps.store.record_invocation(
        &ctx.invocation_id,
        &ctx.run_id,
        &ctx.node_id,
        ctx.attempt,
        ctx.role.as_str(),
        &ctx.backend,
        &serde_json::json!({
            "model": ctx.model, "effort": ctx.effort, "read_only": ctx.read_only,
            "permission": ctx.permission, "policy_version": ctx.policy_version,
            "prompt_version": ctx.prompt_version,
        }),
    );
    let usage = result.map(|r| serde_json::to_value(&r.usage).unwrap()).unwrap_or(serde_json::json!({}));
    let _ = deps.store.finish_invocation(&ctx.invocation_id, &usage, error);
    let tools = result.map(|r| r.usage.tool_calls).unwrap_or(0);
    let _ = deps.store.add_usage_stats(&ctx.run_id, 1, tools);
    if let Some(result) = result {
        for record in &result.evidence {
            let _ = deps.store.insert_evidence(&ctx.run_id, record);
        }
    }
    deps.registry.finish(&ctx.invocation_id);
}

async fn execute_with_retry(
    deps: &Deps<'_>,
    ctx: &mut InvocationContext,
    contract: &TaskContract,
) -> Result<InvocationResult, ExecError> {
    match deps.executor.execute(ctx, contract).await {
        Ok(result) => {
            record(deps, ctx, Some(&result), None);
            Ok(result)
        }
        Err(error) if error.transient && deps.policy_config.max_transient_retries > 0 => {
            record(deps, ctx, None, Some(&error.message));
            ctx.invocation_id = new_id("inv");
            ctx.attempt = deps.store.next_attempt(&ctx.run_id, &ctx.node_id).unwrap_or(ctx.attempt + 1);
            let result = deps.executor.execute(ctx, contract).await;
            match &result {
                Ok(r) => record(deps, ctx, Some(r), None),
                Err(e) => record(deps, ctx, None, Some(&e.message)),
            }
            result
        }
        Err(error) => {
            record(deps, ctx, None, Some(&error.message));
            Err(error)
        }
    }
}

fn record_repair(
    deps: &Deps<'_>,
    base_ctx: &InvocationContext,
    repair: Option<&(String, u32, InvocationResult)>,
) {
    let Some((invocation_id, attempt, result)) = repair else { return };
    let mut repair_ctx = base_ctx.clone();
    repair_ctx.invocation_id = invocation_id.clone();
    repair_ctx.attempt = *attempt;
    record(deps, &repair_ctx, Some(result), None);
}

async fn assess_with_retry(
    deps: &Deps<'_>,
    ctx: &InvocationContext,
    request: &str,
    roster_context: &str,
    revision: u32,
) -> Result<super::triage::AssessOutcome, TriageError> {
    let repair_attempt = ctx.attempt + 1;
    match assess_with_roster(deps.executor, ctx, request, roster_context, revision, repair_attempt).await {
        Ok(outcome) => {
            if outcome.repaired {
                let _ = deps.store.next_attempt(&ctx.run_id, "assess");
            }
            record(deps, ctx, Some(&outcome.result), None);
            record_repair(deps, ctx, outcome.repair_result.as_ref());
            Ok(outcome)
        }
        Err(TriageError::Transient(message)) => {
            record(deps, ctx, None, Some(&message));
            if deps.policy_config.max_transient_retries == 0 {
                return Err(TriageError::Transient(message));
            }
            let attempt = deps.store.next_attempt(&ctx.run_id, "assess").unwrap_or(ctx.attempt + 1);
            let retry_repair = attempt + 1;
            let mut retry_ctx = ctx.clone();
            retry_ctx.invocation_id = new_id("inv");
            retry_ctx.attempt = attempt;
            match assess_with_roster(deps.executor, &retry_ctx, request, roster_context, revision, retry_repair).await {
                Ok(outcome) => {
                    if outcome.repaired {
                        let _ = deps.store.next_attempt(&ctx.run_id, "assess");
                    }
                    record(deps, &retry_ctx, Some(&outcome.result), None);
                    record_repair(deps, &retry_ctx, outcome.repair_result.as_ref());
                    Ok(outcome)
                }
                Err(error) => {
                    record(deps, &retry_ctx, None, Some("retry failed"));
                    Err(error)
                }
            }
        }
        Err(error) => {
            record(deps, ctx, None, Some("assess failed"));
            Err(error)
        }
    }
}

/// 组装节点上下文包；返工时附带审核缺陷与上一版产物，避免执行者盲目重跑。
fn build_work_context(
    deps: &Deps<'_>,
    node_id: &str,
    request: &str,
    round: u32,
    prior_issues: &[ReviewIssue],
    node_artifacts: &HashMap<String, ArtifactManifest>,
) -> String {
    let mut pack = format!("冻结需求（版本 1）：\n{request}");
    if round == 0 {
        return pack;
    }
    let relevant: Vec<&ReviewIssue> = prior_issues
        .iter()
        .filter(|i| i.responsible_node_id == node_id)
        .collect();
    let issues: Vec<&ReviewIssue> = if relevant.is_empty() {
        prior_issues.iter().collect()
    } else {
        relevant
    };
    if !issues.is_empty() {
        pack.push_str(&format!("\n\n## 第 {round} 轮返工缺陷单（请逐项修复）\n"));
        for issue in issues {
            pack.push_str(&format!(
                "- [{}]（{} / {}）{}\n  要求修复：{}\n  复测要求：{}\n",
                issue.id,
                issue.criterion_id,
                serde_json::to_string(&issue.severity).unwrap_or_default().trim_matches('"'),
                issue.description,
                issue.required_fix,
                issue.verification
            ));
        }
    }
    if let Some(prev) = node_artifacts.get(node_id) {
        if let Ok(items) = deps.artifacts.read_snapshot(prev) {
            if let Some((_, content)) = items.first() {
                pack.push_str(&format!(
                    "\n\n## 上一版产物参考（v{}）\n{}",
                    prev.revision,
                    truncate_utf8(content, 4000)
                ));
            }
        }
    }
    pack
}

/// DAG 并行执行：依赖完成 + 读写不冲突 + 资源锁空闲 + 并发上限才启动。
/// 写节点优先 worktree 隔离；隔离失败/后端不支持工作区覆盖时降为串行。
#[allow(clippy::too_many_arguments)]
async fn execute_work_dag(
    deps: &Deps<'_>,
    run: &mut Run<'_>,
    nodes: &[TaskNode],
    policy: &EffectivePolicy,
    round: u32,
    rerun: &HashSet<String>,
    request: &str,
    criteria: &[AcceptanceCriterion],
    prior_issues: &[ReviewIssue],
    node_artifacts: &mut HashMap<String, ArtifactManifest>,
    node_results: &mut HashMap<String, InvocationResult>,
    isolated: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), ExecError> {
    let to_run: Vec<TaskNode> = nodes
        .iter()
        .filter(|n| rerun.is_empty() || rerun.contains(&n.node_id))
        .cloned()
        .collect();
    let mut completed: HashSet<String> = nodes
        .iter()
        .map(|n| n.node_id.clone())
        .filter(|id| !to_run.iter().any(|n| &n.node_id == id))
        .collect();

    let base = deps.routing.workspace.clone();
    let run_dir = deps.artifacts.manifest_path_hint(&run.task_id, &run.run_id);
    let git_ok = workspace::is_git_repo(&base);
    let mut cap = policy.max_concurrency.max(1) as usize;
    let mut prepared: HashMap<String, PathBuf> = HashMap::new();
    let mut node_agents: HashMap<String, CandidateAgent> = HashMap::new();
    for node in &to_run {
        let assigned = agents::resolve_assigned_agent(
            &deps.routing.candidates,
            node.assigned_agent_id.as_deref(),
            &deps.routing.mentioned_agent_ids,
            &deps.routing.backend,
        );
        let node_backend = assigned.backend.as_str();
        if node.write_scopes.is_empty() {
            prepared.insert(node.node_id.clone(), base.clone());
            node_agents.insert(node.node_id.clone(), assigned);
            continue;
        }
        if matches!(node_backend, "claude_code" | "codex") && git_ok {
            match workspace::prepare_isolated(&base, &run_dir, &node.node_id) {
                Ok(wt) => {
                    isolated.push((base.clone(), wt.clone()));
                    prepared.insert(node.node_id.clone(), wt);
                }
                Err(reason) => {
                    cap = 1;
                    prepared.insert(node.node_id.clone(), base.clone());
                    run.emit("workspace.isolation_failed", serde_json::json!({"node_id": node.node_id, "reason": reason}), &node.node_id, "");
                }
            }
        } else if matches!(node_backend, "claude_code" | "codex") {
            // Git 不可用：隔离失败降串行，不静默覆盖用户工作区。
            cap = 1;
            prepared.insert(node.node_id.clone(), base.clone());
            run.emit("workspace.isolation_failed", serde_json::json!({"node_id": node.node_id, "reason": "非 Git 仓库"}), &node.node_id, "");
        } else if !git_ok && node_backend != "direct_api" {
            // 非 Git 任务且后端支持工作区目录：独立输出目录。
            let dir = workspace::scratch_dir(&run_dir, &node.node_id);
            let _ = std::fs::create_dir_all(&dir);
            prepared.insert(node.node_id.clone(), dir);
        } else {
            // 后端不支持工作区覆盖（direct_api 等）：写节点串行。
            cap = 1;
            prepared.insert(node.node_id.clone(), base.clone());
        }
        node_agents.insert(node.node_id.clone(), assigned);
    }

    let mut pending = to_run;
    let mut running: Vec<TaskNode> = Vec::new();
    let mut inflight: FuturesUnordered<
        std::pin::Pin<Box<dyn std::future::Future<Output = (String, Result<InvocationResult, ExecError>)> + Send + '_>>,
    > = FuturesUnordered::new();

    while !pending.is_empty() || !inflight.is_empty() {
        while inflight.len() < cap {
            let Some(index) = pending.iter().position(|node| {
                node.dependencies
                    .iter()
                    .all(|dep| dep == "assess" || completed.contains(dep))
                    && !running.iter().any(|r| planner::conflicts(r, node))
            }) else {
                break;
            };
            let node = pending.remove(index);
            running.push(node.clone());
            let _ = deps.store.set_node_state(&run.run_id, &node.node_id, NodeState::Running);
            if round > 0 {
                let _ = deps.store.set_rework_round(&run.run_id, &node.node_id, round);
            }
            let assigned = node_agents
                .get(&node.node_id)
                .cloned()
                .unwrap_or_else(|| {
                    agents::resolve_assigned_agent(
                        &deps.routing.candidates,
                        node.assigned_agent_id.as_deref(),
                        &deps.routing.mentioned_agent_ids,
                        &deps.routing.backend,
                    )
                });
            let mut ctx = run.make_ctx(
                NodeKind::Work,
                &node.node_id,
                false,
                &assigned.backend,
                run.next_attempt(&node.node_id),
                Some(assigned.clone()),
            );
            ctx.workspace = prepared[&node.node_id].clone();
            let node_criteria: Vec<AcceptanceCriterion> = criteria
                .iter()
                .filter(|c| node.criterion_ids.contains(&c.id))
                .cloned()
                .collect();
            let contract = TaskContract {
                objective: node.objective.clone(),
                context_pack: build_work_context(deps, &node.node_id, request, round, prior_issues, node_artifacts),
                criteria: node_criteria,
                inputs: vec![],
            };
            run.emit(
                "node.started",
                serde_json::json!({
                    "node_id": node.node_id,
                    "workspace": ctx.workspace,
                    "agent_id": assigned.id,
                    "agent_name": assigned.name,
                    "agent_avatar": assigned.avatar,
                    "backend": assigned.backend,
                }),
                &node.node_id,
                &ctx.invocation_id,
            );
            let deps_ref = deps;
            let node_id = node.node_id.clone();
            inflight.push(Box::pin(async move {
                let result = execute_with_retry(deps_ref, &mut ctx, &contract).await;
                (node_id, result)
            }));
        }
        if inflight.is_empty() {
            return Err(ExecError::failed("DAG 依赖无法满足，存在环或外部依赖缺失"));
        }
        let Some((node_id, result)) = inflight.next().await else { break };
        running.retain(|n| n.node_id != node_id);
        match result {
            Ok(res) => {
                completed.insert(node_id.clone());
                let _ = deps.store.set_node_state(&run.run_id, &node_id, NodeState::Completed);
                let manifest = deps
                    .artifacts
                    .publish_text(
                        deps.store,
                        &run.task_id,
                        &run.run_id,
                        1,
                        &format!("art-{}-{}", run.run_id, node_id),
                        round + 1,
                        &[res.invocation_id.clone()],
                        &[("reply".into(), res.text.clone())],
                        &[],
                    )
                    .map_err(|e| ExecError::failed(e.to_string()))?;
                run.emit("node.completed", serde_json::json!({"node_id": node_id}), &node_id, &res.invocation_id);
                run.emit(
                    "artifact.published",
                    serde_json::json!({"artifact_set_id": manifest.artifact_set_id, "revision": manifest.revision, "node_id": node_id}),
                    &node_id,
                    &res.invocation_id,
                );
                node_artifacts.insert(node_id.clone(), manifest);
                node_results.insert(node_id, res);
            }
            Err(error) => {
                let node_state = if error.aborted { NodeState::Cancelled } else { NodeState::Failed };
                let _ = deps.store.set_node_state(&run.run_id, &node_id, node_state);
                for r in &running {
                    let _ = deps.store.set_node_state(&run.run_id, &r.node_id, NodeState::Cancelled);
                }
                return Err(error);
            }
        }
        if deps.cancel.is_cancelled() {
            return Err(ExecError::aborted("已停止"));
        }
    }
    Ok(())
}

/// 集成节点：在独立集成区合并多份产物，产出新版本交付物供整体验收。
async fn integrate(
    deps: &Deps<'_>,
    run: &mut Run<'_>,
    node_artifacts: &HashMap<String, ArtifactManifest>,
    node_ids: &[String],
    round: u32,
) -> Result<ArtifactManifest, ExecError> {
    let mut pack = String::from("合并以下执行者产物，输出集成后的完整交付物：\n");
    for id in node_ids {
        let manifest = node_artifacts
            .get(id)
            .ok_or_else(|| ExecError::failed(format!("缺少节点 {id} 的产物")))?;
        let items = deps
            .artifacts
            .read_snapshot(manifest)
            .map_err(|e| ExecError::failed(format!("产物快照不可读：{e}")))?;
        for (item_id, content) in items {
            pack.push_str(&format!("--- {id}/{item_id} ---\n{content}\n"));
        }
    }
    let _ = deps.store.set_node_state(&run.run_id, "integrate", NodeState::Running);
    let run_dir = deps.artifacts.manifest_path_hint(&run.task_id, &run.run_id);
    let integrator_agent = agents::resolve_assigned_agent(
        &deps.routing.candidates,
        Some(agents::MAIN_AGENT_ID),
        &[],
        &deps.routing.backend,
    );
    let integrator_backend = integrator_agent.backend.clone();
    let mut ctx = run.make_ctx(
        NodeKind::Integrate,
        "integrate",
        false,
        &integrator_backend,
        run.next_attempt("integrate"),
        Some(integrator_agent),
    );
    ctx.workspace = workspace::scratch_dir(&run_dir, "integrate");
    let _ = std::fs::create_dir_all(&ctx.workspace);
    let contract = TaskContract {
        objective: "输出集成后的完整交付物正文。".into(),
        context_pack: pack,
        criteria: vec![],
        inputs: vec![],
    };
    let res = match execute_with_retry(deps, &mut ctx, &contract).await {
        Ok(res) => res,
        Err(error) => {
            let ns = if error.aborted { NodeState::Cancelled } else { NodeState::Failed };
            let _ = deps.store.set_node_state(&run.run_id, "integrate", ns);
            return Err(error);
        }
    };
    let _ = deps.store.set_node_state(&run.run_id, "integrate", NodeState::Completed);
    let manifest = deps
        .artifacts
        .publish_text(
            deps.store,
            &run.task_id,
            &run.run_id,
            1,
            &format!("art-{}-integrated", run.run_id),
            round + 1,
            &[res.invocation_id.clone()],
            &[("integrated".into(), res.text.clone())],
            &[],
        )
        .map_err(|e| ExecError::failed(e.to_string()))?;
    run.emit("integration.completed", serde_json::json!({"revision": manifest.revision}), "integrate", &res.invocation_id);
    Ok(manifest)
}

fn cleanup_workspaces(isolated: &[(PathBuf, PathBuf)]) {
    for (base, wt) in isolated {
        workspace::cleanup(base, wt);
    }
}

pub async fn run_task(
    deps: &Deps<'_>,
    session_id: &str,
    request: &str,
    idempotency_key: &str,
) -> Result<super::persistence::RunSnapshot, String> {
    let (task_id, run_id, created) = deps
        .store
        .create_run(session_id, request, idempotency_key)
        .map_err(|e| e.to_string())?;
    if !created {
        return snapshot(deps, &run_id);
    }
    run_created_task(deps, session_id, &task_id, &run_id, request).await
}

/// 执行已经由服务层原子创建好的 run（避免再次 create_run 命中幂等键而直接退出）。
pub async fn run_created_task(
    deps: &Deps<'_>,
    session_id: &str,
    task_id: &str,
    run_id: &str,
    request: &str,
) -> Result<super::persistence::RunSnapshot, String> {
    let run_id = run_id.to_string();
    let mut run = Run {
        deps,
        task_id: task_id.to_string(),
        run_id: run_id.clone(),
        session_id: session_id.to_string(),
        revision: 1,
    };

    if run.cancelled() {
        run.finish_cancelled();
        return snapshot(deps, &run_id);
    }
    if !backend_supports_read_only(&deps.routing.review_backend) {
        run.transition(
            RunState::Blocked,
            "没有支持只读强制的后端承载评估/审核",
            "run.blocked",
            serde_json::json!({"reason":"read_only_backend_unavailable"}),
        )?;
        return snapshot(deps, &run_id);
    }

    // ===== 必经独立评估 =====
    let assess_agent = CandidateAgent::synthetic(
        "assessor",
        "任务评估",
        "🧭",
        &deps.routing.review_backend,
        false,
    );
    let assess_spec = serde_json::json!({
        "node_id":"assess",
        "kind":"assess",
        "objective":"独立评估",
        "assigned_agent_id": assess_agent.id,
        "assigned_agent_name": assess_agent.name,
        "assigned_agent_avatar": assess_agent.avatar,
        "assigned_backend": assess_agent.backend,
    });
    let _ = deps.store.upsert_node(&run_id, &assess_spec, NodeState::Running);
    run.transition(RunState::Assessing, "", "run.assessing", serde_json::json!({}))?;
    let attempt = run.next_attempt("assess");
    let assess_ctx = run.make_ctx(
        NodeKind::Assess,
        "assess",
        true,
        &deps.routing.review_backend,
        attempt,
        Some(assess_agent),
    );
    let roster_context = agents::format_roster_for_prompt(
        &deps.routing.candidates,
        &deps.routing.mentioned_agent_ids,
    );
    let outcome = match assess_with_retry(deps, &assess_ctx, request, &roster_context, 1).await {
        Ok(outcome) => outcome,
        Err(TriageError::Aborted(_)) => {
            let _ = deps.store.set_node_state(&run_id, "assess", NodeState::Cancelled);
            run.finish_cancelled();
            return snapshot(deps, &run_id);
        }
        Err(TriageError::Blocked(message)) => {
            let _ = deps.store.set_node_state(&run_id, "assess", NodeState::Blocked);
            run.transition(RunState::Blocked, &message, "run.blocked", serde_json::json!({"reason":"assessment_invalid"}))?;
            return snapshot(deps, &run_id);
        }
        Err(TriageError::Transient(message)) => {
            let _ = deps.store.set_node_state(&run_id, "assess", NodeState::Failed);
            run.transition(RunState::Failed, &message, "run.failed", serde_json::json!({"reason":"assessment_transient"}))?;
            return snapshot(deps, &run_id);
        }
    };
    if run.cancelled() {
        let _ = deps.store.set_node_state(&run_id, "assess", NodeState::Cancelled);
        run.finish_cancelled();
        return snapshot(deps, &run_id);
    }
    let _ = deps.store.set_node_state(&run_id, "assess", NodeState::Completed);
    let assessment = outcome.assessment;
    let recommended_agent = agents::resolve_assigned_agent(
        &deps.routing.candidates,
        assessment.recommended_agent_id.as_deref(),
        &deps.routing.mentioned_agent_ids,
        &deps.routing.backend,
    );
    let mut policy = compute_effective_policy(&assessment, has_external_writes(&assessment), &deps.policy_config);
    let criteria = super::triage::freeze_criteria(&assessment);
    enforce_criteria_review(&mut policy, &criteria);
    let _ = deps.store.set_policy(&run_id, &serde_json::to_value(&policy).unwrap(), 1);
    run.emit(
        "assessment.completed",
        serde_json::json!({
            "difficulty": policy.difficulty.as_str(),
            "risk": policy.risk.as_str(),
            "force_review": policy.force_review,
            "rule_hits": policy.rule_hits,
            "repaired": outcome.repaired,
            "task_type": assessment.task_type,
            "rationale": assessment.rationale,
            "recommended_agent_id": recommended_agent.id,
            "recommended_agent_name": recommended_agent.name,
            "recommended_agent_avatar": recommended_agent.avatar,
            "recommended_backend": recommended_agent.backend,
            "recommended_reason": assessment.recommended_reason,
        }),
        "assess",
        &assess_ctx.invocation_id,
    );

    // ===== 计划：L3 且可并行时模型 DAG，后端校验；否则单执行者 =====
    let mut work_nodes;
    if policy.difficulty == Difficulty::L3 && assessment.parallelizable {
        run.transition(RunState::Planning, "", "run.planning", serde_json::json!({}))?;
        let plan_agent = CandidateAgent::synthetic(
            "planner",
            "任务规划",
            "🗺️",
            &deps.routing.review_backend,
            false,
        );
        let plan_spec = serde_json::json!({
            "node_id":"plan",
            "kind":"plan",
            "objective":"生成计划",
            "assigned_agent_id": plan_agent.id,
            "assigned_agent_name": plan_agent.name,
            "assigned_agent_avatar": plan_agent.avatar,
            "assigned_backend": plan_agent.backend,
        });
        let _ = deps.store.upsert_node(&run_id, &plan_spec, NodeState::Running);
        let plan_attempt = run.next_attempt("plan");
        let plan_ctx = run.make_ctx(
            NodeKind::Plan,
            "plan",
            true,
            &deps.routing.review_backend,
            plan_attempt,
            Some(plan_agent),
        );
        let (nodes, planned) = planner::plan_work_nodes_with_roster(
            deps.executor,
            &plan_ctx,
            request,
            &criteria,
            &roster_context,
        )
        .await;
        record(deps, &plan_ctx, None, None);
        if run.cancelled() {
            let _ = deps.store.set_node_state(&run_id, "plan", NodeState::Cancelled);
            run.finish_cancelled();
            return snapshot(deps, &run_id);
        }
        let _ = deps.store.set_node_state(&run_id, "plan", NodeState::Completed);
        run.emit(
            "plan.completed",
            serde_json::json!({"planned": planned, "nodes": nodes.iter().map(|n| &n.node_id).collect::<Vec<_>>()}),
            "plan",
            &plan_ctx.invocation_id,
        );
        work_nodes = nodes;
    } else {
        work_nodes = vec![planner::work_node("work-1", request, &criteria, vec![])];
    }
    for node in &mut work_nodes {
        let preferred = node
            .assigned_agent_id
            .as_deref()
            .or(Some(recommended_agent.id.as_str()));
        let assigned = agents::resolve_assigned_agent(
            &deps.routing.candidates,
            preferred,
            &deps.routing.mentioned_agent_ids,
            &deps.routing.backend,
        );
        node.assigned_agent_id = Some(assigned.id);
        node.assigned_agent_name = Some(assigned.name);
        node.assigned_agent_avatar = Some(assigned.avatar);
        node.assigned_backend = Some(assigned.backend);
        let _ = deps.store.upsert_node(&run_id, &serde_json::to_value(&node).unwrap(), NodeState::Pending);
    }
    let multi = work_nodes.len() > 1;
    let work_ids: Vec<String> = work_nodes.iter().map(|n| n.node_id.clone()).collect();
    if multi {
        let integrator_agent = agents::resolve_assigned_agent(
            &deps.routing.candidates,
            Some(agents::MAIN_AGENT_ID),
            &[],
            &deps.routing.backend,
        );
        let integrate_node = TaskNode {
            node_id: "integrate".into(),
            kind: NodeKind::Integrate,
            dependencies: work_ids.clone(),
            assigned_agent_id: Some(integrator_agent.id),
            assigned_agent_name: Some(integrator_agent.name),
            assigned_agent_avatar: Some(integrator_agent.avatar),
            assigned_backend: Some(integrator_agent.backend),
            objective: "集成多份产物".into(),
            criterion_ids: criteria.iter().map(|c| c.id.clone()).collect(),
            ..planner::work_node("integrate", "", &criteria, vec![])
        };
        let _ = deps.store.upsert_node(&run_id, &serde_json::to_value(&integrate_node).unwrap(), NodeState::Pending);
    }
    if policy.force_review {
        let review_node = TaskNode {
            node_id: "review-final".into(),
            kind: NodeKind::Review,
            dependencies: if multi { vec!["integrate".into()] } else { work_ids.clone() },
            assigned_agent_id: Some("reviewer".into()),
            assigned_agent_name: Some("独立审核".into()),
            assigned_agent_avatar: Some("🔍".into()),
            assigned_backend: Some(deps.routing.review_backend.clone()),
            objective: "独立验收".into(),
            write_scopes: vec![],
            ..planner::work_node("review-final", "", &criteria, vec![])
        };
        let _ = deps.store.upsert_node(&run_id, &serde_json::to_value(&review_node).unwrap(), NodeState::Pending);
    }

    run.transition(RunState::Executing, "", "run.executing", serde_json::json!({}))?;
    let mut node_artifacts: HashMap<String, ArtifactManifest> = HashMap::new();
    let mut node_results: HashMap<String, InvocationResult> = HashMap::new();
    let mut isolated: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut prior_issues: Vec<ReviewIssue> = Vec::new();
    let mut round: u32 = 0;

    loop {
        if run.cancelled() {
            cleanup_workspaces(&isolated);
            run.finish_cancelled();
            return snapshot(deps, &run_id);
        }
        // 定向返工：只重跑责任节点；无明确责任时重跑全部。
        let mut rerun: HashSet<String> = prior_issues
            .iter()
            .map(|i| i.responsible_node_id.clone())
            .filter(|id| work_ids.contains(id))
            .collect();
        if round > 0 && rerun.is_empty() {
            rerun = work_ids.iter().cloned().collect();
        }
        if let Err(error) = execute_work_dag(
            deps, &mut run, &work_nodes, &policy, round, &rerun, request, &criteria, &prior_issues,
            &mut node_artifacts, &mut node_results, &mut isolated,
        )
        .await
        {
            cleanup_workspaces(&isolated);
            if error.aborted || run.cancelled() {
                run.finish_cancelled();
            } else {
                let state = if error.message.contains("暂不承载") || error.message.contains("未配置") {
                    RunState::Blocked
                } else {
                    RunState::Failed
                };
                run.transition(state, &error.message, "run.failed", serde_json::json!({}))?;
            }
            return snapshot(deps, &run_id);
        }

        // ===== 集成（多执行者）=====
        let target = if multi {
            run.transition(RunState::Integrating, "", "run.integrating", serde_json::json!({}))?;
            match integrate(deps, &mut run, &node_artifacts, &work_ids, round).await {
                Ok(manifest) => manifest,
                Err(error) => {
                    cleanup_workspaces(&isolated);
                    if error.aborted || run.cancelled() {
                        run.finish_cancelled();
                    } else {
                        run.transition(RunState::Failed, &error.message, "run.failed", serde_json::json!({"node":"integrate"}))?;
                    }
                    return snapshot(deps, &run_id);
                }
            }
        } else {
            let primary = work_ids.first().map(String::as_str).unwrap_or("work-1");
            node_artifacts
                .get(primary)
                .cloned()
                .ok_or_else(|| format!("缺少 {primary} 产物"))?
        };

        // ===== 必要验证 =====
        run.transition(RunState::Validating, "", "run.validating", serde_json::json!({}))?;
        let snapshot_items = deps
            .artifacts
            .read_snapshot(&target)
            .map_err(|e| format!("产物快照不可读：{e}"))?;
        let evidence: Vec<super::contracts::EvidenceRecord> =
            node_results.values().flat_map(|r| r.evidence.clone()).collect();
        let outcome_validation: ValidationOutcome = validate(&criteria, &evidence, &snapshot_items);
        run.emit(
            "validation.completed",
            serde_json::json!(outcome_validation
                .checks
                .iter()
                .map(|c| serde_json::json!({"criterion_id": c.criterion_id, "status": serde_json::to_value(&c.status).unwrap()}))
                .collect::<Vec<_>>()),
            "work-1",
            "",
        );
        let failures = outcome_validation.required_failures(&criteria);
        let unverified = outcome_validation.required_unverified(&criteria);

        if outcome_validation.environment_blocked {
            cleanup_workspaces(&isolated);
            run.transition(RunState::Blocked, "环境/权限/依赖缺失", "run.blocked", serde_json::json!({}))?;
            return snapshot(deps, &run_id);
        }

        let mut review_report: Option<ReviewReport> = None;
        if policy.force_review {
            // ===== 独立审核（审集成结果）=====
            run.transition(RunState::Reviewing, "", "run.reviewing", serde_json::json!({}))?;
            let _ = deps.store.set_node_state(&run_id, "review-final", NodeState::Running);
            let attempt = run.next_attempt("review-final");
            let repair_attempt = attempt + 1;
            let review_agent = CandidateAgent::synthetic(
                "reviewer",
                "独立审核",
                "🔍",
                &deps.routing.review_backend,
                false,
            );
            let review_ctx = run.make_ctx(
                NodeKind::Review,
                "review-final",
                true,
                &deps.routing.review_backend,
                attempt,
                Some(review_agent),
            );
            let mut known_nodes = work_ids.clone();
            if multi {
                known_nodes.push("integrate".into());
            }
            let review_outcome = run_review(
                deps.executor,
                &review_ctx,
                request,
                1,
                &criteria,
                &target,
                &snapshot_items,
                &evidence,
                &known_nodes,
                if prior_issues.is_empty() { None } else { Some(&prior_issues) },
                repair_attempt,
            )
            .await;
            match review_outcome {
                Ok(outcome) => {
                    if outcome.repair_result.is_some() {
                        let _ = deps.store.next_attempt(&run_id, "review-final");
                    }
                    record(deps, &review_ctx, Some(&outcome.result), None);
                    record_repair(deps, &review_ctx, outcome.repair_result.as_ref());
                    let review_id = new_id("review");
                    let _ = deps.store.insert_review(
                        &review_id,
                        &run_id,
                        &review_ctx.invocation_id,
                        &outcome.report,
                        target.revision,
                    );
                    run.emit(
                        "review.completed",
                        serde_json::json!({"review_id": review_id, "artifact_revision": target.revision, "verdict": serde_json::to_value(&outcome.report.verdict).unwrap()}),
                        "review-final",
                        &review_ctx.invocation_id,
                    );
                    review_report = Some(outcome.report);
                }
                Err(ReviewError::Aborted(_)) => {
                    record(deps, &review_ctx, None, Some("aborted"));
                    let _ = deps.store.set_node_state(&run_id, "review-final", NodeState::Cancelled);
                    cleanup_workspaces(&isolated);
                    run.finish_cancelled();
                    return snapshot(deps, &run_id);
                }
                Err(ReviewError::Invalid(message)) => {
                    record(deps, &review_ctx, None, Some(&message));
                    let _ = deps.store.set_node_state(&run_id, "review-final", NodeState::Blocked);
                    cleanup_workspaces(&isolated);
                    run.transition(RunState::Blocked, &message, "run.blocked", serde_json::json!({"reason":"review_invalid"}))?;
                    return snapshot(deps, &run_id);
                }
                Err(ReviewError::Blocked(message)) | Err(ReviewError::Transient(message)) => {
                    record(deps, &review_ctx, None, Some(&message));
                    let _ = deps.store.set_node_state(&run_id, "review-final", NodeState::Blocked);
                    cleanup_workspaces(&isolated);
                    run.transition(RunState::Blocked, &format!("审核不可用：{message}"), "run.blocked", serde_json::json!({"reason":"review_unavailable"}))?;
                    return snapshot(deps, &run_id);
                }
            }
            let _ = deps.store.set_node_state(&run_id, "review-final", NodeState::Completed);
        }

        // ===== 完成判定 / 返工 =====
        let required_checks: Vec<(String, CheckStatus)> = criteria
            .iter()
            .filter(|c| c.required)
            .map(|c| {
                let status = review_report
                    .as_ref()
                    .and_then(|r| r.checks.iter().find(|check| check.criterion_id == c.id))
                    .map(|check| check.status)
                    .or_else(|| {
                        outcome_validation
                            .checks
                            .iter()
                            .find(|check| check.criterion_id == c.id)
                            .map(|check| check.status)
                    })
                    .unwrap_or(CheckStatus::Unverified);
                (c.id.clone(), status)
            })
            .collect();

        let verdict = review_report.as_ref().map(|r| r.verdict);
        let rework_issues: Vec<ReviewIssue> = match verdict {
            Some(ReviewVerdict::Rework) => review_report.as_ref().unwrap().issues.clone(),
            Some(ReviewVerdict::Blocked) => {
                cleanup_workspaces(&isolated);
                run.transition(RunState::Blocked, "审核判定受阻", "run.blocked", serde_json::json!({}))?;
                return snapshot(deps, &run_id);
            }
            _ => Vec::new(),
        };

        let needs_rework = !failures.is_empty()
            || verdict == Some(ReviewVerdict::Rework)
            || (!policy.force_review && !unverified.is_empty());

        if !needs_rework {
            let ok = success_predicate(
                &required_checks,
                policy.force_review,
                review_report.as_ref(),
                true,
                !run.cancelled(),
            );
            cleanup_workspaces(&isolated);
            if ok {
                let _ = deps.store.set_final_artifact(&run_id, &target.artifact_set_id);
                run.transition(
                    RunState::Succeeded,
                    "验收通过",
                    "run.succeeded",
                    serde_json::json!({"artifact_set_id": target.artifact_set_id, "revision": target.revision}),
                )?;
                return snapshot(deps, &run_id);
            }
            run.transition(
                RunState::Blocked,
                "成功谓词不满足（必需项/审核/版本）",
                "run.blocked",
                serde_json::json!({"required_checks": required_checks}),
            )?;
            return snapshot(deps, &run_id);
        }

        // 若唯一未通过的原因是需要用户交互确认的条目（VerificationKind::User），转入 waiting_input 而非盲目返工。
        if failures.is_empty()
            && verdict != Some(ReviewVerdict::Rework)
            && !unverified.is_empty()
            && unverified.iter().all(|id| {
                criteria
                    .iter()
                    .find(|c| &c.id == id)
                    .is_some_and(|c| c.verification == VerificationKind::User)
            })
        {
            cleanup_workspaces(&isolated);
            run.transition(
                RunState::WaitingInput,
                "需要用户确认验收项后继续",
                "run.waiting_input",
                serde_json::json!({"criteria": unverified}),
            )?;
            return snapshot(deps, &run_id);
        }

        if round >= policy.max_rework {
            cleanup_workspaces(&isolated);
            let state = if policy.force_review { RunState::ReviewFailed } else { RunState::Blocked };
            let message = if policy.force_review {
                "达到返工上限，仍未通过独立审核"
            } else {
                "无法验证必需项，且已达返工上限"
            };
            run.transition(
                state,
                message,
                "run.review_failed",
                serde_json::json!({"remaining_issues": rework_issues.iter().map(|i| &i.id).collect::<Vec<_>>(), "failures": failures, "unverified": unverified}),
            )?;
            return snapshot(deps, &run_id);
        }

        prior_issues = if !rework_issues.is_empty() {
            rework_issues
        } else {
            failures
                .iter()
                .chain(unverified.iter())
                .map(|criterion_id| ReviewIssue {
                    id: new_id("issue"),
                    severity: super::contracts::IssueSeverity::Major,
                    criterion_id: criterion_id.clone(),
                    artifact_ref: target.artifact_set_id.clone(),
                    responsible_node_id: "work-1".into(),
                    description: format!("必需验收项 {criterion_id} 未通过/缺少可核验证据"),
                    required_fix: "补齐真实工具证据或修正产物".into(),
                    verification: "重跑相关检查并提供退出码/日志".into(),
                })
                .collect()
        };
        round += 1;
        run.transition(
            RunState::ReworkRequired,
            &format!("返工 {round}/{}", policy.max_rework),
            "run.rework",
            serde_json::json!({"round": round, "issues": prior_issues.iter().map(|i| &i.id).collect::<Vec<_>>()}),
        )?;
        run.transition(RunState::Executing, "", "run.executing", serde_json::json!({"round": round}))?;
    }
}

fn snapshot(deps: &Deps<'_>, run_id: &str) -> Result<super::persistence::RunSnapshot, String> {
    deps.store.snapshot(run_id).ok().flatten().ok_or_else(|| "run 不存在".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::contracts::{EvidenceRecord, Usage};
    use super::super::executor::FakeExecutor;
    use super::super::persistence::ensure_schema;
    use std::sync::{Arc, Mutex};

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("orch-sched-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn setup() -> (TempDir, Store, ArtifactStore, InvocationRegistry) {
        let dir = TempDir::new();
        let store = Store::new(dir.0.join("pet.db"));
        ensure_schema(store.path()).unwrap();
        let artifacts = ArtifactStore::new(dir.0.join("files"));
        (dir, store, artifacts, InvocationRegistry::new())
    }

    fn routing() -> Routing {
        Routing {
            backend: "fake".into(),
            review_backend: "direct_api".into(),
            model: None,
            effort: None,
            workspace: PathBuf::from("."),
            permission: String::new(),
            candidates: Vec::new(),
            mentioned_agent_ids: Vec::new(),
        }
    }

    fn deps<'a>(
        executor: &'a FakeExecutor,
        store: &'a Store,
        registry: &'a InvocationRegistry,
        artifacts: &'a ArtifactStore,
        cancel: CancellationToken,
    ) -> Deps<'a> {
        Deps {
            executor,
            store,
            registry,
            artifacts,
            routing: routing(),
            policy_config: PolicyConfig::default(),
            cancel,
            sink: None,
        }
    }

    fn assessment_json(task_type: &str, each: u8, risk: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "task_revision": 1, "task_type": task_type, "tags": [],
            "scores": {"scope":each,"uncertainty":each,"reasoning":each,"dependencies":each,"verification":each,"effort":each},
            "suggested_difficulty": "L0", "suggested_risk": risk, "confidence": 0.9,
            "rationale": [], "missing_information": [], "required_capabilities": [],
            "acceptance_criteria": [], "parallelizable": false
        })
    }

    fn l3_assess(executor: &FakeExecutor) {
        let mut value = assessment_json("coding", 2, "R1");
        value["suggested_difficulty"] = serde_json::json!("L3");
        value["parallelizable"] = serde_json::json!(true);
        let mut res = work_result("", false);
        res.structured = Some(value);
        executor.push(Some(NodeKind::Assess), Ok(res));
    }

    fn work_result(text: &str, with_command_evidence: bool) -> InvocationResult {
        InvocationResult {
            invocation_id: String::new(),
            node_id: String::new(),
            attempt: 1,
            text: text.into(),
            structured: None,
            usage: Usage::default(),
            evidence: if with_command_evidence {
                vec![EvidenceRecord {
                    evidence_id: "ev-cmd".into(),
                    source_invocation_id: String::new(),
                    kind: "command".into(),
                    summary: "cargo test".into(),
                    cwd: String::new(),
                    exit_code: Some(0),
                    artifact_refs: vec![],
                    log_ref: "ok".into(),
                    created_at: 0,
                }]
            } else if text.contains("来源") {
                vec![EvidenceRecord {
                    evidence_id: "ev-src".into(),
                    source_invocation_id: String::new(),
                    kind: "tool".into(),
                    summary: "read_file".into(),
                    cwd: String::new(),
                    exit_code: None,
                    artifact_refs: vec![],
                    log_ref: "原文".into(),
                    created_at: 0,
                }]
            } else {
                vec![]
            },
            error: None,
            aborted: false,
        }
    }

    fn extract_between(pack: &str, start: &str, end: &str) -> String {
        let Some(i) = pack.find(start) else { return String::new() };
        let rest = &pack[i + start.len()..];
        let Some(j) = rest.find(end) else { return String::new() };
        rest[..j].to_string()
    }

    fn empty_result() -> InvocationResult {
        work_result("", false)
    }

    /// 模拟真实审核模型：从上下文包读取 set_id/hash，按脚本给结论。
    fn generative_review(executor: &FakeExecutor, verdicts: Vec<&'static str>, wrong_hash: bool) {
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        *executor.generative.lock().unwrap() = Some(Box::new(move |ctx, contract| {
            if ctx.role != NodeKind::Review {
                return None;
            }
            let pack = &contract.context_pack;
            let set_id = extract_between(pack, "artifact_set_id=", "，");
            let hash = extract_between(pack, "content_hash=", "）");
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as usize;
            let verdict = verdicts[n.min(verdicts.len() - 1)];
            let required: Vec<&AcceptanceCriterion> = contract.criteria.iter().filter(|c| c.required).collect();
            let checks: Vec<serde_json::Value> = required
                .iter()
                .map(|c| serde_json::json!({"criterion_id": c.id, "status": "pass", "evidence_refs": ["ev-cmd"], "explanation": "退出码0"}))
                .collect();
            let issues = if verdict == "rework" {
                serde_json::json!([{
                    "id": format!("issue-{n}"), "severity": "major",
                    "criterion_id": required.first().map(|c| c.id.clone()).unwrap_or_default(),
                    "artifact_ref": "reply", "responsible_node_id": "work-1",
                    "description": "边界未覆盖", "required_fix": "补边界测试", "verification": "cargo test"
                }])
            } else {
                serde_json::json!([])
            };
            let report = serde_json::json!({
                "schema_version": 1, "task_revision": 1,
                "artifact_set_id": set_id,
                "artifact_hash": if wrong_hash { "stale-hash".to_string() } else { hash },
                "verdict": verdict, "checks": checks, "issues": issues
            });
            let mut result = empty_result();
            result.structured = Some(report);
            Some(Ok(result))
        }));
    }

    fn coding_l2_assess(executor: &FakeExecutor) {
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assessment_json("coding", 1, "R1")); // 6 分 L2 必审
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
    }

    #[test]
    fn l0_fast_path_one_assess_one_work_no_review() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assessment_json("writing", 0, "R0"));
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
        executor.push(Some(NodeKind::Work), Ok(work_result("改写结果，来源已核对", false)));
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "改写这段话", "key-l0",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        let roles: Vec<NodeKind> = executor.calls.lock().unwrap().iter().map(|(r, _)| *r).collect();
        assert_eq!(roles, vec![NodeKind::Assess, NodeKind::Work]);
        assert!(snapshot.nodes.iter().all(|n| n["node_id"] != "review-final"));
    }

    #[test]
    fn forced_review_pass_delivers_with_review_record() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        executor.push(Some(NodeKind::Work), Ok(work_result("修复代码", true)));
        generative_review(&executor, vec!["pass"], false);
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "修个 bug", "key-review",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert_eq!(snapshot.reviews.len(), 1);
        assert!(snapshot.nodes.iter().any(|n| n["node_id"] == "review-final"));
        assert!(snapshot.final_artifact_set_id.is_some());
    }

    #[test]
    fn rework_then_pass_delivers_revision_two() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        executor.push(Some(NodeKind::Work), Ok(work_result("修复 v1", true)));
        executor.push(Some(NodeKind::Work), Ok(work_result("修复 v2", true)));
        generative_review(&executor, vec!["rework", "pass"], false);
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "修个 bug", "key-rework",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert_eq!(snapshot.artifacts.len(), 2, "返工必须生成新产物版本");
        assert_eq!(snapshot.reviews.len(), 2);
        let events = store.events_after(&snapshot.run_id, 0).unwrap();
        assert!(events.iter().any(|e| e.kind == "run.rework"));
    }

    #[test]
    fn three_strikes_review_failed() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        for text in ["v1", "v2", "v3"] {
            executor.push(Some(NodeKind::Work), Ok(work_result(text, true)));
        }
        generative_review(&executor, vec!["rework"], false);
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "修个 bug", "key-strikes",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::ReviewFailed);
        assert_eq!(snapshot.artifacts.len(), 3);
    }

    #[test]
    fn forged_pass_without_evidence_never_succeeds() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assessment_json("coding", 0, "R0"));
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
        for _ in 0..3 {
            executor.push(Some(NodeKind::Work), Ok(work_result("全部测试通过 PASS", false)));
        }
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "改代码", "key-forged",
        ))
        .unwrap()
        .unwrap();
        assert_ne!(snapshot.state, RunState::Succeeded);
        assert_eq!(snapshot.state, RunState::Blocked);
    }

    #[test]
    fn cancel_before_start_yields_cancelled() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, cancel),
            "s1", "t", "key-cancel",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Cancelled);
    }

    #[test]
    fn reviewer_unavailable_blocks_not_completed() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        executor.push(Some(NodeKind::Work), Ok(work_result("修复代码", true)));
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "修个 bug", "key-noreview",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Blocked);
    }

    #[test]
    fn stale_hash_review_cannot_approve() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        executor.push(Some(NodeKind::Work), Ok(work_result("修复代码", true)));
        generative_review(&executor, vec!["pass"], true);
        let snapshot = futures_util::FutureExt::now_or_never(run_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1", "修个 bug", "key-stale",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Blocked);
    }

    #[test]
    fn success_predicate_requires_independent_review_when_forced() {
        let checks = vec![("c1".to_string(), CheckStatus::Pass)];
        assert!(!success_predicate(&checks, true, None, true, true), "执行者自称完成不能算成功");
        assert!(success_predicate(&checks, false, None, true, true));
        assert!(!success_predicate(&checks, false, None, false, true), "版本不当前不能成功");
        assert!(!success_predicate(&checks, false, None, true, false), "取消/阻塞不能成功");
    }

    // ===== P3：并行 / 资源锁 / 隔离 / 集成 =====

    fn plan_two_nodes(executor: &FakeExecutor, same_scope: bool) {
        let scope_b = if same_scope { "src/a" } else { "src/b" };
        let nodes = serde_json::json!({"nodes": [
            {"node_id":"work-1","kind":"work","objective":"子任务一","dependencies":[],
             "criterion_ids":["checks-pass"],"read_scopes":[],"write_scopes":["src/a"],"resource_locks":[]},
            {"node_id":"work-2","kind":"work","objective":"子任务二","dependencies":[],
             "criterion_ids":["checks-pass"],"read_scopes":[],"write_scopes":[scope_b],"resource_locks":[]}
        ]});
        let mut res = empty_result();
        res.structured = Some(nodes);
        executor.push(Some(NodeKind::Plan), Ok(res));
    }

    type Intervals = Arc<Mutex<Vec<(String, u64, u64)>>>;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    /// work 节点异步耗时 120ms 并记录区间；integrate 输出 merged。
    fn timed_work_hooks(executor: &FakeExecutor, intervals: Intervals, counts: Arc<Mutex<HashMap<String, u32>>>) {
        *executor.generative_async.lock().unwrap() = Some(Box::new(move |ctx, _contract| {
            let intervals = intervals.clone();
            let counts = counts.clone();
            Box::pin(async move {
                match ctx.role {
                    NodeKind::Work => {
                        *counts.lock().unwrap().entry(ctx.node_id.clone()).or_insert(0) += 1;
                        let start = now_ms();
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                        intervals.lock().unwrap().push((ctx.node_id.clone(), start, now_ms()));
                        let mut res = work_result(&format!("part-{}", ctx.node_id), true);
                        res.invocation_id = ctx.invocation_id.clone();
                        Some(Ok(res))
                    }
                    NodeKind::Integrate => {
                        let mut res = work_result("merged", true);
                        res.invocation_id = ctx.invocation_id.clone();
                        Some(Ok(res))
                    }
                    _ => None,
                }
            })
        }));
    }

    fn overlaps(intervals: &[(String, u64, u64)]) -> bool {
        for i in 0..intervals.len() {
            for j in i + 1..intervals.len() {
                let a = &intervals[i];
                let b = &intervals[j];
                if a.0 != b.0 && a.1 < b.2 && b.1 < a.2 {
                    return true;
                }
            }
        }
        false
    }

    fn p3_deps<'a>(
        executor: &'a FakeExecutor,
        store: &'a Store,
        registry: &'a InvocationRegistry,
        artifacts: &'a ArtifactStore,
        dir: &TempDir,
        backend: &str,
    ) -> Deps<'a> {
        let mut d = deps(executor, store, registry, artifacts, CancellationToken::new());
        d.routing.backend = backend.into();
        d.routing.workspace = dir.0.clone();
        d.policy_config.serial_mvp = false;
        d
    }

    #[test]
    fn independent_nodes_run_concurrently_then_integrate_and_review() {
        let (dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        l3_assess(&executor);
        plan_two_nodes(&executor, false);
        let intervals: Intervals = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(Mutex::new(HashMap::new()));
        timed_work_hooks(&executor, intervals.clone(), counts.clone());
        generative_review(&executor, vec!["pass"], false);
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(
                &p3_deps(&executor, &store, &registry, &artifacts, &dir, "fake"),
                "s1", "大任务", "key-par",
            ))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert!(overlaps(&intervals.lock().unwrap()), "独立写范围应并行");
        assert!(snapshot.nodes.iter().any(|n| n["node_id"] == "integrate"));
        assert_eq!(snapshot.reviews.len(), 1, "最终审的是集成结果");
        assert!(snapshot.final_artifact_set_id.unwrap().ends_with("-integrated"));
    }

    #[test]
    fn conflicting_write_scopes_serialize() {
        let (dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        l3_assess(&executor);
        plan_two_nodes(&executor, true);
        let intervals: Intervals = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(Mutex::new(HashMap::new()));
        timed_work_hooks(&executor, intervals.clone(), counts.clone());
        generative_review(&executor, vec!["pass"], false);
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(
                &p3_deps(&executor, &store, &registry, &artifacts, &dir, "fake"),
                "s1", "大任务", "key-conflict",
            ))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert!(!overlaps(&intervals.lock().unwrap()), "同写范围必须串行，不能静默覆盖");
    }

    #[test]
    fn isolation_failure_degrades_to_serial() {
        let (dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        l3_assess(&executor);
        plan_two_nodes(&executor, false);
        let intervals: Intervals = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(Mutex::new(HashMap::new()));
        timed_work_hooks(&executor, intervals.clone(), counts.clone());
        generative_review(&executor, vec!["pass"], false);
        // claude_code 后端 + 非 Git 工作区 ⇒ worktree 隔离失败 ⇒ 降串行。
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(
                &p3_deps(&executor, &store, &registry, &artifacts, &dir, "claude_code"),
                "s1", "大任务", "key-iso",
            ))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert!(!overlaps(&intervals.lock().unwrap()), "隔离失败降串行");
        let events = store.events_after(&snapshot.run_id, 0).unwrap();
        assert!(events.iter().any(|e| e.kind == "workspace.isolation_failed"));
    }

    #[test]
    fn rework_reruns_only_responsible_node_and_reintegrates() {
        let (dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        l3_assess(&executor);
        plan_two_nodes(&executor, false);
        let intervals: Intervals = Arc::new(Mutex::new(Vec::new()));
        let counts = Arc::new(Mutex::new(HashMap::new()));
        timed_work_hooks(&executor, intervals.clone(), counts.clone());
        generative_review(&executor, vec!["rework", "pass"], false);
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(
                &p3_deps(&executor, &store, &registry, &artifacts, &dir, "fake"),
                "s1", "大任务", "key-targeted",
            ))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        let counts = counts.lock().unwrap();
        assert_eq!(counts["work-1"], 2, "责任节点返工重跑");
        assert_eq!(counts["work-2"], 1, "已通过节点不重跑");
        assert_eq!(snapshot.reviews.len(), 2);
        let events = store.events_after(&snapshot.run_id, 0).unwrap();
        assert_eq!(events.iter().filter(|e| e.kind == "integration.completed").count(), 2, "返工后重新集成");
        let work1 = snapshot.nodes.iter().find(|n| n["node_id"] == "work-1").unwrap();
        assert_eq!(work1["state"], "completed");
        assert_eq!(work1["rework_round"], 1);
    }

    #[test]
    fn service_precreated_run_executes_and_pure_qa_succeeds_without_tools() {
        let (_dir, store, artifacts, registry) = setup();
        let (task_id, run_id, created) = store.create_run("s1", "帮我找两个适合旅游的国家", "send-1").unwrap();
        assert!(created);
        let executor = FakeExecutor::new();
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assessment_json("qa", 0, "R0"));
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
        executor.push(
            Some(NodeKind::Work),
            Ok(work_result("推荐日本和泰国。\n```json\n{\"summary\":\"推荐两个旅游国家\",\"artifacts\":[]}\n```", false)),
        );
        let snapshot = futures_util::FutureExt::now_or_never(run_created_task(
            &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
            "s1",
            &task_id,
            &run_id,
            "帮我找两个适合旅游的国家",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        let work1 = snapshot.nodes.iter().find(|n| n["node_id"] == "work-1").unwrap();
        assert_eq!(work1["state"], "completed");
    }

    #[test]
    fn mid_run_cancel_with_bumped_revision_still_reaches_cancelled() {
        let (_dir, store, artifacts, registry) = setup();
        let (task_id, run_id, _) = store.create_run("s1", "t", "key-bump").unwrap();
        let executor = FakeExecutor::new();
        let cancel = CancellationToken::new();
        let store_ref = store.path().to_path_buf();
        let cancel_clone = cancel.clone();
        let run_clone = run_id.clone();
        *executor.generative.lock().unwrap() = Some(Box::new(move |ctx, _contract| {
            if ctx.role == NodeKind::Assess {
                // 模拟 Service::cancel 在运行中把数据库推进到 Cancelling（revision+1）后才取消令牌。
                let s = Store::new(store_ref.clone());
                let _ = s.transition(&run_clone, 2, RunState::Cancelling, "停止中", "run.cancelling", &serde_json::json!({}), "", "");
                cancel_clone.cancel();
                Some(Err(ExecError::aborted("已停止")))
            } else {
                None
            }
        }));
        let snapshot = futures_util::FutureExt::now_or_never(run_created_task(
            &deps(&executor, &store, &registry, &artifacts, cancel),
            "s1",
            &task_id,
            &run_id,
            "t",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.state, RunState::Cancelled);
    }

    #[test]
    fn rework_passes_prior_issues_to_worker_contract() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        coding_l2_assess(&executor);
        let seen_packs = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_clone = seen_packs.clone();
        *executor.generative_async.lock().unwrap() = Some(Box::new(move |ctx, contract| {
            let seen_clone = seen_clone.clone();
            Box::pin(async move {
                if ctx.role == NodeKind::Work {
                    seen_clone.lock().unwrap().push(contract.context_pack);
                    Some(Ok(work_result("修复后代码", true)))
                } else {
                    None
                }
            })
        }));
        generative_review(&executor, vec!["rework", "pass"], false);
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(
                &deps(&executor, &store, &registry, &artifacts, CancellationToken::new()),
                "s1",
                "修个 bug",
                "key-rework-pack",
            ))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded);
        let packs = seen_packs.lock().unwrap();
        assert_eq!(packs.len(), 2);
        assert!(packs[1].contains("返工缺陷单") && packs[1].contains("补边界测试"));
    }

    #[test]
    fn assessor_recommendation_routes_work_node_to_codex_and_dsh() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        let mut assess_json = assessment_json("coding", 1, "R1");
        assess_json["recommended_agent_id"] = serde_json::json!("builtin-codex");
        assess_json["recommended_reason"] = serde_json::json!("代码实现交由 Codex");
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assess_json);
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
        let seen_backends = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
        let seen_clone = seen_backends.clone();
        *executor.generative_async.lock().unwrap() = Some(Box::new(move |ctx, _contract| {
            let seen_clone = seen_clone.clone();
            Box::pin(async move {
                if ctx.role == NodeKind::Work {
                    seen_clone
                        .lock()
                        .unwrap()
                        .push((ctx.backend.clone(), ctx.agent.as_ref().map(|a| a.id.clone())));
                    Some(Ok(work_result("Codex 已实现并测试通过。", true)))
                } else {
                    None
                }
            })
        }));
        generative_review(&executor, vec!["pass"], false);
        let mut d = deps(&executor, &store, &registry, &artifacts, CancellationToken::new());
        d.routing.backend = "direct_api".into();
        d.routing.candidates = vec![
            CandidateAgent::synthetic("main", "主助理", "🐾", "direct_api", true),
            CandidateAgent::synthetic("builtin-codex", "codex", "⌘", "codex", false),
            CandidateAgent::synthetic("builtin-dsh", "dsh", "⌘", "dsh", false),
        ];
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(&d, "s1", "实现排序函数", "key-route-codex"))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert_eq!(
            seen_backends.lock().unwrap().as_slice(),
            &[("codex".to_string(), Some("builtin-codex".to_string()))]
        );
        let work1 = snapshot.nodes.iter().find(|n| n["node_id"] == "work-1").unwrap();
        assert_eq!(work1["assigned_agent_id"], "builtin-codex");
        assert_eq!(work1["assigned_backend"], "codex");
        let events = store.events_after(&snapshot.run_id, 0).unwrap();
        let assess_evt = events.iter().find(|e| e.kind == "assessment.completed").unwrap();
        assert_eq!(assess_evt.payload["recommended_agent_id"], "builtin-codex");
    }

    #[test]
    fn explicit_mention_overrides_assessor_and_routes_to_dsh() {
        let (_dir, store, artifacts, registry) = setup();
        let executor = FakeExecutor::new();
        let mut assess_json = assessment_json("research", 0, "R0");
        assess_json["recommended_agent_id"] = serde_json::json!("main");
        let mut assess_res = work_result("", false);
        assess_res.structured = Some(assess_json);
        executor.push(Some(NodeKind::Assess), Ok(assess_res));
        let seen = Arc::new(Mutex::new(Vec::<(String, Option<String>)>::new()));
        let seen_clone = seen.clone();
        *executor.generative.lock().unwrap() = Some(Box::new(move |ctx, _contract| {
            if ctx.role == NodeKind::Work {
                seen_clone
                    .lock()
                    .unwrap()
                    .push((ctx.backend.clone(), ctx.agent.as_ref().map(|a| a.id.clone())));
                Some(Ok(work_result("DSH 分析完成（来源：已核对）。", false)))
            } else {
                None
            }
        }));
        let mut d = deps(&executor, &store, &registry, &artifacts, CancellationToken::new());
        d.routing.backend = "direct_api".into();
        d.routing.candidates = vec![
            CandidateAgent::synthetic("main", "主助理", "🐾", "direct_api", true),
            CandidateAgent::synthetic("builtin-codex", "codex", "⌘", "codex", false),
            CandidateAgent::synthetic("builtin-dsh", "dsh", "⌘", "dsh", false),
        ];
        d.routing.mentioned_agent_ids = vec!["builtin-dsh".into()];
        let snapshot = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_task(&d, "s1", "@dsh 分析这个方案", "key-mention-dsh"))
            .unwrap();
        assert_eq!(snapshot.state, RunState::Succeeded, "status: {}", snapshot.status_message);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[("dsh".to_string(), Some("builtin-dsh".to_string()))]
        );
    }
}