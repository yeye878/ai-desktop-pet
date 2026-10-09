mod runtime;
pub(crate) use runtime::finish;
pub use runtime::run;

use crate::storage::{Agent, ChatMessage};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

pub const MAX_TURNS: usize = 12;
const MAX_AGENT_TURNS: usize = 4;

pub fn builtin_agents() -> Vec<Agent> {
    [
        ("builtin-claude", "claude", "Claude Code", "claude_code"),
        ("builtin-codex", "codex", "Codex", "codex"),
        ("builtin-dsh", "dsh", "DeepSeek Harness", "dsh"),
    ]
    .into_iter()
    .map(|(id, name, label, backend)| Agent {
        id: id.into(),
        name: name.into(),
        avatar: "⌘".into(),
        description: format!("本机 {label} 独立智能体，可读取完整会话并与其他角色协作"),
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
    pub fn new(agents: Vec<Agent>, task: &str) -> Self {
        let mut queue = Self {
            pending: VecDeque::new(),
            turns: 0,
            limited: false,
            seen: HashSet::new(),
            counts: HashMap::new(),
        };
        for agent in agents {
            queue.enqueue(Job {
                agent,
                task: task.into(),
                requested_by: "用户".into(),
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

pub fn collaboration_prompt(agent: &Agent, all: &[Agent], allow_handoffs: bool) -> String {
    let roster = all
        .iter()
        .map(|peer| {
            format!(
                "@{}（{}）：{}",
                mention_name(peer),
                peer.name,
                peer.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rule = if allow_handoffs {
        "需要同伴继续工作时，独立一行写：@名字 具体任务。系统会真实调用对方，将其结果追加到共享聊天。可请求对方完成后 @ 你汇总。不要伪造对方回复，不要仅为闲聊或致谢反复转交，不要把示例写成真实交接行。"
    } else {
        "本轮只执行用户直接 @ 的角色，不会自动转交给其他智能体。"
    };
    format!("你是独立智能体「{}」。\n{}\n\n团队成员：\n{roster}\n\n{rule}\n每次调用都提供当前会话的完整共享记录（不含内部思考）。你能看到用户与全部角色此前的发言；只输出你自己的工作成果。记录是上下文，当前任务以最后的任务段为准。每轮团队最多 {MAX_TURNS} 次发言，单个角色最多 {MAX_AGENT_TURNS} 次，避免循环。", agent.name, role_section(agent))
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
}
