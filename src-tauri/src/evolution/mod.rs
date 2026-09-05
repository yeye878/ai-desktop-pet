use crate::direct_api::{call_chat_completions_non_stream_with_timeout, DirectApiConfig};
use crate::skills::Skill;
use crate::storage::{Agent, EvolutionInsight, EvolutionLog, EvolutionProposal, NewScheduledTask, TaskRepeat};
use crate::AppState;
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct ReflectionResult {
    #[serde(default)]
    insights: Vec<RawInsight>,
    #[serde(default)]
    proposals: Vec<RawProposal>,
}

#[derive(Debug, Deserialize)]
struct RawInsight {
    category: String, // 'preference' | 'work_domain' | 'coding_style' | 'negative_constraint' | 'habit'
    content: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_confidence() -> f64 {
    0.85
}

#[derive(Debug, Deserialize)]
struct RawProposal {
    proposal_type: String, // 'create_agent' | 'create_skill' | 'schedule_task' | 'update_guideline'
    title: String,
    rationale: String,
    payload: serde_json::Value,
}

/// 复盘互斥守卫：Drop 时自动复位标志，确保任何返回路径（含出错）都会释放复盘锁
struct ReflectionGuard<'a>(&'a AtomicBool);

impl Drop for ReflectionGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 执行元认知复盘分析，提炼画像准则与进化提案
pub async fn run_reflection_analysis(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    config: &DirectApiConfig,
) -> Result<serde_json::Value, String> {
    // 互斥：手动复盘与后台自动复盘共用此入口，并发执行会在读-改-写窗口内产生重复准则/提案
    if state.is_reflecting.swap(true, Ordering::SeqCst) {
        return Err("复盘任务正在进行中，请稍候再试。".to_string());
    }
    let _reflection_guard = ReflectionGuard(&state.is_reflecting);

    let (recent_messages, active_insights, all_insights, existing_agents, existing_skills, _existing_tasks, rejected_proposals) = {
        let db = state.db.lock().await;
        let messages = db.get_recent_messages(50).unwrap_or_default();
        let active_insights = db.get_active_evolution_insights().unwrap_or_default();
        let all_insights = db.list_evolution_insights().unwrap_or_default();
        let agents = db.list_agents().unwrap_or_default();
        let skills = db.list_skills().unwrap_or_default();
        let tasks = db.get_all_scheduled_tasks().unwrap_or_default();
        let rejected_props = db.list_evolution_proposals(Some("rejected")).unwrap_or_default();
        (messages, active_insights, all_insights, agents, skills, tasks, rejected_props)
    };

    if config.api_key.trim().is_empty() && config.base_url.trim().is_empty() {
        return Err("未配置 API Key 或 Base URL，无法运行自进化反思引擎。请先在设置中配置模型接口。".to_string());
    }

    if recent_messages.is_empty() {
        return Err("近期暂无足够的对话记录可供复盘进化。请先与桌宠进行日常交流吧！".to_string());
    }

    // 格式化近期对话文本（get_recent_messages 按时间倒序返回，需反转为正序再喂给模型）
    let mut history_text = String::new();
    for msg in recent_messages.iter().rev() {
        let role_label = match msg.role.as_str() {
            "user" => "用户",
            "assistant" => "桌宠",
            _ => "系统",
        };
        let agent_tag = msg
            .agent_name
            .as_deref()
            .map(|name| format!("【{}】", name))
            .unwrap_or_default();
        history_text.push_str(&format!("{role_label}{agent_tag}: {}\n", msg.content.trim()));
    }

    // 格式化已激活准则
    let existing_insights_text = if active_insights.is_empty() {
        "（暂无）".to_string()
    } else {
        active_insights
            .iter()
            .map(|i| format!("- [{}] {}", i.category, i.content))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // 格式化已废弃/拒绝的准则与提案
    let rejected_insights: Vec<_> = all_insights.iter().filter(|i| i.status != "active").collect();
    let mut rejected_text = String::new();
    for ri in &rejected_insights {
        rejected_text.push_str(&format!("- [已归档/拒绝准则] {}\n", ri.content));
    }
    for rp in &rejected_proposals {
        rejected_text.push_str(&format!("- [已拒绝提案] {}\n", rp.title));
    }
    if rejected_text.is_empty() {
        rejected_text = "（暂无）".to_string();
    }

    let existing_agents_text = if existing_agents.is_empty() {
        "（暂无自定义智能体）".to_string()
    } else {
        existing_agents
            .iter()
            .map(|a| format!("- {} ({}): {}", a.name, a.avatar, a.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let existing_skills_text = if existing_skills.is_empty() {
        "（暂无自定义技能）".to_string()
    } else {
        existing_skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let prompt = format!(
        r#"你是一个资深的 AI 桌宠元认知反思引擎（Self-Evolution Reflection Engine）。
你的使命是通过深入分析用户与桌宠近期的对话历史，敏锐提炼用户的真实偏好、工作方向、纠错教训与行为准则，并主动构思进化提案（如衍生专属特化智能体、技能或定时工作流）。

【近期对话历史】
{history_text}

【当前已习得的准则】
{existing_insights_text}

【当前已存在的智能体与技能】
智能体：
{existing_agents_text}
技能：
{existing_skills_text}

【已废弃/用户明确拒绝过的准则与提案（严禁再次重复提出）】
{rejected_text}

【深度分析任务要求】
请从对话中敏锐提炼：
1. insights（画像准则与负面避坑）：
   - 用户纠错与负面约束识别（特别重要！）：仔细检查对话中用户是否表达过修正、纠错、反驳或不满（如包含“不对”、“别这样”、“禁止”、“必须”、“改用”、“重写”、“请用简洁方式”等）。一旦发现，必须提取为 category: "negative_constraint" 的准则，明确写清【禁止做什么】以及【应该如何做】。
   - 用户的技术栈、常用工具、习惯偏好（category: preference / work_domain / coding_style / habit）。
   - 若准则与【当前已习得的准则】高度重合，依然列出，系统会自动增加其命中验证次数与置信度。
2. proposals（结构化进化提案，仅在有明确且显著价值时提出，没有则为空数组 []）：
   - create_agent: 如果用户有明显重复的垂直专业需求（如“Rust优化专家”、“小红书文案润色”、“学术翻译润色”），构思专属特化智能体。
     payload 字段要求：name (名字，无空格), avatar (单个 Emoji), description (一句话描述), system_prompt (专属提示词), allowed_tools (如 ["read_file", "write_file", "run_command", "web_search", "code_search"])。
   - create_skill: 如果用户有固定套路式的工作流（如“一键生成周报”、“代码安全审计”），构思专属 Skill。
     payload 字段要求：name (技能名), description (一句话描述), system_prompt (专属注入提示词), allowed_tools (工具列表)。
   - schedule_task: 如果用户表现出固定的日常周期习惯（如每日早晨看资讯、傍晚提醒整理），提议一个定时任务。
     payload 字段要求：title (任务标题), note (说明/提示词), due_at (下次触发时间戳秒数，默认今天或明天), repeat ("daily"|"weekly"|"once")。

【输出格式】
必须严格输出合法的 JSON 对象，不包含任何 Markdown 代码块标记（不要加 ```json ），直接输出纯 JSON：
{{
  "insights": [
    {{
      "category": "negative_constraint",
      "content": "在回答编程问题时禁止冗长寒暄，必须先直接给出完整可运行的代码及核心原理说明",
      "confidence": 0.95
    }}
  ],
  "proposals": [
    {{
      "proposal_type": "create_agent",
      "title": "创建专属「Rust 优化专家」智能体",
      "rationale": "检测到用户近期频繁探讨 Rust 内存生命周期与宏编写问题",
      "payload": {{
        "name": "Rust专家",
        "avatar": "🦀",
        "description": "专注 Rust 高性能架构、unsafe 审计与宏编写",
        "system_prompt": "你是一个资深的 Rust 架构师...",
        "allowed_tools": ["read_file", "write_file", "run_command", "code_search"]
      }}
    }}
  ]
}}"#
    );

    let messages = vec![json!({
        "role": "user",
        "content": prompt,
    })];

    // 复盘 prompt 体量大且推理模型思考时间长（实测 60~120s+），
    // 必须给足总超时，否则每次复盘都会在保存前被掐断，表现为「点击无产出」
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let res = call_chat_completions_non_stream_with_timeout(
        &client,
        config,
        &messages,
        None,
        Some(std::time::Duration::from_secs(290)),
    )
    .await?;
    let raw_content = res.content.as_deref().unwrap_or_default();
    let content = clean_json_output(raw_content);

    let parsed: ReflectionResult = serde_json::from_str(&content).map_err(|e| {
        format!("解析模型反思输出 JSON 失败: {e}，原始返回: {}", truncate_str(raw_content, 200))
    })?;

    let mut new_insights_count = 0;
    let mut updated_insights_count = 0;
    let mut new_proposals_count = 0;

    let now = chrono::Utc::now().timestamp();

    // 落地保存 Insights 与 Proposals
    {
        let db = state.db.lock().await;
        let mut existing_all_insights = db.list_evolution_insights().unwrap_or_default();

        for insight in parsed.insights {
            let content_clean = insight.content.trim();
            if content_clean.is_empty() {
                continue;
            }

            // 检查相似准则是否已存在；快照随保存动态更新，防止同一批输出内部的近重复条目重复入库
            if let Some(idx) = existing_all_insights.iter().position(|i| {
                i.category == insight.category
                    && (i.content == content_clean || calculate_similarity(&i.content, content_clean) > 0.8)
            }) {
                let existing = existing_all_insights[idx].clone();
                let updated = EvolutionInsight {
                    id: existing.id,
                    category: existing.category,
                    content: content_clean.to_string(),
                    confidence: (existing.confidence + 0.08).min(1.0),
                    hit_count: existing.hit_count + 1,
                    status: existing.status,
                    created_at: existing.created_at,
                    updated_at: now,
                };
                let _ = db.save_evolution_insight(&updated);
                existing_all_insights[idx] = updated;
                updated_insights_count += 1;
            } else {
                let new_insight = EvolutionInsight {
                    id: Uuid::new_v4().to_string(),
                    category: insight.category,
                    content: content_clean.to_string(),
                    confidence: insight.confidence.clamp(0.5, 1.0),
                    hit_count: 1,
                    status: "active".to_string(),
                    created_at: now,
                    updated_at: now,
                };
                let _ = db.save_evolution_insight(&new_insight);
                existing_all_insights.push(new_insight);
                new_insights_count += 1;
            }
        }

        // 保存进化提案
        let mut existing_proposals = db.list_evolution_proposals(None).unwrap_or_default();
        for proposal in parsed.proposals {
            let title_clean = proposal.title.trim();
            if title_clean.is_empty() {
                continue;
            }
            // 避免与已有或被拒绝过的提案重复
            if existing_proposals.iter().any(|p| p.title == title_clean) {
                continue;
            }

            let new_prop = EvolutionProposal {
                id: Uuid::new_v4().to_string(),
                proposal_type: proposal.proposal_type,
                title: title_clean.to_string(),
                rationale: proposal.rationale.trim().to_string(),
                payload_json: serde_json::to_string(&proposal.payload).unwrap_or_else(|_| "{}".to_string()),
                status: "pending".to_string(),
                created_at: now,
                applied_at: None,
            };
            let _ = db.save_evolution_proposal(&new_prop);
            existing_proposals.push(new_prop);
            new_proposals_count += 1;
        }

        // 记录复盘日志
        let summary_text = format!(
            "深度复盘：提炼 {} 条新认知，强化 {} 条已有认知，提出 {} 项进化提案",
            new_insights_count, updated_insights_count, new_proposals_count
        );
        let log = EvolutionLog {
            id: Uuid::new_v4().to_string(),
            proposal_id: None,
            category: "reflection_run".to_string(),
            summary: summary_text.clone(),
            before_state_json: "{}".to_string(),
            after_state_json: json!({
                "new_insights": new_insights_count,
                "updated_insights": updated_insights_count,
                "new_proposals": new_proposals_count,
            }).to_string(),
            created_at: now,
        };
        let _ = db.create_evolution_log(&log);
    }

    let _ = app_handle.emit("evolution-updated", json!({
        "new_insights_count": new_insights_count,
        "updated_insights_count": updated_insights_count,
        "new_proposals_count": new_proposals_count,
    }));

    Ok(json!({
        "success": true,
        "new_insights_count": new_insights_count,
        "updated_insights_count": updated_insights_count,
        "new_proposals_count": new_proposals_count,
        "message": format!("复盘完成！习得 {} 条新认知，强化 {} 条已有认知，生成 {} 项进化提案。", new_insights_count, updated_insights_count, new_proposals_count)
    }))
}

/// 采纳并执行一项进化提案（如创建智能体、创建技能、创建定时任务等）
pub async fn apply_evolution_proposal(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    proposal_id: &str,
) -> Result<EvolutionLog, String> {
    let (proposal, now) = {
        let db = state.db.lock().await;
        let p = db
            .get_evolution_proposal(proposal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("未找到 ID 为 {proposal_id} 的进化提案"))?;
        if p.status == "applied" {
            return Err("该进化提案此前已被采纳执行。".to_string());
        }
        (p, chrono::Utc::now().timestamp())
    };

    let payload: serde_json::Value = serde_json::from_str(&proposal.payload_json)
        .map_err(|e| format!("提案载荷格式错误: {e}"))?;

    let mut before_state = json!({});
    let mut after_state = json!({});

    {
        let db = state.db.lock().await;
        match proposal.proposal_type.as_str() {
            "create_agent" => {
                let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("新智能体").trim().to_string();
                let avatar = payload.get("avatar").and_then(|v| v.as_str()).unwrap_or("🤖").trim().to_string();
                let description = payload.get("description").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let system_prompt = payload.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let allowed_tools: Vec<String> = payload.get("allowed_tools")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                let agent_id = Uuid::new_v4().to_string();
                let agent = Agent {
                    id: agent_id.clone(),
                    name: name.clone(),
                    avatar,
                    description,
                    system_prompt,
                    model,
                    allowed_tools,
                    created_at: now,
                    updated_at: now,
                };
                db.save_agent(&agent).map_err(|e| format!("保存智能体失败: {e}"))?;
                after_state = serde_json::to_value(&agent).unwrap_or_default();
            }
            "create_skill" => {
                let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("新技能").trim().to_string();
                let description = payload.get("description").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let system_prompt = payload.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                let allowed_tools: Vec<String> = payload.get("allowed_tools")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();

                let skill_id = format!("skill-{}", Uuid::new_v4().simple());
                let skill = Skill {
                    id: skill_id.clone(),
                    name,
                    description,
                    system_prompt,
                    allowed_tools,
                    keywords: Vec::new(),
                    is_active: false,
                    created_at: now,
                    updated_at: now,
                };
                db.save_skill(&skill).map_err(|e| format!("保存技能失败: {e}"))?;
                after_state = serde_json::to_value(&skill).unwrap_or_default();
            }
            "schedule_task" => {
                let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("新任务").trim();
                let note = payload.get("note").and_then(|v| v.as_str()).unwrap_or("").trim();
                // 兼容秒级时间戳与 RFC3339 字符串两种格式；无效或已过期的时间统一回退到 1 小时后，
                // 避免幻觉时间让调度器（15 秒轮询一次）在采纳后立即触发任务
                let mut due_at = payload.get("due_at").and_then(|v| v.as_i64()).or_else(|| {
                    payload.get("due_at").and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.timestamp())
                }).unwrap_or(now + 3600);
                if due_at <= now {
                    due_at = now + 3600;
                }
                let repeat_str = payload.get("repeat").and_then(|v| v.as_str()).unwrap_or("once");
                let repeat = TaskRepeat::from_str(repeat_str);

                let new_task = NewScheduledTask {
                    title,
                    note,
                    due_at,
                    repeat,
                    enabled: true,
                };
                let task_id = db.save_scheduled_task(new_task).map_err(|e| format!("创建定时任务失败: {e}"))?;
                after_state = json!({ "id": task_id, "title": title, "note": note, "due_at": due_at });
            }
            "update_guideline" => {
                let category = payload.get("category").and_then(|v| v.as_str()).unwrap_or("preference").trim();
                let content = payload.get("content").and_then(|v| v.as_str()).unwrap_or("").trim();
                let confidence = payload.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.9);

                if content.is_empty() {
                    return Err("提案内容为空，无法保存为协作准则。".to_string());
                }

                // 与反思路径保持一致：先做相似度去重。命中已有准则时合并强化，
                // 并把原准则快照记入 before_state，供回滚时恢复，避免回滚误删既有准则
                let all_insights = db.list_evolution_insights().unwrap_or_default();
                if let Some(existing) = all_insights.iter().find(|i| {
                    i.category == category
                        && (i.content == content || calculate_similarity(&i.content, content) > 0.8)
                }) {
                    before_state = serde_json::to_value(existing).unwrap_or_default();
                    let merged = EvolutionInsight {
                        id: existing.id.clone(),
                        category: existing.category.clone(),
                        content: content.to_string(),
                        confidence: (existing.confidence.max(confidence) + 0.08).min(1.0),
                        hit_count: existing.hit_count + 1,
                        status: "active".to_string(),
                        created_at: existing.created_at,
                        updated_at: now,
                    };
                    db.save_evolution_insight(&merged).map_err(|e| format!("保存准则失败: {e}"))?;
                    after_state = serde_json::to_value(&merged).unwrap_or_default();
                } else {
                    let insight = EvolutionInsight {
                        id: Uuid::new_v4().to_string(),
                        category: category.to_string(),
                        content: content.to_string(),
                        confidence: confidence.clamp(0.5, 1.0),
                        hit_count: 1,
                        status: "active".to_string(),
                        created_at: now,
                        updated_at: now,
                    };
                    db.save_evolution_insight(&insight).map_err(|e| format!("保存准则失败: {e}"))?;
                    after_state = serde_json::to_value(&insight).unwrap_or_default();
                }
            }
            _ => {
                return Err(format!("未知的提案类型: {}", proposal.proposal_type));
            }
        }

        // 更新提案状态为已执行
        db.update_evolution_proposal_status(proposal_id, "applied").map_err(|e| e.to_string())?;

        // 记录进化日志
        let log = EvolutionLog {
            id: Uuid::new_v4().to_string(),
            proposal_id: Some(proposal_id.to_string()),
            category: proposal.proposal_type.clone(),
            summary: format!("采纳进化提案：「{}」", proposal.title),
            before_state_json: before_state.to_string(),
            after_state_json: after_state.to_string(),
            created_at: now,
        };
        db.create_evolution_log(&log).map_err(|e| e.to_string())?;

        let _ = app_handle.emit("evolution-updated", ());
        let _ = app_handle.emit("agents-changed", ());
        let _ = app_handle.emit("skills-changed", ());
        let _ = app_handle.emit("scheduled-tasks-changed", ());
        Ok(log)
    }
}

/// 拒绝/忽略一项进化提案
pub async fn reject_evolution_proposal(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    proposal_id: &str,
) -> Result<(), String> {
    let db = state.db.lock().await;
    db.update_evolution_proposal_status(proposal_id, "rejected")
        .map_err(|e| e.to_string())?;
    let _ = app_handle.emit("evolution-updated", ());
    Ok(())
}

/// 回滚某次进化操作
pub async fn rollback_evolution_log(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    log_id: &str,
) -> Result<(), String> {
    let db = state.db.lock().await;
    let log = db
        .get_evolution_log(log_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到 ID 为 {log_id} 的进化日志"))?;

    let now = chrono::Utc::now().timestamp();

    match log.category.as_str() {
        "create_agent" => {
            if let Ok(after) = serde_json::from_str::<serde_json::Value>(&log.after_state_json) {
                if let Some(agent_id) = after.get("id").and_then(|v| v.as_str()) {
                    let _ = db.delete_agent(agent_id);
                }
            }
        }
        "create_skill" => {
            if let Ok(after) = serde_json::from_str::<serde_json::Value>(&log.after_state_json) {
                if let Some(skill_id) = after.get("id").and_then(|v| v.as_str()) {
                    let _ = db.delete_skill(skill_id);
                }
            }
        }
        "schedule_task" => {
            if let Ok(after) = serde_json::from_str::<serde_json::Value>(&log.after_state_json) {
                if let Some(task_id) = after.get("id").and_then(|v| v.as_i64()) {
                    let _ = db.delete_scheduled_task(task_id);
                }
            }
        }
        "update_guideline" => {
            // 合并式采纳（强化已有准则）：before_state 记录了原准则，回滚时恢复；
            // 新建式采纳：before_state 为空对象，回滚时删除该准则。
            // 若准则在采纳后又被修改过（用户手动编辑或后续复盘继续合并），
            // 则跳过恢复/删除，避免回滚静默覆盖新内容（丢失更新防护）
            if let Ok(orig) = serde_json::from_str::<EvolutionInsight>(&log.before_state_json) {
                if insight_untouched_since(&db, &orig.id, &log.after_state_json) {
                    let _ = db.save_evolution_insight(&orig);
                }
            } else if let Ok(after) = serde_json::from_str::<serde_json::Value>(&log.after_state_json) {
                if let Some(insight_id) = after.get("id").and_then(|v| v.as_str()) {
                    if insight_untouched_since(&db, insight_id, &log.after_state_json) {
                        let _ = db.delete_evolution_insight(insight_id);
                    }
                }
            }
        }
        _ => {}
    }

    if let Some(prop_id) = log.proposal_id.as_deref() {
        let _ = db.update_evolution_proposal_status(prop_id, "rolled_back");
    }

    let rollback_log = EvolutionLog {
        id: Uuid::new_v4().to_string(),
        proposal_id: log.proposal_id.clone(),
        category: "rollback".to_string(),
        summary: format!("回滚操作：{}", log.summary),
        before_state_json: log.after_state_json.clone(),
        after_state_json: "{}".to_string(),
        created_at: now,
    };
    let _ = db.create_evolution_log(&rollback_log);

    let _ = app_handle.emit("evolution-updated", ());
    let _ = app_handle.emit("agents-changed", ());
    let _ = app_handle.emit("skills-changed", ());
    let _ = app_handle.emit("scheduled-tasks-changed", ());
    Ok(())
}

/// 判断准则自某次进化日志之后是否未被修改（当前内容与日志 after_state 记录一致）。
/// 用于回滚前的丢失更新防护：被修改过的不恢复/不删除。
fn insight_untouched_since(
    db: &crate::storage::Database,
    insight_id: &str,
    after_state_json: &str,
) -> bool {
    let after = match serde_json::from_str::<EvolutionInsight>(after_state_json) {
        Ok(a) => a,
        Err(_) => return false,
    };
    match db.get_evolution_insight(insight_id) {
        Ok(Some(current)) => current.content == after.content,
        _ => false,
    }
}

/// 把已激活的进化准则注入 System Prompt
pub fn attach_evolution_prompt(system_prompt: String, insights: &[EvolutionInsight]) -> String {
    if insights.is_empty() {
        return system_prompt;
    }

    let mut guidelines = String::new();
    for insight in insights {
        let label = match insight.category.as_str() {
            "preference" => "用户偏好",
            "work_domain" => "工作领域",
            "coding_style" => "编码风格",
            "negative_constraint" => "负面约束与避坑",
            "habit" => "协作习惯",
            _ => "习惯认知",
        };
        guidelines.push_str(&format!("- [{label}] {}\n", insight.content.trim()));
    }

    format!(
        "{system_prompt}

【自进化认知与用户协作准则】
这是你根据与用户的历史互动自进化提炼出的协作默契与工作习惯，请在后续对话与工具调用中严格自然遵守：
{guidelines}
（遵守以上准则即可，无需在回复中刻意说明你应用了自进化认知）"
    )
}

fn clean_json_output(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(start) = trimmed.find("```json") {
        let rest = &trimmed[start + 7..];
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    } else if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    }

    if let (Some(first_brace), Some(last_brace)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if last_brace > first_brace {
            return trimmed[first_brace..=last_brace].to_string();
        }
    }
    trimmed.to_string()
}

fn truncate_str(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    // 按字节截断必须回退到最近的 UTF-8 字符边界，否则切在中文等多字节字符中间会 panic
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 基于字符 bigram 集合的 Jaccard 相似度。
/// 相比单字集合，bigram 保留了字序信息，对中文短句的区分度显著更好，
/// 可减少「含义不同但常用字重叠」的误判合并，也能识别换词复述的近重复。
fn calculate_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let a_grams = char_bigrams(a);
    let b_grams = char_bigrams(b);
    if a_grams.is_empty() || b_grams.is_empty() {
        return 0.0;
    }
    let intersection = a_grams.intersection(&b_grams).count();
    let union = a_grams.union(&b_grams).count();
    intersection as f64 / union as f64
}

fn char_bigrams(s: &str) -> std::collections::HashSet<(char, char)> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}
