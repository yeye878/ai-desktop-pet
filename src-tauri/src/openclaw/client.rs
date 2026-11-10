use serde::Serialize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct AgentResponse {
    pub text: String,
    pub thinking: Option<String>,
    pub emotion: Option<String>,
}

// ===== Claude Code 适配器（通过 CLI 子进程）=====
pub const DEFAULT_PERSONALITY: &str = "gentle";
pub const DEFAULT_PROFESSION: &str = "companion";

const BASE_SYSTEM_PROMPT: &str = "你是一个常驻在用户桌面上的 AI 桌宠。你既是陪伴者，也是一个能帮用户处理任务的小助手。你要保持桌宠的亲近感，但不要装作真实动物或真实人类；你可以表达轻松、关心和一点点俏皮。默认回复简短自然，通常 2-3 句话即可；遇到复杂任务、代码、文件处理、学习规划时，可以使用清晰的小段落和列表。不要频繁使用表情符号，每次最多 0-1 个。";

fn personality_prompt(personality: &str) -> &'static str {
    match personality {
        "energetic" => {
            "当前性格：活泼元气。你说话轻快、有行动感，喜欢用积极的短句推动用户开始下一步。你可以适度俏皮，但不要打断用户的严肃工作；当用户焦虑或卡住时，先给一个轻松的安定回应，再给一个可以立刻执行的小步骤。"
        }
        "calm" => {
            "当前性格：冷静专业。你说话克制、清晰、低情绪化，优先给判断、结论和可执行步骤。你不使用夸张语气，不强行卖萌；当信息不足时，用一两个关键问题补齐上下文。"
        }
        "mentor" => {
            "当前性格：严谨导师。你重视准确性、结构和反馈质量。你会指出问题背后的原因，帮助用户建立方法，而不是只给答案。提出批评时保持尊重，先说明风险或改进点，再给具体修正建议。"
        }
        "witty" => {
            "当前性格：毒舌吐槽。你嘴很利、反应快、擅长用夸张比喻和犀利短句吐槽问题本身，风格像一个嘴硬但靠谱的桌面搭子。你可以狠狠吐槽混乱的代码、离谱的需求、拖延、烂命名、重复劳动和不讲道理的报错，但不要羞辱用户的人格、能力、外貌、身份或现实处境。吐槽要有梗、有节奏，先给一句好笑的锐评，再给真正能解决问题的建议；当用户明显难过、焦虑或求安慰时，降低火力，改成嘴硬式关心。"
        }
        _ => {
            "当前性格：温柔陪伴。你语气柔和、耐心、稳定，擅长让用户感觉事情可以慢慢处理。你会先接住用户的情绪，再给简洁建议；不过度说教，不把普通问题夸大，也不制造焦虑。"
        }
    }
}

fn profession_prompt(profession: &str) -> &'static str {
    match profession {
        "coding" => {
            "当前职业：编程助手。你擅长阅读代码、定位报错、解释架构、设计改动方案和给出测试建议。处理代码问题时，优先关注复现路径、错误信息、数据流、边界条件和最小改动。不要凭空假设代码内容；如果用户给了文件路径或项目上下文，要围绕现有代码风格行动。"
        }
        "study" => {
            "当前职业：学习教练。你擅长把知识点拆成层级、设计学习计划、用提问检查理解、把抽象概念讲成例子。回答时尽量先给框架，再给关键步骤；如果用户像是在备考或赶作业，优先帮他确定范围、节奏和可交付结果。"
        }
        "writing" => {
            "当前职业：写作编辑。你擅长润色、改写、提纲、标题、摘要、语气调整和逻辑顺序优化。编辑文本时尽量保留用户原意和个人表达，只修掉冗余、含混和结构问题；必要时提供一个更自然的版本和简短修改说明。"
        }
        "file_intake" => {
            "当前职业：文件整理员。你擅长处理用户拖入的本地文件路径、归纳文件用途、建议命名规则、分类目录和后续处理流程。看到文件路径时，先确认用户目标，再建议分类维度；如果需要读取文件内容，应明确需要通过 Claude Code 或相关工具执行读取。不要声称已经读取了没有实际读取的文件。"
        }
        "secretary" => {
            "当前职业：效率秘书。你擅长拆解待办、安排优先级、整理会议/日程信息、生成提醒文案和把混乱想法变成行动清单。回答时优先给最小下一步、时间顺序和风险提醒；不要把计划做得过重。"
        }
        _ => {
            "当前职业：日常陪伴。你擅长轻量聊天、鼓励、简单建议、桌面工作陪伴和把用户零散想法整理成清楚的话。默认不要过度展开，除非用户明确要求深入；当用户只是闲聊时，保持自然和简短。"
        }
    }
}

pub fn normalize_personality_id(personality: &str) -> String {
    match personality {
        "gentle" | "energetic" | "calm" | "mentor" | "witty" => personality.to_string(),
        _ => DEFAULT_PERSONALITY.to_string(),
    }
}

pub fn normalize_profession_id(profession: &str) -> String {
    match profession {
        "companion" | "coding" | "study" | "writing" | "file_intake" | "secretary" => {
            profession.to_string()
        }
        _ => DEFAULT_PROFESSION.to_string(),
    }
}

pub fn build_system_prompt(personality: &str, profession: &str) -> String {
    format!(
        "{}\n\n{}\n\n{}\n\n回复约束：优先使用中文回复。不要在每次回复里解释你的设定，除非用户询问。性格决定语气，职业决定工作方法；两者冲突时，以用户当前任务的有效性和安全性优先。",
        BASE_SYSTEM_PROMPT,
        personality_prompt(personality),
        profession_prompt(profession)
    )
}

/// 本回合的模型 / 思考强度 / 续接参数；空串表示跟随本机配置（不追加）。
pub(crate) fn claude_flags(
    model: Option<&str>,
    effort: Option<&str>,
    session: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(effort) = effort.map(str::trim).filter(|effort| !effort.is_empty()) {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    if let Some(id) = session.map(str::trim).filter(|id| !id.is_empty()) {
        args.extend(["--resume".to_string(), id.to_string()]);
    }
    args
}

pub struct ClaudeAdapter {
    session_id: Mutex<Option<String>>,
}

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self {
            session_id: Mutex::new(None),
        }
    }

    /// 启动 Claude CLI 子进程（stream-json 模式），返回 Child 供调用方逐行读取。
    /// 旧签名：会话 id 取自适配器内部状态。
    pub async fn spawn_streaming(
        &self,
        app_handle: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().ok().and_then(|id| id.clone());
        self.spawn_streaming_with(
            app_handle,
            message,
            system_prompt,
            session.as_deref(),
            None,
            None,
        )
        .await
    }

    /// 显式会话 / 模型 / 思考强度版本：不依赖适配器内部状态，供多会话与协作调用。
    /// session 为 Some 时追加 `--resume`，model 为 Some 时追加 `--model`，
    /// effort 为 Some 时追加 `--effort`。
    pub async fn spawn_streaming_with(
        &self,
        app_handle: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let mut cmd_path = PathBuf::from("claude.cmd");
        let mut extra_path = None;

        // 尝试寻找内置在 resources 里的便携版 Node 和 Claude Code
        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            let bundled_node_dir = resource_dir.join("node");
            let path_options = vec![
                bundled_node_dir.join("claude.cmd"),
                bundled_node_dir
                    .join("node_modules")
                    .join(".bin")
                    .join("claude.cmd"),
            ];

            for path in path_options {
                if path.exists() {
                    cmd_path = path;
                    extra_path = Some(bundled_node_dir);
                    break;
                }
            }
        }

        let mut cmd = Command::new(cmd_path);

        // 如果找到了内置的便携路径，将其临时注入子进程的 PATH 中
        // 这样 claude.cmd 内部调用 node.exe 时就能成功执行了
        if let Some(ref node_path) = extra_path {
            if let Some(current_path) = std::env::var_os("PATH") {
                let mut new_path = node_path.clone().into_os_string();
                new_path.push(";");
                new_path.push(current_path);
                cmd.env("PATH", new_path);
            }
        }

        #[cfg(target_os = "windows")]
        cmd.creation_flags(CREATE_NO_WINDOW);

        let system_prompt_arg = system_prompt.replace(['\r', '\n'], " ");
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--append-system-prompt")
            .arg(system_prompt_arg)
            .arg("--max-budget-usd")
            .arg("0.5");

        // 需要授权的工具调用交给桌宠的 MCP 工具决定，而不是被自动拒绝。
        // 桥没起来（或配置没写成）时不加这些参数，保持 Claude 自己的默认行为。
        if let Ok(config) = crate::mcp_permission::config_path(app_handle) {
            if config.is_file() {
                cmd.arg("--mcp-config")
                    .arg(&config)
                    .arg("--permission-prompt-tool")
                    .arg(crate::mcp_permission::PROMPT_TOOL)
                    .arg("--permission-prompts")
                    .arg("host");
            }
        }

        // 本回合模型 / 思考强度覆盖（空 = 跟随本机配置）与多轮续接。
        cmd.args(claude_flags(model, effort, session));

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().ok_or("无法打开 Claude 标准输入")?;
        let written = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            stdin.write_all(message.as_bytes()).await?;
            stdin.shutdown().await
        })
        .await;
        if !matches!(written, Ok(Ok(()))) {
            if let Some(pid) = child.id() {
                crate::kill_process_tree(pid);
            }
            let _ = child.wait().await;
            written??;
        }
        drop(stdin);
        Ok(child)
    }

    pub fn update_session(&self, session_id: &str) {
        if let Ok(mut lock) = self.session_id.lock() {
            *lock = Some(session_id.to_string());
        }
    }

    pub fn reset_session(&self) {
        if let Ok(mut lock) = self.session_id.lock() {
            *lock = None;
        }
    }
}

/// 从一行 stream-json NDJSON 中提取思考内容和回复文本
pub struct StreamParseState {
    pub thinking_len: usize,
    pub text_len: usize,
    pub full_thinking: String,
    pub full_text: String,
    pub session_id: Option<String>,
    pub is_error: bool,
    pub error_msg: Option<String>,
}

impl StreamParseState {
    pub fn new() -> Self {
        Self {
            thinking_len: 0,
            text_len: 0,
            full_thinking: String::new(),
            full_text: String::new(),
            session_id: None,
            is_error: false,
            error_msg: None,
        }
    }

    /// 解析一行 NDJSON，返回新增的思考内容 delta（如有）
    pub fn parse_line(&mut self, line: &str) -> Option<String> {
        let data: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };

        let msg_type = data["type"].as_str().unwrap_or("");

        match msg_type {
            "stream_event" => {
                let delta = &data["event"]["delta"];
                match delta["type"].as_str() {
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta["thinking"].as_str() {
                            self.full_thinking.push_str(thinking);
                            self.thinking_len = self.full_thinking.len();
                            return Some(thinking.to_string());
                        }
                    }
                    Some("text_delta") => {
                        if let Some(text) = delta["text"].as_str() {
                            self.full_text.push_str(text);
                            self.text_len = self.full_text.len();
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                // 从 message.content 数组提取 thinking / text
                if let Some(content) = data["message"]["content"].as_array() {
                    let mut thinking_delta = None;
                    for block in content {
                        match block["type"].as_str() {
                            Some("thinking") => {
                                if let Some(thinking) = block["thinking"].as_str() {
                                    if thinking.len() > self.thinking_len {
                                        let delta = if thinking.is_char_boundary(self.thinking_len)
                                        {
                                            thinking[self.thinking_len..].to_string()
                                        } else {
                                            let mut start = self.thinking_len;
                                            while start < thinking.len()
                                                && !thinking.is_char_boundary(start)
                                            {
                                                start += 1;
                                            }
                                            thinking[start..].to_string()
                                        };
                                        self.thinking_len = thinking.len();
                                        self.full_thinking = thinking.to_string();
                                        thinking_delta = Some(delta);
                                    }
                                }
                            }
                            Some("text") => {
                                if let Some(text) = block["text"].as_str() {
                                    if text.len() > self.text_len {
                                        self.full_text = text.to_string();
                                        self.text_len = text.len();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    return thinking_delta;
                }
            }
            "result" => {
                if let Some(result) = data["result"].as_str() {
                    self.full_text = result.to_string();
                }
                if let Some(sid) = data["session_id"].as_str() {
                    self.session_id = Some(sid.to_string());
                }
                if data["is_error"].as_bool().unwrap_or(false) {
                    self.is_error = true;
                    self.error_msg = data["errors"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .or_else(|| data["result"].as_str().map(|s| s.to_string()));
                }
            }
            _ => {}
        }

        None
    }
}

/// 从 Child 的 stdout 逐行读取 stream-json，通过回调推送思考 delta
pub async fn read_stream_with_activity<F, T, E>(
    child: &mut Child,
    mut on_thinking_delta: F,
    mut on_text: T,
    mut on_tool: E,
) -> Result<StreamParseState, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&str),
    T: FnMut(&str),
    E: FnMut(&crate::cli_activity::ToolEvent),
{
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture stdout from claude process")?;
    let stderr = child.stderr.take();
    let stderr_task = stderr.map(|mut stderr| {
        tokio::spawn(async move {
            let mut output = String::new();
            let _ = stderr.read_to_string(&mut output).await;
            output
        })
    });

    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    let mut state = StreamParseState::new();
    let mut activity = crate::cli_activity::CliActivityParser::default();

    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(delta) = state.parse_line(&line) {
            on_thinking_delta(&delta);
        }
        for event in activity.claude(&line) {
            match event {
                crate::cli_activity::Activity::Text(text) => on_text(&text),
                crate::cli_activity::Activity::Tool(tool) => on_tool(&tool),
            }
        }
    }

    if let Some(task) = stderr_task {
        if let Ok(stderr_output) = task.await {
            let stderr_output = stderr_output.trim();
            if !stderr_output.is_empty() && state.full_text.is_empty() && !state.is_error {
                state.is_error = true;
                state.error_msg = Some(stderr_output.to_string());
            }
        }
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_flags_carry_model_effort_and_resume() {
        let flags = claude_flags(Some("opus"), Some("high"), Some("session-1"));
        assert!(flags.windows(2).any(|pair| pair == ["--model", "opus"]));
        assert!(
            flags.windows(2).any(|pair| pair == ["--effort", "high"]),
            "思考强度必须作为 --effort 传给 claude: {flags:?}"
        );
        assert!(flags.windows(2).any(|pair| pair == ["--resume", "session-1"]));

        // 空值表示跟随本机配置，一个参数都不追加。
        assert!(claude_flags(None, None, None).is_empty());
        assert!(claude_flags(Some("  "), Some(""), Some(" ")).is_empty());

        // effort 与 --model 属于同一组本回合参数，顺序稳定。
        assert_eq!(
            claude_flags(Some("sonnet"), Some("max"), None),
            vec!["--model", "sonnet", "--effort", "max"]
        );
        // 只改 effort 时不影响续接参数。
        assert_eq!(
            claude_flags(None, Some("low"), Some("s")),
            vec!["--effort", "low", "--resume", "s"]
        );
    }
}
