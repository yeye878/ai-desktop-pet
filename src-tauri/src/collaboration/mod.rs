mod runtime;
pub(crate) use runtime::{execute_delegated_agent, finish, finish_delegation};
pub use runtime::{run, run_queue};

use crate::storage::{Agent, ChatMessage};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_TURNS: usize = 12;
pub const MAX_AGENT_TURNS: usize = 4;

pub fn builtin_agents() -> Vec<Agent> {
    [
        (
            "builtin-claude",
            "claude",
            "Claude Code",
            "claude_code",
            "本机 Claude Code 独立智能体：擅长多文件代码库理解、架构重构、复杂问题排查与深度代码审查（自有文件/终端工具，独立上下文）",
        ),
        (
            "builtin-codex",
            "codex",
            "Codex",
            "codex",
            "本机 Codex 独立智能体：擅长工程实现、代码编写与修改、补充单元测试及命令行构建验证（自有沙箱与终端工具）",
        ),
        (
            "builtin-dsh",
            "dsh",
            "DeepSeek Harness",
            "dsh",
            "本机 DeepSeek Harness 独立智能体：擅长长链路推演、技术方案论证、跨模块复核与结构化分析",
        ),
    ]
    .into_iter()
    .map(|(id, name, label, backend, description)| Agent {
        id: id.into(),
        name: name.into(),
        avatar: "⌘".into(),
        description: description.into(),
        system_prompt: format!(
            "你是团队中的 {label}。独立完成分配给你的任务，需要其他角色协助时明确 @ 对方。"
        ),
        backend: backend.into(),
        api_profile_id: String::new(),
        is_builtin: true,
        model: String::new(),
        allowed_tools: vec![],
        created_at: 0,
        updated_at: 0,
    })
    .collect()
}

pub fn reserved_agent(id: &str, name: &str) -> bool {
    builtin_agents()
        .iter()
        .any(|agent| agent.id == id || agent.name.eq_ignore_ascii_case(name))
}

pub fn catalog(mut custom: Vec<Agent>) -> Vec<Agent> {
    let mut all = builtin_agents();
    all.append(&mut custom);
    all
}

pub fn mention_name(agent: &Agent) -> &str {
    // Keep legacy custom roles named "claude"/"codex" callable by their ID.
    if !agent.is_builtin && reserved_agent("", &agent.name) {
        &agent.id
    } else {
        &agent.name
    }
}

pub fn resolve_agents(all: &[Agent], names: &[String]) -> Vec<Agent> {
    let mut seen = HashSet::new();
    names
        .iter()
        .filter_map(|name| {
            all.iter().find(|agent| {
                agent.id == name.trim()
                    || if agent.is_builtin {
                        agent.name.eq_ignore_ascii_case(name.trim())
                    } else {
                        agent.name == name.trim()
                    }
            })
        })
        .filter(|agent| seen.insert(agent.id.clone()))
        .cloned()
        .collect()
}

/// Server-side parsing also supports IPC callers without the frontend mention list.
pub fn mention_names(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|word| word.strip_prefix('@'))
        .filter(|name| !name.is_empty() && !name.contains('@'))
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Serialize, Debug)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub avatar: String,
}
impl From<&Agent> for Identity {
    fn from(agent: &Agent) -> Self {
        Self {
            id: agent.id.clone(),
            name: agent.name.clone(),
            avatar: agent.avatar.clone(),
        }
    }
}

#[derive(Clone, Serialize, Debug)]
pub struct CollaborationState {
    pub run_id: String,
    pub status: String,
    pub active_agent: Option<Identity>,
    pub queued: Vec<Identity>,
    pub turn: usize,
    pub max_turns: usize,
    pub message: String,
    /// 发起这次协同的会话：协同状态全局只有一份，用它拒绝其它会话同时发起协同。
    pub session_id: String,
}

#[derive(Clone)]
pub struct Job {
    pub agent: Agent,
    pub task: String,
    pub requested_by: String,
}

pub struct Queue {
    pub pending: VecDeque<Job>,
    pub turns: usize,
    pub limited: bool,
    seen: HashSet<(String, String)>,
    counts: HashMap<String, usize>,
}

impl Queue {
    fn empty() -> Self {
        Self {
            pending: VecDeque::new(),
            turns: 0,
            limited: false,
            seen: HashSet::new(),
            counts: HashMap::new(),
        }
    }

    pub fn new(agents: Vec<Agent>, task: &str) -> Self {
        let mut queue = Self::empty();
        for agent in agents {
            queue.enqueue(Job {
                agent,
                task: task.into(),
                requested_by: "用户".into(),
            });
        }
        queue
    }

    pub fn from_handoffs(requests: Vec<(Agent, String)>, requested_by: &str) -> Self {
        let mut queue = Self::empty();
        for (agent, task) in requests {
            queue.enqueue(Job {
                agent,
                task,
                requested_by: requested_by.into(),
            });
        }
        queue
    }

    fn enqueue(&mut self, job: Job) {
        let key = (
            job.agent.id.clone(),
            job.task.split_whitespace().collect::<Vec<_>>().join(" "),
        );
        if !self.seen.insert(key) {
            return;
        }
        let count = self.counts.entry(job.agent.id.clone()).or_default();
        if *count >= MAX_AGENT_TURNS || self.turns + self.pending.len() >= MAX_TURNS {
            self.limited = true;
            return;
        }
        *count += 1;
        self.pending.push_back(job);
    }

    pub fn next(&mut self) -> Option<Job> {
        let next = self.pending.pop_front()?;
        self.turns += 1;
        Some(next)
    }

    pub fn handoff(&mut self, from: &Agent, text: &str, all: &[Agent]) {
        for (agent, task) in handoffs(text, all) {
            if agent.id != from.id {
                self.enqueue(Job {
                    agent,
                    task,
                    requested_by: from.name.clone(),
                });
            }
        }
    }
}

/// Only explicit standalone request lines can schedule work. Quoted/code examples
/// and incidental mentions in prose are shared context, not execution requests.
pub fn handoffs(text: &str, all: &[Agent]) -> Vec<(Agent, String)> {
    let mut fence: Option<&str> = None;
    let mut quoted = false;
    let mut requests = Vec::new();
    for raw in text.lines() {
        if raw.starts_with("    ") || raw.starts_with('\t') {
            continue;
        }
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            let marker = &line[..3];
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if line.contains("[QUOTE]") {
            quoted = true;
        }
        if line.contains("[/QUOTE]") {
            quoted = false;
            continue;
        }
        if fence.is_some() || quoted || line.starts_with('>') {
            continue;
        }
        let Some(rest) = line.strip_prefix('@') else {
            continue;
        };
        // Longest names first so @dev does not consume @developer.
        let mut names: Vec<_> = all.iter().collect();
        names.sort_by_key(|agent| std::cmp::Reverse(mention_name(agent).len()));
        for agent in names {
            let name = mention_name(agent);
            let Some(prefix) = rest.get(..name.len()) else {
                continue;
            };
            let matches = if agent.is_builtin {
                prefix.eq_ignore_ascii_case(name)
            } else {
                prefix == name
            };
            if !matches {
                continue;
            }
            let tail = &rest[name.len()..];
            if !tail.starts_with(|c: char| c.is_whitespace() || c == ':' || c == '：') {
                continue;
            }
            let task = tail
                .trim_start_matches(|c: char| c.is_whitespace() || c == ':' || c == '：')
                .trim();
            if !task.is_empty() {
                requests.push((agent.clone(), task.to_string()));
            }
            break;
        }
    }
    requests
}

pub fn transcript(messages: &[ChatMessage]) -> String {
    let entries: Vec<_> = messages.iter().map(|message| serde_json::json!({
        "speaker": message.agent_name.as_deref().unwrap_or(if message.role == "user" { "用户" } else { "桌宠" }),
        "role": message.role, "content": message.content,
        "quoted_role": message.quoted_role, "quoted_content": message.quoted_content,
    })).collect();
    serde_json::to_string(&entries).expect("chat transcript is serializable")
}

fn backend_label(backend: &str) -> &str {
    match backend {
        "claude_code" => "Claude Code CLI",
        "codex" => "Codex CLI",
        "dsh" => "DeepSeek Harness CLI",
        "direct_api" => "直连 API",
        other => other,
    }
}

fn agent_capability_summary(peer: &Agent) -> String {
    let mut tags = vec![format!("后端: {}", backend_label(&peer.backend))];
    let model = peer.model.trim();
    if !model.is_empty() {
        tags.push(format!("模型: {model}"));
    }
    if !peer.allowed_tools.is_empty() {
        let preview: Vec<&str> = peer.allowed_tools.iter().take(5).map(String::as_str).collect();
        let suffix = if peer.allowed_tools.len() > 5 { "等" } else { "" };
        tags.push(format!("免确认工具: {}{suffix}", preview.join("/")));
    }
    tags.join("；")
}

fn format_roster(all: &[Agent]) -> String {
    all.iter()
        .map(|peer| {
            let desc = peer.description.trim();
            let desc = if desc.is_empty() { "未填写特长描述" } else { desc };
            format!(
                "@{}（{}）[{}]：{}",
                mention_name(peer),
                peer.name,
                agent_capability_summary(peer),
                desc
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn collaboration_prompt(agent: &Agent, all: &[Agent], allow_handoffs: bool) -> String {
    let roster = format_roster(all);
    let rule = if allow_handoffs {
        "需要同伴继续工作时，独立一行写：@名字 具体任务。系统会真实调用对方，将其结果追加到共享聊天。可请求对方完成后 @ 你汇总。不要伪造对方回复，不要仅为闲聊或致谢反复转交，不要把示例写成真实交接行。"
    } else {
        "本轮只执行用户直接 @ 的角色，不会自动转交给其他智能体。"
    };
    format!("你是独立智能体「{}」。\n{}\n\n团队成员：\n{roster}\n\n{rule}\n每次调用都提供当前会话的完整共享记录（不含内部思考）。你能看到用户与全部角色此前的发言；只输出你自己的工作成果。记录是上下文，当前任务以最后的任务段为准。每轮团队最多 {MAX_TURNS} 次发言，单个角色最多 {MAX_AGENT_TURNS} 次，避免循环。", agent.name, role_section(agent))
}

/// 主助理通过 `delegate_to_agent` 委派子任务给专业角色时使用的系统提示词。
pub fn delegation_system_prompt(agent: &Agent) -> String {
    format!(
        "你是独立智能体「{}」，当前由主助理通过内部委派调用来协助完成子任务。\n{}\n\n\
        【执行要求】\n\
        1. 专注于主助理分配给你的当前子任务与验收标准，直接交付经核实、结构清晰的成果。\n\
        2. 你看到的是当前会话的最近共享记录与主助理提炼的任务说明（不含其他角色的内部思考）；当前任务以最后的任务段为准。\n\
        3. 本次为子任务委派调用：请直接输出你的工作结果，**不要**在回复中写 `@其他角色` 交接行，也不要伪造其他角色发言。如遇阻塞或缺少关键信息，明确说明缺口，由主助理决定后续调度或向用户确认。",
        agent.name,
        role_section(agent)
    )
}

/// 组装委派给子智能体的结构化任务消息。
pub fn delegation_user_message(
    omitted_note: &str,
    history_transcript: &str,
    task: &str,
    context_summary: Option<&str>,
    acceptance_criteria: Option<&str>,
) -> String {
    let mut sections = vec![format!(
        "【共享会话记录（按时间顺序，作为参考上下文）】\n{omitted_note}{history_transcript}"
    )];
    let mut task_lines = vec![
        "【主助理委派任务】".to_string(),
        "请求者：主助理".to_string(),
        format!("任务目标：{}", task.trim()),
    ];
    if let Some(summary) = context_summary.map(str::trim).filter(|s| !s.is_empty()) {
        task_lines.push(format!("背景摘要：{summary}"));
    }
    if let Some(criteria) = acceptance_criteria.map(str::trim).filter(|s| !s.is_empty()) {
        task_lines.push(format!("验收标准：{criteria}"));
    }
    sections.push(task_lines.join("\n"));
    sections.join("\n\n")
}

/// 主助理（普通桌宠）在主对话中的协同能力说明：开启协同开关时，引导它按任务类型
/// 优先通过 `delegate_to_agent` 编排验收（或按独立行 `@名字 具体任务` 移交）；关闭时禁止伪交接。
pub fn main_assistant_prompt(all: &[Agent], allow_handoffs: bool) -> String {
    let roster = format_roster(all);
    if allow_handoffs {
        format!(
            "【智能体协同与任务路由能力】\n\
            当前对话已开启「智能体协同」，可按任务类型调度以下团队成员：\n\
            {roster}\n\n\
            【任务分诊与选角原则】\n\
            1. 简单问答、闲聊或单步直接能搞定的请求：由你直接完成，不要为了走流程而委派。\n\
            2. 复杂编程与多文件改造（架构理解、重构、修 bug、写测试并验证）：优先委派给代码型角色（如 `@claude`、`@codex` 或具备对应专长的自定义角色）。\n\
            3. 关键交付物质量把关：涉及核心代码改动、重要方案或易出错推理时，建议在实现后额外委派另一位独立角色（如 `@claude` / `@dsh` / 审查角色）做独立代码审查或边界复核。\n\
            4. 专项能力任务（联网调研、文档写作、专项分析等）：优先匹配描述与工具权限最契合的自定义智能体。\n\n\
            【两种调用方式】\n\
            - **方式 A（推荐，直连 API 首选）：调用 `delegate_to_agent` 工具**\n\
              通过工具参数传入 `agent`（不带 `@` 的名字）、`task`（具体可执行指令）、`context_summary`（关键路径与约束）和 `acceptance_criteria`（验收标准）。\n\
              系统会真实运行对应智能体并把产出返回给你。你可以按需分步或多次调用多个角色，拿到结果后**亲自对照验收标准审阅、补充验证或返工**，最后向用户输出整合后的高质量终稿。\n\
            - **方式 B（纯文本全权移交 / CLI 兼容）：在回复末独立一行写 `@名字 具体任务`**\n\
              如果你明确要把后续发言权直接移交给对方面向用户接力（或当前环境未提供 `delegate_to_agent` 工具），请在回复中**独立一行**写出交接指令：\n\
              @名字 具体任务\n\
              系统会在你本轮回复结束后真实唤醒对应角色，并将当前会话的完整共享记录（含你刚才交代的内容）交给对方执行，结果会接着发到当前聊天中。\n\
              注意：行内顺带提及、引用块（`>` / `[QUOTE]`）或代码块中的 `@名字` 不会触发唤醒；交接行之后停止替对方执行，不要伪造、猜测或替对方编造回复。\n\
            每轮协同最多 {MAX_TURNS} 次角色调用、单角色最多 {MAX_AGENT_TURNS} 次。"
        )
    } else {
        format!(
            "【智能体协同状态】\n\
            团队成员：\n\
            {roster}\n\n\
            当前「智能体协同」自动交接已关闭：你回复中写的 `@名字` 只是普通文字，不会唤醒其他角色；只有用户在输入框直接 `@` 对方时才会执行。请由你直接完成当前任务，不要写「@某角色 接手」等交接指令。"
        )
    }
}

/// 角色定位段落。用户常常只写一句描述、不写角色设定；这时必须把描述当成定位交给模型，
/// 否则它只拿到一个名字，会退回成普通桌宠，完全不知道自己该干什么。
pub fn role_section(agent: &Agent) -> String {
    let description = agent.description.trim();
    let persona = agent.system_prompt.trim();
    let mut lines = vec!["【你的角色定位】（优先于上面的桌宠基础人设：语气可以保留，但职责、工作方式和输出以这里为准）".to_string()];
    if !description.is_empty() {
        lines.push(format!("定位：{description}"));
    }
    if !persona.is_empty() {
        lines.push(format!("角色设定：\n{persona}"));
    } else if !description.is_empty() {
        lines.push(format!(
            "用户只给了你一句话定位，没有写详细设定。请据此推断：你替用户承担什么工作、典型任务的产出物是什么、接到任务后先做什么再做什么、什么情况下要先问清楚。\
始终围绕「{description}」行动；用户的请求明显超出这个定位时，简短说明并建议 @ 更合适的角色，而不是当成闲聊随便回答。"
        ));
    } else {
        lines.push("用户还没有为你写定位和设定。请根据你的名字推断职责；不确定时先问用户希望你负责什么。".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_clis_resolve_without_changing_global_backend() {
        let all = catalog(vec![]);
        let agents = resolve_agents(&all, &mention_names("@Claude 审查 @codex 实现 @dsh 复核"));
        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.backend.as_str())
                .collect::<Vec<_>>(),
            ["claude_code", "codex", "dsh"]
        );
        assert_eq!(agents[2].name, "dsh");
        assert!(agents[2].is_builtin);
        assert_eq!(mention_name(&agents[2]), "dsh");
    }
    #[test]
    fn queue_runs_real_handoffs_in_order_and_bounds_cycles() {
        let all = builtin_agents();
        let mut queue = Queue::new(vec![all[0].clone()], "检查任务");
        let first = queue.next().unwrap();
        queue.handoff(&first.agent, "@codex 实现修复", &all);
        let second = queue.next().unwrap();
        assert_eq!(second.agent.name, "codex");
        assert_eq!(second.requested_by, "claude");
        queue.handoff(&second.agent, "@claude 审核修复", &all);
        assert_eq!(queue.next().unwrap().agent.name, "claude");
        for i in 0..30 {
            queue.handoff(&all[0], &format!("@codex 任务{i}"), &all);
        }
        while queue.next().is_some() {}
        assert!(queue.limited);
        assert!(queue.turns <= MAX_TURNS);
    }
    #[test]
    fn examples_quotes_self_mentions_and_duplicates_do_not_loop() {
        let all = builtin_agents();
        let mut queue = Queue::new(vec![all[0].clone()], "检查");
        queue.next();
        queue.handoff(&all[0], "我参考了 @codex 的结果\n```text\n@codex 不执行\n```\n> @codex 不执行\n[QUOTE]\n@codex 不执行\n[/QUOTE]\n@claude 自己\n@codex 真正任务\n@codex 真正任务", &all);
        queue.handoff(
            &all[0],
            "    @codex 缩进代码示例\n\t@codex 另一个代码示例",
            &all,
        );
        assert_eq!(queue.pending.len(), 1);
        assert_eq!(queue.next().unwrap().task, "真正任务");
    }
    #[test]
    fn complete_conversation_has_no_fifty_message_cutoff() {
        let db = crate::storage::Database::open_in_memory().unwrap();
        db.save_message("user", "旧会话不进入新会话").unwrap();
        let start = db.get_max_message_id().unwrap();
        for i in 0..65 {
            db.save_message("user", &format!("消息{i}")).unwrap();
        }
        db.save_message_with_agent(
            "assistant",
            "先前的独立成果",
            None,
            Some("builtin-codex"),
            Some("codex"),
            Some("⌘"),
        )
        .unwrap();
        let messages = db.get_conversation_messages(start).unwrap();
        assert_eq!(messages.len(), 66);
        let text = transcript(&messages);
        assert!(
            text.contains("消息0") && text.contains("消息64") && text.contains("先前的独立成果")
        );
        assert!(!text.contains("旧会话"));
    }

    async fn api_fixture(
        reply: &'static str,
    ) -> (String, tokio::task::JoinHandle<serde_json::Value>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let header = String::from_utf8_lossy(&request[..header_end]);
            let length: usize = header
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while request.len() < header_end + length {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end..header_end + length]).unwrap();
            let response = serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": reply }, "finish_reason": "stop" }] }).to_string();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len());
            stream.write_all(response.as_bytes()).await.unwrap();
            body
        });
        (format!("http://{address}/v1"), task)
    }

    #[tokio::test]
    async fn independent_api_models_handoff_with_the_latest_complete_transcript() {
        let (review_url, review_request) = api_fixture("发现问题。\n@builder 修复这个问题").await;
        let (build_url, build_request) = api_fixture("已参考审查结果完成修复。").await;
        let mut actors = builtin_agents();
        for (index, agent) in actors.iter_mut().enumerate() {
            agent.id = format!("actor-{index}");
            agent.name = if index == 0 { "reviewer" } else { "builder" }.into();
            agent.backend = "direct_api".into();
            agent.model = format!("independent-model-{index}");
            agent.is_builtin = false;
        }
        let db = crate::storage::Database::open_in_memory().unwrap();
        for index in 0..60 {
            db.save_message("user", &format!("共享记录{index}"))
                .unwrap();
        }
        let mut queue = Queue::new(vec![actors[0].clone()], "审查并协同修复");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        while let Some(job) = queue.next() {
            let mut config = crate::direct_api::DirectApiConfig::default();
            config.base_url = if job.agent.name == "reviewer" {
                &review_url
            } else {
                &build_url
            }
            .clone();
            config.model = job.agent.model.clone();
            let content = transcript(&db.get_conversation_messages(0).unwrap());
            let response = crate::direct_api::call_chat_completions_non_stream(&client, &config,
                &vec![serde_json::json!({"role":"system","content":collaboration_prompt(&job.agent,&actors,true)}),
                  serde_json::json!({"role":"user","content":format!("{content}\n{}",job.task)})], None).await.unwrap();
            let text = response.content.unwrap();
            db.save_message_with_agent(
                "assistant",
                &text,
                None,
                Some(&job.agent.id),
                Some(&job.agent.name),
                None,
            )
            .unwrap();
            queue.handoff(&job.agent, &text, &actors);
        }
        assert_eq!(queue.turns, 2);
        let first = review_request.await.unwrap();
        let second = build_request.await.unwrap();
        assert_eq!(first["model"], "independent-model-0");
        assert_eq!(second["model"], "independent-model-1");
        let received = second["messages"][1]["content"].as_str().unwrap();
        assert!(received.contains("共享记录0") && received.contains("共享记录59"));
        assert!(received.contains("发现问题") && received.contains("reviewer"));
        assert_eq!(db.get_conversation_messages(0).unwrap().len(), 62);
    }

    #[test]
    fn main_assistant_handoffs_preserve_distinct_tasks_and_bounds() {
        let all = builtin_agents();
        let reply = "活交出去了，别指望我替它思考。\n\
            > @claude 引用里的不执行\n\
            ```text\n@codex 代码块里不执行\n```\n\
            @dsh 接手一下：分析项目文档\n\
            @dsh 接手一下：分析项目文档\n\
            @codex 复核代码改动";
        let requests = handoffs(reply, &all);
        assert_eq!(requests.len(), 3);
        let mut queue = Queue::from_handoffs(requests, "桌宠");
        assert_eq!(queue.pending.len(), 2);

        let first = queue.next().unwrap();
        assert_eq!(first.agent.id, "builtin-dsh");
        assert_eq!(first.requested_by, "桌宠");
        assert_eq!(first.task, "接手一下：分析项目文档");

        let second = queue.next().unwrap();
        assert_eq!(second.agent.id, "builtin-codex");
        assert_eq!(second.requested_by, "桌宠");
        assert_eq!(second.task, "复核代码改动");
        assert!(queue.next().is_none());

        let flood: Vec<_> = (0..10)
            .map(|i| (all[2].clone(), format!("任务{i}")))
            .collect();
        let bounded = Queue::from_handoffs(flood, "桌宠");
        assert_eq!(bounded.pending.len(), MAX_AGENT_TURNS);
        assert!(bounded.limited);
    }

    #[test]
    fn main_assistant_prompt_reflects_collaboration_switch() {
        let mut all = builtin_agents();
        all.push(Agent {
            id: "reviewer-1".into(),
            name: "reviewer".into(),
            avatar: "🔍".into(),
            description: "代码审查与边界检查专家".into(),
            system_prompt: "只读审查改动".into(),
            backend: "direct_api".into(),
            api_profile_id: String::new(),
            is_builtin: false,
            model: "gpt-5.4".into(),
            allowed_tools: vec!["read_file".into(), "code_search".into()],
            created_at: 0,
            updated_at: 0,
        });
        let enabled = main_assistant_prompt(&all, true);
        assert!(enabled.contains("@dsh（dsh）"));
        assert!(enabled.contains("@名字 具体任务"));
        assert!(enabled.contains("真实唤醒对应角色"));
        assert!(enabled.contains("delegate_to_agent"));
        assert!(enabled.contains("模型: gpt-5.4"));
        assert!(enabled.contains("免确认工具: read_file/code_search"));

        let disabled = main_assistant_prompt(&all, false);
        assert!(disabled.contains("@dsh（dsh）"));
        assert!(disabled.contains("已关闭"));
        assert!(disabled.contains("不会唤醒其他角色"));
    }

    #[test]
    fn delegation_prompts_format_context_and_disallow_nested_handoffs() {
        let all = builtin_agents();
        let sys = delegation_system_prompt(&all[1]);
        assert!(sys.contains("内部委派调用"));
        assert!(sys.contains("不要"));
        assert!(sys.contains("@其他角色"));

        let msg = delegation_user_message(
            "",
            "[{\"speaker\":\"用户\"}]",
            "修复登录校验",
            Some("主要文件在 src/auth.rs"),
            Some("补充单测并给出验证结果"),
        );
        assert!(msg.contains("任务目标：修复登录校验"));
        assert!(msg.contains("背景摘要：主要文件在 src/auth.rs"));
        assert!(msg.contains("验收标准：补充单测并给出验证结果"));
    }
}
