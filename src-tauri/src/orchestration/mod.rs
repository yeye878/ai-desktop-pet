//! 智能调度模式（adaptive）：任务评估、执行分配、独立验收与返工。
//! 设计见 docs/adaptive-task-orchestration-plan.md；MVP = P0–P2 串行闭环。
//! 模型提出建议，程序执行约束；feature flag 默认关闭（orchestration_mode=direct）。

pub mod agents;
pub mod artifacts;
pub mod contracts;
pub mod executor;
pub mod persistence;
pub mod planner;
pub mod policy;
pub mod prompts;
pub mod registry;
pub mod review;
pub mod scheduler;
pub mod triage;
pub mod validation;
pub mod workspace;

use futures_util::FutureExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

use contracts::RunState;
use persistence::Store;
use registry::InvocationRegistry;

pub const MODE_DIRECT: &str = "direct";
pub const MODE_COLLABORATIVE: &str = "collaborative";
pub const MODE_ADAPTIVE: &str = "adaptive";

#[derive(Clone, Default)]
pub struct StartOptions {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission: String,
    pub mentioned_agents: Vec<String>,
}

pub struct Service {
    pub store: Store,
    pub registry: Arc<InvocationRegistry>,
    pub artifacts: artifacts::ArtifactStore,
    handles: StdMutex<HashMap<String, CancellationToken>>,
}

impl Service {
    pub fn new(db_path: PathBuf, data_root: PathBuf) -> Arc<Self> {
        let _ = persistence::ensure_schema(&db_path);
        let store = Store::new(db_path);
        // 重启核对：不确定的进行中 run 标 interrupted，不盲目重复外部动作。
        let _ = store.mark_interrupted_on_startup();
        Arc::new(Self {
            store,
            registry: Arc::new(InvocationRegistry::new()),
            artifacts: artifacts::ArtifactStore::new(artifacts::root_under(&data_root)),
            handles: StdMutex::new(HashMap::new()),
        })
    }

    /// 路由：发现所有可用智能体（主助理/内置 CLI/自定义），评估/审核限定支持只读强制的后端。
    async fn routing(
        app: &tauri::AppHandle,
        state: &crate::AppState,
        options: &StartOptions,
    ) -> scheduler::Routing {
        let backend_type = state.backend_type.lock().await.clone();
        let has_direct = state.direct_api_config.lock().await.is_some();
        let candidates = agents::discover_candidates(app, state).await;
        let mentioned_agent_ids =
            agents::resolve_mentioned_ids(&candidates, &options.mentioned_agents);
        let worker = candidates
            .iter()
            .find(|c| c.is_main && c.available)
            .map(|c| c.backend.clone())
            .unwrap_or_else(|| {
                if has_direct {
                    "direct_api".into()
                } else if backend_type.is_empty() {
                    "direct_api".into()
                } else {
                    backend_type
                }
            });
        let review = if has_direct { "direct_api" } else { "claude_code" };
        scheduler::Routing {
            backend: worker,
            review_backend: review.into(),
            model: options.model.clone().filter(|m| !m.trim().is_empty()),
            effort: options.effort.clone().filter(|e| !e.trim().is_empty()),
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            permission: options.permission.clone(),
            candidates,
            mentioned_agent_ids,
        }
    }

    pub async fn start(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        session_id: &str,
        request: &str,
        idempotency_key: &str,
    ) -> Result<(String, String), String> {
        self.start_with_options(app, session_id, request, idempotency_key, StartOptions::default())
            .await
    }

    pub async fn start_with_options(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        session_id: &str,
        request: &str,
        idempotency_key: &str,
        options: StartOptions,
    ) -> Result<(String, String), String> {
        let (task_id, run_id, created) = self
            .store
            .create_run(session_id, request, idempotency_key)
            .map_err(|e| e.to_string())?;
        if !created {
            return Ok((task_id, run_id));
        }
        let cancel = CancellationToken::new();
        if let Ok(mut handles) = self.handles.lock() {
            handles.insert(run_id.clone(), cancel.clone());
        }
        let _ = app.emit(
            "orchestration-run-started",
            serde_json::json!({"session_id": session_id, "task_id": task_id, "run_id": run_id}),
        );
        let state = app.state::<crate::AppState>();
        let routing = Self::routing(app, &state, &options).await;
        let service = self.clone();
        let app2 = app.clone();
        let session2 = session_id.to_string();
        let task2 = task_id.clone();
        let request2 = request.to_string();
        let run2 = run_id.clone();
        tauri::async_runtime::spawn(async move {
            let executor = executor::RuntimeExecutor { app: app2.clone() };
            let sink_app = app2.clone();
            let deps = scheduler::Deps {
                executor: &executor,
                store: &service.store,
                registry: &service.registry,
                artifacts: &service.artifacts,
                routing,
                policy_config: policy::PolicyConfig::default(),
                cancel,
                sink: Some(Box::new(move |event| {
                    let _ = sink_app.emit("orchestration-event", &event);
                })),
            };
            let task = std::panic::AssertUnwindSafe(scheduler::run_created_task(
                &deps, &session2, &task2, &run2, &request2,
            ));
            let outcome = task.catch_unwind().await;
            if let Ok(mut handles) = service.handles.lock() {
                handles.remove(&run2);
            }
            match outcome {
                Ok(Ok(snapshot)) => service.finalize(&app2, &snapshot).await,
                Ok(Err(message)) => {
                    let snapshot = service.ensure_terminal_after_error(&run2, &message);
                    if let Some(snapshot) = snapshot {
                        service.finalize(&app2, &snapshot).await;
                    } else {
                        let _ = app2.emit(
                            "orchestration-run-finished",
                            serde_json::json!({"run_id": run2, "session_id": session2, "state": "failed", "message": message}),
                        );
                    }
                }
                Err(_) => {
                    let msg = "智能调度内部执行异常退出".to_string();
                    let snapshot = service.ensure_terminal_after_error(&run2, &msg);
                    if let Some(snapshot) = snapshot {
                        service.finalize(&app2, &snapshot).await;
                    } else {
                        let _ = app2.emit(
                            "orchestration-run-finished",
                            serde_json::json!({"run_id": run2, "session_id": session2, "state": "failed", "message": msg}),
                        );
                    }
                }
            }
        });
        Ok((task_id, run_id))
    }

    /// 避免 Err/panic 时数据库停在非终态而永久锁死该会话。
    fn ensure_terminal_after_error(&self, run_id: &str, message: &str) -> Option<persistence::RunSnapshot> {
        if let Some((state, rev)) = self.store.run_state(run_id).ok().flatten() {
            if !state.is_terminal() {
                if state == RunState::Cancelling {
                    let _ = self.store.transition(run_id, rev, RunState::Cancelled, "已取消", "run.cancelled", &serde_json::json!({}), "", "");
                } else {
                    let _ = self.store.transition(run_id, rev, RunState::Failed, message, "run.failed", &serde_json::json!({"error": message}), "", "");
                }
            }
        }
        self.store.snapshot(run_id).ok().flatten()
    }

    /// 最终聊天结果只写入一次；各执行者草稿留在任务详情。
    async fn finalize(&self, app: &tauri::AppHandle, snapshot: &persistence::RunSnapshot) {
        let mut text = snapshot.status_message.clone();
        let mut chat_reply: Option<String> = None;
        if snapshot.state == RunState::Succeeded {
            if let Some(set_id) = snapshot.final_artifact_set_id.as_ref() {
                if let Some(manifest) = snapshot
                    .artifacts
                    .iter()
                    .rev()
                    .find(|a| a["artifact_set_id"].as_str() == Some(set_id.as_str()))
                {
                    if let Ok(manifest) = serde_json::from_value::<contracts::ArtifactManifest>(manifest.clone()) {
                        if let Ok(items) = self.artifacts.read_snapshot(&manifest) {
                            if let Some((_, content)) = items
                                .iter()
                                .find(|(id, _)| id == "reply" || id == "integrated")
                                .or_else(|| items.first())
                            {
                                text = executor::strip_worker_envelope(content);
                            }
                        }
                    }
                }
            }
            chat_reply = Some(text.clone());
        } else if matches!(
            snapshot.state,
            RunState::Blocked | RunState::Failed | RunState::ReviewFailed | RunState::WaitingInput
        ) {
            let reason = if snapshot.status_message.trim().is_empty() {
                "任务未能继续执行"
            } else {
                snapshot.status_message.trim()
            };
            let prefix = match snapshot.state {
                RunState::Blocked => "智能调度受阻",
                RunState::ReviewFailed => "智能调度审核未通过",
                RunState::WaitingInput => "智能调度需要你的补充",
                _ => "智能调度失败",
            };
            let notice = format!("⚠️ {prefix}：{reason}");
            text = notice.clone();
            chat_reply = Some(notice);
        }

        if let Some(reply) = chat_reply {
            let primary_node = snapshot
                .nodes
                .iter()
                .find(|n| n["node_id"].as_str() == Some("integrate"))
                .or_else(|| snapshot.nodes.iter().rev().find(|n| n["kind"].as_str() == Some("work")));
            let agent_id = primary_node
                .and_then(|n| n["assigned_agent_id"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let agent_name = primary_node
                .and_then(|n| n["assigned_agent_name"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let agent_avatar = primary_node
                .and_then(|n| n["assigned_agent_avatar"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            let events = self.store.events_after(&snapshot.run_id, 0).unwrap_or_default();
            let mut trace_lines = Vec::new();
            for event in &events {
                if event.kind == "assessment.completed" {
                    let diff = event.payload["difficulty"].as_str().unwrap_or("-");
                    let risk = event.payload["risk"].as_str().unwrap_or("-");
                    let assignee = event.payload["recommended_agent_name"].as_str().unwrap_or("主助理");
                    let reason = event.payload["recommended_reason"].as_str().unwrap_or("");
                    if reason.is_empty() {
                        trace_lines.push(format!("任务评估：{diff} / {risk} → 派给「{assignee}」执行"));
                    } else {
                        trace_lines.push(format!("任务评估：{diff} / {risk} → 派给「{assignee}」执行（{reason}）"));
                    }
                } else if event.kind == "review.completed" {
                    let verdict = event.payload["verdict"].as_str().unwrap_or("-");
                    trace_lines.push(format!("独立审核结论：{verdict}"));
                }
            }
            let thinking_summary = (!trace_lines.is_empty()).then(|| trace_lines.join("\n"));

            let state = app.state::<crate::AppState>();
            {
                let db = state.db.lock().await;
                let _ = db.save_message_with_agent_in(
                    &snapshot.session_id,
                    "assistant",
                    &reply,
                    thinking_summary.as_deref(),
                    agent_id.as_deref(),
                    agent_name.as_deref(),
                    agent_avatar.as_deref(),
                );
                let _ = db.touch_chat_session(&snapshot.session_id, None, Some(&reply.chars().take(80).collect::<String>()));
            }
            crate::notify_sessions_changed(app);
            crate::emit_ai_for(
                app,
                &snapshot.session_id,
                "sync-chat-message",
                serde_json::json!({
                    "role": "assistant",
                    "content": reply,
                    "thinking": thinking_summary,
                    "orchestration_run_id": snapshot.run_id,
                    "agent": agent_name.as_ref().map(|name| serde_json::json!({
                        "id": agent_id,
                        "name": name,
                        "avatar": agent_avatar,
                    })),
                }),
            );
        }
        let _ = app.emit(
            "orchestration-run-finished",
            serde_json::json!({
                "run_id": snapshot.run_id, "task_id": snapshot.task_id,
                "session_id": snapshot.session_id,
                "state": snapshot.state.as_str(), "text": text,
            }),
        );
    }

    pub fn cancel(&self, app: Option<&tauri::AppHandle>, run_id: &str, expected_revision: u64) -> Result<bool, String> {
        let has_live_handle = self
            .handles
            .lock()
            .map(|h| h.contains_key(run_id))
            .unwrap_or(false);
        let current_state = self.store.run_state(run_id).map_err(|e| e.to_string())?;
        let Some((state, current_rev)) = current_state else {
            return Ok(false);
        };
        if state.is_terminal() {
            return Ok(false);
        }
        // 已无后台运行协程的残留任务（如重启前 queued / 曾卡在 cancelling），直接推进到终态取消。
        if !has_live_handle {
            let rev = if state == RunState::Cancelling {
                current_rev
            } else {
                if current_rev != expected_revision {
                    return Ok(false);
                }
                let ok = self
                    .store
                    .transition(run_id, expected_revision, RunState::Cancelling, "停止中", "run.cancelling", &serde_json::json!({}), "", "")
                    .map_err(|e| e.to_string())?;
                if !ok {
                    return Ok(false);
                }
                expected_revision + 1
            };
            let _ = self
                .store
                .transition(run_id, rev, RunState::Cancelled, "已取消", "run.cancelled", &serde_json::json!({}), "", "");
            if let Some(app) = app {
                let session_id = self
                    .store
                    .snapshot(run_id)
                    .ok()
                    .flatten()
                    .map(|s| s.session_id)
                    .unwrap_or_default();
                let _ = app.emit(
                    "orchestration-run-finished",
                    serde_json::json!({"run_id": run_id, "session_id": session_id, "state": "cancelled", "text": ""}),
                );
            }
            return Ok(true);
        }

        let ok = self
            .store
            .transition(
                run_id,
                expected_revision,
                RunState::Cancelling,
                "停止中",
                "run.cancelling",
                &serde_json::json!({}),
                "",
                "",
            )
            .map_err(|e| e.to_string())?;
        if !ok {
            return Ok(false);
        }
        // 先持久化取消意图，再取消请求/工具并终止进程树。
        let pids = self.registry.cancel_run(run_id);
        for pid in pids {
            crate::kill_process_tree(pid);
        }
        if let Ok(handles) = self.handles.lock() {
            if let Some(token) = handles.get(run_id) {
                token.cancel();
            }
        }
        Ok(true)
    }
}

// ===== Tauri 命令（第 11 节）=====

async fn session_start_options(state: &crate::AppState, session_id: &str) -> StartOptions {
    let db = state.db.lock().await;
    let Ok(Some(session)) = db.get_chat_session(session_id) else {
        return StartOptions::default();
    };
    StartOptions {
        model: Some(session.model).filter(|m| !m.trim().is_empty()),
        effort: Some(session.reasoning_effort).filter(|e| !e.trim().is_empty()),
        permission: session.permission,
        mentioned_agents: Vec::new(),
    }
}

#[tauri::command]
pub async fn start_orchestrated_task(
    request: String,
    idempotency_key: String,
    session_id: Option<String>,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let session = crate::resolve_chat_session(&state, session_id.as_deref()).await?;
    let options = StartOptions {
        model: Some(session.model.clone()).filter(|m| !m.trim().is_empty()),
        effort: Some(session.reasoning_effort.clone()).filter(|e| !e.trim().is_empty()),
        permission: session.permission.clone(),
        mentioned_agents: crate::collaboration::mention_names(&request),
    };
    let (task_id, run_id) = state
        .orchestration
        .start_with_options(&app, &session.id, &request, &idempotency_key, options)
        .await?;
    Ok(serde_json::json!({"task_id": task_id, "run_id": run_id, "session_id": session.id}))
}

#[tauri::command]
pub async fn get_orchestrated_run(
    run_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let snapshot = state
        .orchestration
        .store
        .snapshot(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "run 不存在".to_string())?;
    serde_json::to_value(snapshot).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_orchestrated_runs(
    session_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let runs = state
        .orchestration
        .store
        .list_runs(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!(runs))
}

#[tauri::command]
pub async fn get_orchestration_events(
    run_id: String,
    after_seq: u64,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let events = state
        .orchestration
        .store
        .events_after(&run_id, after_seq)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(events).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_orchestrated_run(
    run_id: String,
    expected_revision: u64,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<bool, String> {
    state.orchestration.cancel(Some(&app), &run_id, expected_revision)
}

/// MVP 恢复：可恢复状态建立关联新 run（新尝试 ID），不复用已结束 invocation。
#[tauri::command]
pub async fn resume_orchestrated_run(
    run_id: String,
    expected_revision: u64,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let (run_state, revision) = state
        .orchestration
        .store
        .run_state(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "run 不存在".to_string())?;
    if revision != expected_revision {
        return Err("revision 过期，请刷新后重试".into());
    }
    if !run_state.is_resumable() {
        return Err("该状态不可恢复；终态请新建任务".into());
    }
    let snapshot = state
        .orchestration
        .store
        .snapshot(&run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "run 不存在".to_string())?;
    let _ = state
        .orchestration
        .store
        .transition(&run_id, revision, RunState::Cancelled, "恢复：建立关联新 run", "run.cancelled", &serde_json::json!({"resumed_into": "new-run"}), "", "");
    let key = format!("resume-{run_id}");
    let options = session_start_options(&state, &snapshot.session_id).await;
    let (task_id, new_run) = state
        .orchestration
        .start_with_options(&app, &snapshot.session_id, &snapshot.original_request, &key, options)
        .await?;
    Ok(serde_json::json!({"task_id": task_id, "run_id": new_run, "previous_run_id": run_id}))
}

/// 需求版本事务递增；停止旧版本运行，新版本重新验收（MVP 串行：整 run 停止）。
#[tauri::command]
pub async fn amend_orchestrated_task(
    task_id: String,
    expected_revision: u32,
    change: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let next = state
        .orchestration
        .store
        .amend_task(&task_id, expected_revision, &change)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task 不存在或 revision 过期".to_string())?;
    let session_id = state
        .orchestration
        .store
        .task_session_id(&task_id)
        .ok()
        .flatten()
        .unwrap_or_default();
    let active = state
        .orchestration
        .store
        .list_runs(&session_id)
        .unwrap_or_default();
    for run in active {
        if run["task_id"].as_str() == Some(task_id.as_str()) {
            let run_id = run["run_id"].as_str().unwrap_or_default();
            let rev = run["state_revision"].as_u64().unwrap_or(0);
            let _ = state.orchestration.cancel(Some(&app), run_id, rev);
        }
    }
    Ok(serde_json::json!({"task_revision": next}))
}

#[tauri::command]
pub async fn answer_orchestration_interaction(
    interaction_id: String,
    answer: String,
) -> Result<serde_json::Value, String> {
    // MVP 澄清走 amend（补充要求）；独立交互恢复在 P4 开放。
    let _ = (interaction_id, answer);
    Err("当前版本请通过补充要求（amend）回应澄清；交互恢复在后续阶段开放".into())
}

#[tauri::command]
pub async fn get_orchestration_artifact(
    artifact_set_id: String,
    revision: u32,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let conn = persistence::open(state.orchestration.store.path()).map_err(|e| e.to_string())?;
    let manifest_json: String = conn
        .query_row(
            "SELECT manifest_json FROM orchestration_artifacts WHERE artifact_set_id = ?1 AND revision = ?2",
            rusqlite::params![artifact_set_id, revision],
            |row| row.get(0),
        )
        .map_err(|_| "产物版本不存在".to_string())?;
    let manifest: contracts::ArtifactManifest =
        serde_json::from_str(&manifest_json).map_err(|e| e.to_string())?;
    let items = state
        .orchestration
        .artifacts
        .read_snapshot(&manifest)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"manifest": manifest, "items": items.into_iter().map(|(id, content)| serde_json::json!({"id": id, "content": content})).collect::<Vec<_>>()}))
}

#[tauri::command]
pub async fn get_orchestration_mode(
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    let db = state.db.lock().await;
    Ok(db
        .get_setting("orchestration_mode")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| MODE_DIRECT.to_string()))
}

#[tauri::command]
pub async fn set_orchestration_mode(
    mode: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    if !matches!(mode.as_str(), MODE_DIRECT | MODE_COLLABORATIVE | MODE_ADAPTIVE) {
        return Err("未知组织方式".into());
    }
    let db = state.db.lock().await;
    db.save_setting("orchestration_mode", &mode).map_err(|e| e.to_string())?;
    drop(db);
    let _ = app.emit("orchestration-mode-changed", &mode);
    Ok(mode)
}