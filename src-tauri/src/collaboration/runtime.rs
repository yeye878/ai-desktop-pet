use super::*;
use crate::{
    agent_turn::{TurnError, TurnOutput},
    direct_api, AppState,
};
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

fn owns_run(state: &AppState, id: &str) -> bool {
    state
        .collaboration_state
        .lock()
        .ok()
        .and_then(|value| value.as_ref().map(|value| value.run_id == id))
        .unwrap_or(false)
}

fn publish(app: &tauri::AppHandle, value: CollaborationState) {
    let state = app.state::<AppState>();
    if let Ok(mut current) = state.collaboration_state.lock() {
        if current
            .as_ref()
            .is_some_and(|current| current.run_id != value.run_id)
        {
            return;
        }
        *current = Some(value.clone());
    }
    let _ = app.emit("collaboration-state", value);
}

pub async fn run(
    app: tauri::AppHandle,
    message: String,
    attachments: Vec<direct_api::UserAttachment>,
    selected: Vec<Agent>,
    allow_handoffs: bool,
    run_id: String,
    cancel: CancellationToken,
) {
    let state = app.state::<AppState>();
    let all = {
        let db = state.db.lock().await;
        match db.list_agents() {
            Ok(agents) => catalog(agents),
            Err(error) => {
                finish(
                    &app,
                    &run_id,
                    "failed",
                    0,
                    format!("读取智能体失败：{error}"),
                )
                .await;
                return;
            }
        }
    };
    let mut queue = Queue::new(selected, &message);
    let mut failures = 0;
    while !cancel.is_cancelled() && owns_run(&state, &run_id) {
        let Some(job) = queue.next() else {
            break;
        };
        if cancel.is_cancelled() || !owns_run(&state, &run_id) {
            break;
        }
        state.approved_tool_types.lock().await.clear();
        if let Ok(mut active) = state.active_chat.lock() {
            if let Some(active) = active.active.as_mut() {
                active.thinking.clear();
            }
        }
        publish(
            &app,
            CollaborationState {
                run_id: run_id.clone(),
                status: "running".into(),
                active_agent: Some((&job.agent).into()),
                queued: queue
                    .pending
                    .iter()
                    .map(|job| (&job.agent).into())
                    .collect(),
                turn: queue.turns,
                max_turns: MAX_TURNS,
                message: format!(
                    "{} 正在处理来自 {} 的任务",
                    job.agent.name, job.requested_by
                ),
            },
        );
        let result = execute_job(&app, &job, &all, &attachments, allow_handoffs, &cancel).await;
        if cancel.is_cancelled() || !owns_run(&state, &run_id) {
            break;
        }
        let (reply, failed) = match result {
            Ok(reply) => (reply, false),
            Err(error) if error.aborted => break,
            Err(error) => {
                failures += 1;
                (
                    TurnOutput {
                        text: format!("执行失败：{}", error.message),
                        thinking: error.thinking,
                    },
                    true,
                )
            }
        };
        // Persist before scheduling successors: the next independent invocation
        // must read this reply from the same shared conversation.
        let saved = {
            let db = state.db.lock().await;
            if cancel.is_cancelled() || !owns_run(&state, &run_id) {
                break;
            }
            db.save_message_with_agent(
                "assistant",
                &reply.text,
                if reply.thinking.is_empty() {
                    None
                } else {
                    Some(&reply.thinking)
                },
                Some(&job.agent.id),
                Some(&job.agent.name),
                Some(&job.agent.avatar),
            )
        };
        if let Err(error) = saved {
            finish(
                &app,
                &run_id,
                "failed",
                queue.turns,
                format!("保存协同回复失败：{error}"),
            )
            .await;
            return;
        }
        let _ = app.emit("collaboration-reply", serde_json::json!({
            "run_id": run_id, "turn_id": format!("{run_id}:{}", queue.turns),
            "text": reply.text, "thinking": reply.thinking,
            "agent_id": job.agent.id, "agent_name": job.agent.name, "agent_avatar": job.agent.avatar,
            "failed": failed,
        }));
        if allow_handoffs && !failed {
            queue.handoff(&job.agent, &reply.text, &all);
        }
    }
    let (status, note) = if cancel.is_cancelled() {
        ("aborted", "协同已停止，剩余任务已取消。".to_string())
    } else if queue.limited {
        ("limited", format!("本轮已达到协同上限（共 {MAX_TURNS} 次、每角色 {MAX_AGENT_TURNS} 次），可再次 @ 角色继续。"))
    } else if failures > 0 {
        (
            "completed",
            format!("协同结束，{failures} 个任务失败；详情见对应角色的回复。"),
        )
    } else {
        ("completed", "协同已完成。".to_string())
    };
    finish(&app, &run_id, status, queue.turns, note).await;
}

pub(crate) async fn finish(
    app: &tauri::AppHandle,
    run_id: &str,
    status: &str,
    turns: usize,
    message: String,
) {
    let state = app.state::<AppState>();
    if !owns_run(&state, run_id) {
        return;
    }
    state.pending_confirms.lock().await.clear();
    state.pending_questions.lock().await.clear();
    state.approved_tool_types.lock().await.clear();
    if let Some(pid) = state.active_ai_pid.lock().await.take() {
        crate::kill_process_tree(pid);
    }
    *state.abort_token.lock().await = None;
    state
        .behavior
        .lock()
        .await
        .set_state(crate::behavior::PetState::Idle);
    publish(
        app,
        CollaborationState {
            run_id: run_id.into(),
            status: status.into(),
            active_agent: None,
            queued: vec![],
            turn: turns,
            max_turns: MAX_TURNS,
            message,
        },
    );
    // Release the request gate last, after all async cleanup and final events.
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    };
}

async fn execute_job(
    app: &tauri::AppHandle,
    job: &Job,
    all: &[Agent],
    attachments: &[direct_api::UserAttachment],
    allow_handoffs: bool,
    cancel: &CancellationToken,
) -> Result<TurnOutput, TurnError> {
    let state = app.state::<AppState>();
    let start_id = *state
        .chat_start_id
        .lock()
        .map_err(|_| TurnError::failed("无法读取会话起点", ""))?;
    let (history, skills, skill, custom_prompt, profile) = {
        let db = state.db.lock().await;
        let history = db
            .get_conversation_messages(start_id)
            .map_err(|error| TurnError::failed(error.to_string(), ""))?;
        let profile = if job.agent.backend == "direct_api" {
            if job.agent.api_profile_id.is_empty() {
                Some(crate::active_api_profile(&db))
            } else {
                Some(
                    crate::load_api_profiles(&db)
                        .into_iter()
                        .find(|profile| profile.id == job.agent.api_profile_id)
                        .ok_or_else(|| {
                            TurnError::failed(
                                "此角色的 API 配置已不存在，请在智能体工坊重新选择。",
                                "",
                            )
                        })?,
                )
            }
        } else {
            None
        };
        (
            history,
            db.list_skills().unwrap_or_default(),
            db.get_active_skill().unwrap_or(None),
            db.get_setting("custom_system_prompt")
                .unwrap_or(None)
                .unwrap_or_default(),
            profile,
        )
    };
    let message = format!("【完整共享会话记录（按时间顺序，作为上下文）】\n{}\n\n【当前交给你的任务】\n请求者：{}\n{}",
        transcript(&history), job.requested_by, job.task);
    let personality = state.personality.lock().await.clone();
    let profession = state.profession.lock().await.clone();
    let mut system = crate::openclaw::build_system_prompt(&personality, &profession);
    system = crate::attach_memory_prompt(system);
    system = crate::attach_registered_apps_prompt(system, &state).await;
    system = crate::attach_custom_prompt(system, &custom_prompt);
    system = crate::attach_skill_prompt(system, skill.as_ref(), &skills, profile.is_some()).await;
    system.push_str("\n\n");
    system.push_str(&collaboration_prompt(&job.agent, all, allow_handoffs));
    if cancel.is_cancelled() {
        return Err(TurnError::aborted("", ""));
    }
    if let Some(mut config) = profile {
        crate::apply_agent_model(&mut config, std::slice::from_ref(&job.agent));
        system = crate::attach_runtime_identity_prompt(system, &config);
        system = crate::attach_execution_mode_prompt(system, &config);
        system = crate::attach_word_and_ask_prompt(system);
        system = crate::attach_agent_factory_prompt(system);
        system = crate::attach_coding_prompt(system);
        system = crate::attach_computer_use_prompt(system);
        system = crate::attach_search_prompt(system);
        system = crate::attach_weather_prompt(system);
        let grants =
            crate::collect_turn_tool_grants(std::slice::from_ref(&job.agent), skill.as_ref());
        return tokio::select! {
            _ = cancel.cancelled() => Err(TurnError::aborted("", "")),
            result = direct_api::execute_direct_api_turn(app.clone(), message, config, system, vec![], attachments.to_vec(), None,
                Some(crate::AgentContext { agents: vec![job.agent.clone()] }), grants, cancel.clone(), job.agent.api_profile_id.is_empty()) => result,
        };
    }
    execute_cli(app, &job.agent.backend, &message, &system, cancel).await
}

async fn execute_cli(
    app: &tauri::AppHandle,
    backend: &str,
    message: &str,
    system: &str,
    cancel: &CancellationToken,
) -> Result<TurnOutput, TurnError> {
    // Each role invocation has its own CLI context. Reconstruct it from the full
    // transcript instead of borrowing the ordinary pet's CLI session.
    let mut child = match backend {
        "claude_code" => {
            crate::openclaw::ClaudeAdapter::new()
                .spawn_streaming(app, message, system)
                .await
        }
        "codex" => {
            crate::codex::CodexAdapter::default()
                .spawn_streaming(app, message, system)
                .await
        }
        _ => {
            return Err(TurnError::failed(
                format!("不支持的角色后端：{backend}"),
                "",
            ))
        }
    }
    .map_err(|error| TurnError::failed(format!("启动 {backend} 失败：{error}"), ""))?;
    let state = app.state::<AppState>();
    let pid = child.id();
    {
        let mut active_pid = state.active_ai_pid.lock().await;
        if cancel.is_cancelled() {
            if let Some(pid) = pid {
                crate::kill_process_tree(pid);
            }
            let _ = child.wait().await;
            return Err(TurnError::aborted("", ""));
        }
        *active_pid = pid;
    }
    let on_thinking = |delta: &str| {
        if cancel.is_cancelled() {
            return;
        }
        if let Ok(mut active) = state.active_chat.lock() {
            if let Some(active) = active.active.as_mut() {
                active.thinking.push_str(delta);
            }
        }
        let _ = app.emit("ai-thinking", delta);
    };
    let read = async {
        if backend == "codex" {
            crate::codex::read_stream_with_activity(
                &mut child,
                on_thinking,
                |text| {
                    if !cancel.is_cancelled() {
                        let _ = app.emit("ai-answer-delta", serde_json::json!({ "text": text }));
                    }
                },
                |tool| {
                    if !cancel.is_cancelled() {
                        let _ = app.emit("ai-tool-event", tool);
                    }
                },
            )
            .await
        } else {
            crate::openclaw::read_stream_with_activity(
                &mut child,
                on_thinking,
                |text| {
                    if !cancel.is_cancelled() {
                        let _ = app.emit("ai-answer-delta", serde_json::json!({ "text": text }));
                    }
                },
                |tool| {
                    if !cancel.is_cancelled() {
                        let _ = app.emit("ai-tool-event", tool);
                    }
                },
            )
            .await
        }
    };
    let parsed = tokio::select! {
        biased;
        _ = cancel.cancelled() => None,
        result = read => Some(result),
    };
    if parsed.is_none() {
        if let Some(pid) = pid {
            crate::kill_process_tree(pid);
        }
    }
    let exit = child.wait().await;
    {
        let mut active_pid = state.active_ai_pid.lock().await;
        if *active_pid == pid {
            *active_pid = None;
        }
    }
    if cancel.is_cancelled() {
        return Err(TurnError::aborted("", ""));
    }
    let parsed = parsed
        .ok_or_else(|| TurnError::aborted("", ""))?
        .map_err(|error| TurnError::failed(error.to_string(), ""))?;
    if parsed.is_error
        || !exit.map(|exit| exit.success()).unwrap_or(false)
        || parsed.full_text.trim().is_empty()
    {
        return Err(TurnError::failed(
            parsed
                .error_msg
                .unwrap_or_else(|| "CLI 未正常完成回复，请检查本机登录或配置。".into()),
            &parsed.full_thinking,
        ));
    }
    Ok(TurnOutput {
        text: parsed.full_text,
        thinking: parsed.full_thinking,
    })
}
