//! 多智能体协作模式（/team）：多个智能体真正并行分工合作完成一个任务。
//!
//! 流程：规划器（拆分子任务）→ 并行执行（每个智能体独立工具循环）→ 综合汇报。
//!
//! 并发安全设计：
//! - 每个智能体有独立的取消令牌（父令牌的 child），停止时级联取消；
//! - 确认弹窗全局串行化（前端 pendingConfirm 是单槽，多智能体同时请求授权会互相覆盖）；
//! - 鼠标键盘 / 浏览器会话等独占 OS 资源的工具按类别加全局互斥，防止并行注入输入；
//! - 每个智能体只能看到自己 allowed_tools 白名单内的工具（能力限制语义）。

use crate::direct_api::agent::{
    append_active_thinking, append_visual_tool_message, emit_tool_event_labeled,
    mode_requires_confirmation, push_tool_message, request_tool_confirmation,
    request_user_answer, tool_output_for_log, tool_output_for_message, tool_summary,
};
use crate::direct_api::api_client::{
    call_chat_completions_non_stream_with_timeout, DirectApiConfig,
};
use crate::direct_api::tools::{execute_tool, tool_definitions_for};
use crate::storage::Agent;
use crate::AppState;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tauri::{Emitter, Manager};

/// 单个协作智能体的最大对话轮数（工具循环步数上限）
const MAX_COLLAB_TURNS: u32 = 24;
/// 单个智能体整体墙钟上限
const AGENT_WALL_CLOCK: Duration = Duration::from_secs(600);
/// 非流式调用：等待响应头（推理模型 = 等待完整生成）的时限
const HEADERS_TIMEOUT: Duration = Duration::from_secs(170);
/// 客户端总超时
const CLIENT_TIMEOUT: Duration = Duration::from_secs(180);
/// 综合阶段单份成员报告的最大字符数
const MAX_REPORT_CHARS: usize = 6000;

/// 确认弹窗全局闸门：前端授权面板是单槽，多智能体并行时必须排队弹出
static CONFIRM_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// 鼠标/键盘/窗口焦点：全局唯一输入流，并行注入会产生混沌
static INPUT_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// 浏览器会话：单实例共享配置目录，并行操作互相干扰
static BROWSER_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn os_resource_mutex(tool: &str) -> Option<&'static tokio::sync::Mutex<()>> {
    match tool {
        "computer_mouse" | "computer_keyboard" | "window_focus" => Some(&INPUT_MUTEX),
        t if t.starts_with("browser_") => Some(&BROWSER_MUTEX),
        _ => None,
    }
}

/// 识别协作指令：消息以 /team、/组队 或 /协作 开头。
/// 返回去掉指令前缀后的任务描述（可能为空串）。
pub fn parse_team_command(message: &str) -> Option<String> {
    let trimmed = message.trim_start();
    for prefix in ["/team", "/组队", "/协作"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            // 必须紧跟空白或结尾，避免误伤 /teamwork 之类的词
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return Some(rest.trim().to_string());
            }
        }
    }
    None
}

fn collab_progress(state: &tauri::State<'_, AppState>, app_handle: &tauri::AppHandle, text: &str) {
    append_active_thinking(state, &format!("{text}\n"));
    let _ = app_handle.emit("ai-thinking", format!("{text}\n"));
}

fn collab_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(CLIENT_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())
}

/// 运行一次非流式补全（无工具），带取消监听。
/// 取消时返回 Err("已中止")。
async fn plain_completion(
    client: &reqwest::Client,
    config: &DirectApiConfig,
    system: &str,
    user: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<String, String> {
    let messages = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": user }),
    ];
    tokio::select! {
        res = call_chat_completions_non_stream_with_timeout(
            client, config, &messages, None, Some(HEADERS_TIMEOUT),
        ) => {
            let res = res?;
            Ok(res.content.unwrap_or_default())
        }
        _ = cancel.cancelled() => Err("已中止".to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct PlanSubtask {
    agent: String,
    subtask: String,
}

#[derive(Debug, Deserialize)]
struct PlanResult {
    #[serde(default)]
    subtasks: Vec<PlanSubtask>,
}

fn extract_json_object(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end > start {
        Some(&trimmed[start..=end])
    } else {
        None
    }
}

/// 规划阶段：把任务拆成每个成员一个子任务。
/// 规划调用失败（非取消）时退化为「人人拿到完整任务」，保证功能可用。
async fn plan_subtasks(
    client: &reqwest::Client,
    config: &DirectApiConfig,
    task: &str,
    agents: &[Agent],
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<(Agent, String)>, String> {
    let fallback = || {
        agents
            .iter()
            .map(|a| (a.clone(), task.to_string()))
            .collect::<Vec<_>>()
    };

    let mut roster = String::new();
    for agent in agents {
        let tools = if agent.allowed_tools.is_empty() {
            "全部工具".to_string()
        } else {
            agent.allowed_tools.join(",")
        };
        roster.push_str(&format!(
            "- {}：{}（可用工具：{}）\n",
            agent.name,
            agent.description.trim(),
            tools
        ));
    }

    let system = "你是多智能体团队的任务调度器。把用户任务拆分成团队成员各自独立完成的子任务。\
要求：子任务互补不重叠、合起来完整覆盖目标；每个成员恰好一个子任务；子任务要具体、可独立执行、写明交付物。\
必须严格输出纯 JSON 对象（无 Markdown 标记）：{\"subtasks\":[{\"agent\":\"成员名\",\"subtask\":\"子任务描述\"}]}";
    let user = format!("【任务】\n{task}\n\n【团队成员】\n{roster}");

    let raw = match plain_completion(client, config, system, &user, cancel).await {
        Ok(text) => text,
        Err(e) => {
            if cancel.is_cancelled() {
                return Err(e);
            }
            return Ok(fallback());
        }
    };

    let parsed = extract_json_object(&raw)
        .and_then(|json_str| serde_json::from_str::<PlanResult>(json_str).ok());

    let Some(plan) = parsed else {
        return Ok(fallback());
    };

    // 校验成员名；未拿到子任务的成员回退为完整任务
    let mut result: Vec<(Agent, String)> = Vec::new();
    for agent in agents {
        let subtask = plan
            .subtasks
            .iter()
            .find(|s| s.agent.trim() == agent.name)
            .map(|s| s.subtask.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| task.to_string());
        result.push((agent.clone(), subtask));
    }
    if result.is_empty() {
        return Ok(fallback());
    }
    Ok(result)
}

struct AgentReport {
    agent: Agent,
    subtask: String,
    outcome: Result<String, String>,
}

/// 协作主入口（在独立 tokio 任务中运行）
pub async fn run_collab(
    app_handle: tauri::AppHandle,
    task: String,
    agents: Vec<Agent>,
    _attachments: Vec<crate::direct_api::UserAttachment>,
) {
    let state = app_handle.state::<AppState>();
    let task = if task.trim().is_empty() {
        "用户未给出具体任务描述，请团队成员基于各自专长协商一个有价值的目标并完成。".to_string()
    } else {
        task
    };

    let base_config = match state.direct_api_config.lock().await.clone() {
        Some(config) if !config.api_key.trim().is_empty() || !config.base_url.trim().is_empty() => {
            config
        }
        _ => {
            if let Ok(mut active) = state.active_chat.lock() {
                active.active = None;
            }
            let _ = app_handle.emit(
                "ai-error",
                json!({ "message": "协作模式需要先在设置中配置模型接口。", "thinking": null, "aborted": false }),
            );
            return;
        }
    };

    let client = match collab_client() {
        Ok(client) => client,
        Err(e) => {
            if let Ok(mut active) = state.active_chat.lock() {
                active.active = None;
            }
            let _ = app_handle.emit(
                "ai-error",
                json!({ "message": format!("创建 HTTP 客户端失败: {e}"), "thinking": null, "aborted": false }),
            );
            return;
        }
    };

    // 注册取消令牌（run_id 保证只清理自己的槽位）
    let run_id = crate::next_run_id();
    let parent_token = tokio_util::sync::CancellationToken::new();
    {
        let mut abort = state.abort_token.lock().await;
        *abort = Some((run_id, parent_token.clone()));
    }

    let team_names = agents
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    collab_progress(&state, &app_handle, &format!("【协作】任务启动：{task}"));
    collab_progress(&state, &app_handle, &format!("【协作】团队成员：{team_names}"));

    // ===== 阶段 1：规划分工 =====
    collab_progress(&state, &app_handle, "【协作】正在拆分子任务…");
    let assignments = match plan_subtasks(&client, &base_config, &task, &agents, &parent_token).await {
        Ok(list) => list,
        Err(e) => {
            finish_collab_error(&app_handle, &state, run_id, &format!("任务规划失败: {e}"), parent_token.is_cancelled()).await;
            return;
        }
    };
    for (agent, subtask) in &assignments {
        collab_progress(&state, &app_handle, &format!("【协作】分工 → {}：{}", agent.name, truncate_chars(subtask, 80)));
    }

    // ===== 阶段 2：并行执行 =====
    collab_progress(&state, &app_handle, "【协作】成员开始并行工作…");
    let mut futures = Vec::new();
    for (agent, subtask) in assignments {
        let app = app_handle.clone();
        let config = base_config.clone();
        let token = parent_token.child_token();
        let task_ctx = task.clone();
        let teammates = agents
            .iter()
            .filter(|a| a.id != agent.id)
            .map(|a| format!("{}：{}", a.name, a.description.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        futures.push(async move {
            let outcome = tokio::time::timeout(
                AGENT_WALL_CLOCK,
                run_collab_agent(&app, &agent, &task_ctx, &subtask, &teammates, config, token.clone()),
            )
            .await
            .unwrap_or_else(|_| Err("执行超时（10 分钟），已中断本成员的工作".to_string()));
            AgentReport { agent, subtask, outcome }
        });
    }
    let reports = futures_util::future::join_all(futures).await;

    // 逐份落库 + 推送（保持完成顺序；每份带智能体归属）
    for report in &reports {
        match &report.outcome {
            Ok(text) if !text.trim().is_empty() => {
                collab_progress(&state, &app_handle, &format!("【协作】{} 已完成子任务", report.agent.name));
                {
                    let db = state.db.lock().await;
                    let _ = db.save_message_with_agent(
                        "assistant",
                        text,
                        None,
                        Some(report.agent.id.as_str()),
                        Some(report.agent.name.as_str()),
                        Some(report.agent.avatar.as_str()),
                    );
                }
                let _ = app_handle.emit(
                    "sync-chat-message",
                    json!({
                        "role": "assistant",
                        "content": text,
                        "agent": { "id": report.agent.id, "name": report.agent.name, "avatar": report.agent.avatar },
                    }),
                );
            }
            Ok(_) => {
                collab_progress(&state, &app_handle, &format!("【协作】{} 没有产出内容", report.agent.name));
            }
            Err(e) => {
                collab_progress(&state, &app_handle, &format!("【协作】{} 执行失败：{}", report.agent.name, e));
            }
        }
    }

    if parent_token.is_cancelled() {
        finish_collab_error(&app_handle, &state, run_id, "协作任务已被用户中止", true).await;
        return;
    }

    // ===== 阶段 3：综合汇报 =====
    collab_progress(&state, &app_handle, "【协作】正在汇总各成员成果…");
    let succeeded_any = reports.iter().any(|r| r.outcome.is_ok());
    if !succeeded_any {
        let errors = reports
            .iter()
            .map(|r| format!("【{}】{}", r.agent.name, r.outcome.as_ref().err().cloned().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n");
        finish_collab_error(&app_handle, &state, run_id, &format!("所有成员均未完成任务：\n{errors}"), false).await;
        return;
    }

    let mut digest = String::new();
    for report in &reports {
        digest.push_str(&format!(
            "【{}】子任务：{}\n结果：{}\n\n",
            report.agent.name,
            truncate_chars(&report.subtask, 200),
            match &report.outcome {
                Ok(text) => truncate_chars(text.trim(), MAX_REPORT_CHARS),
                Err(e) => format!("（失败：{e}）"),
            },
        ));
    }

    let system = "你是多智能体团队的汇报汇总者。把各成员的执行报告整合成一份给用户的最终答复：\
结论先行；用【成员名】标注各自的贡献与产出位置；指出成员之间的分歧或未完成项；不要复读原文，要提炼。\
如果成员创建了文件或执行了命令，在答复末尾集中列出关键产物清单。";
    let user = format!("【原始任务】\n{task}\n\n【各成员报告】\n{digest}");

    let synthesis = match plain_completion(&client, &base_config, system, &user, &parent_token).await {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => "（汇总模型未返回内容，请直接查看上方各成员的报告。）".to_string(),
        Err(e) => {
            finish_collab_error(&app_handle, &state, run_id, &format!("汇总失败: {e}"), parent_token.is_cancelled()).await;
            return;
        }
    };

    // 收尾：保存综合答复并按正常流程结束本轮
    {
        let db = state.db.lock().await;
        let _ = db.save_message_with_thinking("assistant", &synthesis, None);
    }
    {
        let mut abort = state.abort_token.lock().await;
        if abort.as_ref().map(|(id, _)| *id == run_id).unwrap_or(false) {
            *abort = None;
        }
    }
    {
        let mut behavior = state.behavior.lock().await;
        behavior.set_state(crate::behavior::PetState::Speaking);
    }
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let _ = app_handle.emit(
        "ai-finished",
        json!({
            "text": synthesis,
            "thinking": null,
            "agent_id": null,
            "agent_name": null,
            "agent_avatar": null,
        }),
    );
}

async fn finish_collab_error(
    app_handle: &tauri::AppHandle,
    state: &tauri::State<'_, AppState>,
    run_id: u64,
    message: &str,
    aborted: bool,
) {
    {
        let mut abort = state.abort_token.lock().await;
        if abort.as_ref().map(|(id, _)| *id == run_id).unwrap_or(false) {
            *abort = None;
        }
    }
    if let Ok(mut active) = state.active_chat.lock() {
        active.active = None;
    }
    let _ = app_handle.emit(
        "ai-error",
        json!({ "message": message, "thinking": null, "aborted": aborted }),
    );
}

/// 单个协作智能体的执行循环：非流式 + 工具调用，独立取消令牌。
/// 复用主对话的确认/执行管线（含 execution_mode、会话授权缓存、ask_user）。
#[allow(clippy::too_many_arguments)]
async fn run_collab_agent(
    app_handle: &tauri::AppHandle,
    agent: &Agent,
    task: &str,
    subtask: &str,
    teammates: &str,
    base_config: DirectApiConfig,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<String, String> {
    let state = app_handle.state::<AppState>();
    let label = agent.name.clone();

    let mut config = base_config;
    let model = agent.model.trim();
    if !model.is_empty() {
        config.model = model.to_string();
    }

    let persona = if agent.system_prompt.trim().is_empty() {
        "你是一名专业智能体。".to_string()
    } else {
        agent.system_prompt.trim().to_string()
    };
    let system_prompt = format!(
        "{persona}\n\n【协作任务背景】\n总任务：{task}\n你的子任务：{subtask}\n其他成员（仅供参考，不要替他们工作）：\n{teammates}\n\
你是团队中被并行派出的成员，独立完成自己的子任务后直接给出最终报告（交付物、关键结论、未尽事项）。\
报告用中文、结构清晰、结论先行。不要询问用户是否开始，直接执行。"
    );

    let mut api_messages = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": format!("请开始执行你的子任务：{subtask}") }),
    ];

    let client = collab_client()?;
    let tools = if config.execution_mode == "plan" {
        None
    } else if agent.allowed_tools.is_empty() {
        Some(tool_definitions_for(None))
    } else {
        Some(tool_definitions_for(Some(agent.allowed_tools.as_slice())))
    };

    let mut full_text = String::new();

    for turn in 1..=MAX_COLLAB_TURNS {
        if cancel.is_cancelled() {
            return Err("已中止".to_string());
        }

        let result = call_chat_completions_non_stream_with_timeout(
            &client,
            &config,
            &api_messages,
            tools.clone(),
            Some(HEADERS_TIMEOUT),
        )
        .await?;

        let turn_text = result.content.unwrap_or_default();
        if !turn_text.trim().is_empty() {
            full_text.push_str(&turn_text);
            full_text.push('\n');
        }

        if result.tool_calls.is_empty() {
            return Ok(full_text);
        }

        // 与主循环一致：assistant 工具调用消息 + 逐个执行
        let tool_calls_json: Vec<serde_json::Value> = result
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id.clone().unwrap_or_default(),
                    "type": "function",
                    "function": { "name": tc.function.name, "arguments": tc.function.arguments }
                })
            })
            .collect();
        api_messages.push(json!({
            "role": "assistant",
            "content": if turn_text.is_empty() { None } else { Some(turn_text.as_str()) },
            "tool_calls": tool_calls_json,
        }));

        for tc in &result.tool_calls {
            if cancel.is_cancelled() {
                return Err("已中止".to_string());
            }
            let tc_id = tc.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
            let tc_name = tc.function.name.as_str();
            let tc_args: serde_json::Value =
                serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
            let pretty_args = serde_json::to_string_pretty(&tc_args)
                .unwrap_or_else(|_| tc.function.arguments.clone());

            let call_line = format!("【{label}】[调用工具] {}\n", tool_summary(tc_name, &tc_args));
            append_active_thinking(&state, &call_line);
            let _ = app_handle.emit("ai-thinking", &call_line);
            emit_tool_event_labeled(app_handle, &tc_id, tc_name, "requested", &tc_args, None, None, Some(&label));

            if config.execution_mode == "plan" {
                let output = format!("计划模式已阻止执行工具 {tc_name}。");
                emit_tool_event_labeled(app_handle, &tc_id, tc_name, "skipped", &tc_args, Some(&output), Some(false), Some(&label));
                push_tool_message(&mut api_messages, &tc_id, tc_name, &output);
                continue;
            }

            if tc_name == "ask_user" {
                let mut thinking = String::new();
                let output = request_user_answer(
                    &state, app_handle, &mut thinking, &tc_id, &tc_args, Some(&label), &cancel,
                )
                .await;
                append_active_thinking(&state, &thinking);
                let result_line = format!("【{label}】[用户回答]\n{}\n", tool_output_for_log(&output));
                append_active_thinking(&state, &result_line);
                push_tool_message(&mut api_messages, &tc_id, tc_name, &output);
                append_visual_tool_message(&mut api_messages, tc_name, &output);
                continue;
            }

            let already_approved = {
                let approved_tool_types = state.approved_tool_types.lock().await;
                approved_tool_types.contains(tc_name)
            };
            let latest_config = state.direct_api_config.lock().await.clone();
            let effective_config = latest_config.as_ref().unwrap_or(&config);
            let requires_confirm = mode_requires_confirmation(effective_config, tc_name, already_approved);

            let approved = if requires_confirm {
                // 前端授权面板是单槽：多智能体并行请求授权必须全局排队
                let _gate = CONFIRM_GATE.lock().await;
                let mut thinking = String::new();
                let approved = request_tool_confirmation(
                    &state,
                    app_handle,
                    &mut thinking,
                    &tc_id,
                    tc_name,
                    &tc_args,
                    &pretty_args,
                    Some(&label),
                    &cancel,
                )
                .await;
                append_active_thinking(&state, &thinking);
                approved
            } else {
                true
            };

            let tool_output = if approved {
                emit_tool_event_labeled(app_handle, &tc_id, tc_name, "running", &tc_args, None, Some(true), Some(&label));
                let output = if let Some(mutex) = os_resource_mutex(tc_name) {
                    let _guard = mutex.lock().await;
                    execute_tool(tc_name, &tc_args, &config.search_provider, cancel.clone()).await
                } else {
                    execute_tool(tc_name, &tc_args, &config.search_provider, cancel.clone()).await
                };
                let output_for_event = tool_output_for_log(&output);
                emit_tool_event_labeled(app_handle, &tc_id, tc_name, "completed", &tc_args, Some(&output_for_event), Some(true), Some(&label));
                if tc_name == "create_scheduled_task" || tc_name == "list_scheduled_tasks" {
                    let _ = app_handle.emit("scheduled-tasks-changed", json!({}));
                }
                if tc_name == "save_memory" || tc_name == "delete_memory" {
                    let _ = app_handle.emit("memories-changed", json!({}));
                }
                output
            } else {
                "用户拒绝了此工具的操作权限。请调整方案，改用不需要该权限的方式完成子任务。".to_string()
            };

            let result_line = format!("【{label}】[执行结果]\n{}\n", tool_output_for_log(&tool_output));
            append_active_thinking(&state, &result_line);
            let output_for_message = tool_output_for_message(&tool_output);
            push_tool_message(&mut api_messages, &tc_id, tc_name, &output_for_message);
            append_visual_tool_message(&mut api_messages, tc_name, &tool_output);
        }

        if turn == MAX_COLLAB_TURNS {
            return Ok(format!(
                "{full_text}\n（已达到最大工作步数 {MAX_COLLAB_TURNS}，提前汇报当前进展。）"
            ));
        }
    }

    Ok(full_text)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut value: String = input.chars().take(max_chars).collect();
    value.push('…');
    value
}
