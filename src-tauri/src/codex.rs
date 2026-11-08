//! Local Codex CLI integration. Authentication and model selection stay in Codex.
use crate::openclaw::StreamParseState;
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::Mutex,
    time::Duration,
};
use tauri::Manager;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn find_cli() -> Result<PathBuf, String> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    // GUI apps can inherit PATH from before npm installed the CLI.
    if let Some(app_data) = dirs::data_dir() {
        directories.push(app_data.join("npm"));
    }
    if let Some(home) = dirs::home_dir() {
        directories.push(home.join(".local").join("bin"));
        directories.push(home.join(".cargo").join("bin"));
    }
    #[cfg(target_os = "windows")]
    let names = ["codex.exe", "codex.cmd", "codex.bat"];
    #[cfg(not(target_os = "windows"))]
    let names = ["codex"];

    for directory in directories.iter().filter(|path| path.is_absolute()) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("未找到本机 Codex CLI。请先运行 npm install -g @openai/codex，再刷新状态。".into())
}

fn cli_command(path: &std::path::Path) -> Command {
    let mut command = Command::new(path);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    command.kill_on_drop(true);
    command
}

pub fn workspace_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("codex-workspace"))
        .map_err(|error| error.to_string())
}

#[derive(Default)]
pub struct CodexAdapter {
    session_id: Mutex<Option<String>>,
}

fn exec_args(session_id: Option<&str>, model: Option<&str>) -> Vec<String> {
    // exec cannot ask for approval in the desktop chat. Keep a bounded writable
    // workspace and fail commands that would need additional permission.
    let mut args: Vec<String> = [
        "exec",
        "--sandbox",
        "workspace-write",
        "-c",
        "approval_policy=\"never\"",
        // 默认配置下 reasoning 事件常常为空，detailed 摘要才能展示"思考过程"。
        "-c",
        "model_reasoning_summary=\"detailed\"",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();
    // 本回合模型覆盖必须放在子命令（resume）之前。
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        args.extend(["-m".into(), model.to_string()]);
    }
    if let Some(id) = session_id {
        args.extend(["resume".into(), id.into()]);
    }
    args.extend(["--json".into(), "--skip-git-repo-check".into(), "-".into()]);
    args
}

impl CodexAdapter {
    /// 旧签名：会话 id 取自适配器内部状态。
    pub async fn spawn_streaming(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().unwrap().clone();
        self.spawn_streaming_with(app, message, system_prompt, session.as_deref(), None)
            .await
    }

    /// 显式会话 / 模型版本：不依赖适配器内部状态，供多会话与协作调用。
    pub async fn spawn_streaming_with(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let workspace = workspace_dir(app)?;
        self.spawn_in_workspace_with(&workspace, message, system_prompt, session, model)
            .await
    }

    #[cfg(test)]
    async fn spawn_in_workspace(
        &self,
        workspace: &std::path::Path,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let session = self.session_id.lock().unwrap().clone();
        self.spawn_in_workspace_with(workspace, message, system_prompt, session.as_deref(), None)
            .await
    }

    async fn spawn_in_workspace_with(
        &self,
        workspace: &std::path::Path,
        message: &str,
        system_prompt: &str,
        session: Option<&str>,
        model: Option<&str>,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        std::fs::create_dir_all(&workspace)?;
        let mut child = cli_command(&find_cli()?)
            .args(exec_args(session, model))
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // stdin preserves Chinese, multiline text and shell metacharacters,
        // including when the executable is npm's Windows .cmd wrapper.
        let prompt = format!("【桌宠角色与规则】\n{system_prompt}\n\n【用户消息】\n{message}");
        let mut stdin = child.stdin.take().ok_or("无法打开 Codex 标准输入")?;
        tokio::time::timeout(Duration::from_secs(10), async {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await
        })
        .await??;
        drop(stdin);
        Ok(child)
    }

    pub fn update_session(&self, session_id: &str) {
        // Only resume the thread returned by this adapter, never --last (which
        // might pick an unrelated session from the user's terminal).
        if uuid::Uuid::parse_str(session_id).is_ok() {
            *self.session_id.lock().unwrap() = Some(session_id.to_string());
        }
    }

    pub fn reset_session(&self) {
        *self.session_id.lock().unwrap() = None;
    }
}

/// 一行 Codex 事件产生的有序增量：思考与正文分开，调用方按顺序消费。
#[derive(Debug, PartialEq, Eq)]
enum Delta {
    Thinking(String),
    Text(String),
}

/// 取文本字段：字符串直接使用，数组按 text 子字段拼接。
fn text_field(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let joined = value
        .as_array()?
        .iter()
        .filter_map(|entry| {
            entry
                .as_str()
                .map(str::to_string)
                .or_else(|| entry["text"].as_str().map(str::to_string))
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// reasoning 文本：优先 text，为空时回退 summary_text / raw_content。
fn reasoning_text(item: &serde_json::Value) -> Option<String> {
    ["text", "summary_text", "raw_content"]
        .iter()
        .find_map(|key| text_field(&item[*key]))
}

/// 返回 text 相对 previous 的新增部分（按最长公共前缀计算），避免重复上报。
fn increment(previous: &str, text: &str) -> String {
    let common = previous
        .char_indices()
        .zip(text.char_indices())
        .find(|((_, left), (_, right))| left != right)
        .map_or_else(
            || previous.len().min(text.len()),
            |((index, _), _)| index,
        );
    text[common..].to_string()
}

#[derive(Default)]
struct EventParser {
    completed_items: HashSet<String>,
    turn_completed: bool,
    last_error: Option<String>,
    /// reasoning 条目 id → 已输出文本，item.started/updated 只补增量。
    reasoning_items: HashMap<String, String>,
    /// 最近一次输出思考的条目 id，用于判断是否需要插入分隔。
    last_reasoning_id: Option<String>,
}

impl EventParser {
    /// 解析一行 Codex --json 事件，返回本行按真实顺序产生的思考/正文增量。
    fn parse_line(&mut self, state: &mut StreamParseState, line: &str) -> Vec<Delta> {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return Vec::new();
        };
        let mut deltas = Vec::new();
        match event["type"].as_str().unwrap_or_default() {
            "thread.started" => {
                state.session_id = event["thread_id"].as_str().map(str::to_string);
            }
            "item.started" | "item.updated" | "item.completed" => {
                self.item(&event, state, &mut deltas);
            }
            "turn.completed" => self.turn_completed = true,
            "turn.failed" => {
                state.is_error = true;
                state.error_msg = Some(
                    event["error"]["message"]
                        .as_str()
                        .unwrap_or("Codex 本轮执行失败")
                        .to_string(),
                );
            }
            "error" => {
                // Transient connection errors can be followed by a successful
                // retry. Only surface these if the turn never completes.
                self.last_error = event["message"].as_str().map(str::to_string);
            }
            _ => {}
        }
        deltas
    }

    /// 处理单个 item：reasoning 进思考通道，agent_message 进正文通道。
    /// 返回值内的增量顺序即事件顺序，所以 reasoning 一定排在它之后的
    /// agent_message 之前。
    fn item(
        &mut self,
        event: &serde_json::Value,
        state: &mut StreamParseState,
        deltas: &mut Vec<Delta>,
    ) {
        let event_type = event["type"].as_str().unwrap_or_default();
        let item = &event["item"];
        let id = item["id"].as_str().unwrap_or_default();
        match item["type"].as_str().unwrap_or_default() {
            "reasoning" => {
                if event_type == "item.completed" {
                    // 定稿事件只发尚未流出的尾部，避免与 started/updated 重复。
                    if !self.completed_items.insert(id.to_string()) {
                        return;
                    }
                    let text = reasoning_text(item).unwrap_or_default();
                    let previous = self.reasoning_items.remove(id).unwrap_or_default();
                    let delta = increment(&previous, &text);
                    self.emit_thinking(state, deltas, id, &delta);
                } else {
                    // started/updated 可能重复推送同一份文本，按 id 记住上次内容。
                    let Some(text) = reasoning_text(item) else {
                        return;
                    };
                    let previous = self.reasoning_items.get(id).cloned().unwrap_or_default();
                    // 比已缓存文本更短的事件直接忽略：否则缓存被改短后，定稿事件
                    // 会按最长公共前缀把中段再补发一次，思考过程出现重复段落。
                    if text.chars().count() <= previous.chars().count() {
                        return;
                    }
                    let delta = increment(&previous, &text);
                    self.reasoning_items.insert(id.to_string(), text);
                    self.emit_thinking(state, deltas, id, &delta);
                }
            }
            "agent_message" => {
                // 只在定稿事件处理。若某个 CLI 版本改用 started/updated 增量推送，
                // 定稿事件仍带完整文本；提前把 id 记入 completed_items 会让定稿被
                // 丢弃，最终回复直接截断。
                if event_type != "item.completed" {
                    return;
                }
                if !self.completed_items.insert(id.to_string()) {
                    return;
                }
                let text = item["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                if !state.full_text.is_empty() {
                    state.full_text.push_str("\n\n");
                }
                state.full_text.push_str(text);
                state.text_len = state.full_text.len();
                deltas.push(Delta::Text(text.to_string()));
            }
            _ => {}
        }
    }

    /// 追加思考增量：只有换了条目才插入分隔，同一 item 的增量保持连续。
    fn emit_thinking(
        &mut self,
        state: &mut StreamParseState,
        deltas: &mut Vec<Delta>,
        id: &str,
        delta: &str,
    ) {
        if delta.is_empty() {
            return;
        }
        let separate = id.is_empty() || self.last_reasoning_id.as_deref() != Some(id);
        if separate && !state.full_thinking.is_empty() {
            state.full_thinking.push_str("\n\n");
        }
        state.full_thinking.push_str(delta);
        state.thinking_len = state.full_thinking.len();
        self.last_reasoning_id = Some(id.to_string());
        deltas.push(Delta::Thinking(delta.to_string()));
    }

    fn finish(&self, state: &mut StreamParseState, success: bool, stderr: &str) {
        if state.is_error || (success && self.turn_completed) {
            return;
        }
        state.is_error = true;
        state.error_msg = Some(self.last_error.clone().unwrap_or_else(|| {
            if stderr.trim().is_empty() {
                "Codex 在本轮完成前退出，请检查本机登录和配置后重试。".to_string()
            } else {
                stderr.trim().to_string()
            }
        }));
    }
}

#[cfg(test)]
pub async fn read_stream<F, T>(
    child: &mut Child,
    on_thinking: F,
    on_text: T,
) -> Result<StreamParseState, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&str),
    T: FnMut(&str),
{
    read_stream_with_activity(child, on_thinking, on_text, |_| {}).await
}

pub async fn read_stream_with_activity<F, T, E>(
    child: &mut Child,
    mut on_thinking: F,
    mut on_text: T,
    mut on_tool: E,
) -> Result<StreamParseState, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnMut(&str),
    T: FnMut(&str),
    E: FnMut(&crate::cli_activity::ToolEvent),
{
    let stdout = child.stdout.take().ok_or("无法读取 Codex 输出")?;
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut output = Vec::new();
            let mut buffer = [0u8; 4096];
            // Continue draining after the diagnostic cap to avoid blocking CLI.
            while let Ok(count) = stderr.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                let keep = count.min((16 * 1024usize).saturating_sub(output.len()));
                output.extend_from_slice(&buffer[..keep]);
            }
            String::from_utf8_lossy(&output).into_owned()
        })
    });
    let mut lines = BufReader::new(stdout).lines();
    let mut state = StreamParseState::new();
    let mut parser = EventParser::default();
    let mut activity = crate::cli_activity::CliActivityParser::default();
    let mut read_error = None;
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                for event in activity.codex(&line) {
                    if let crate::cli_activity::Activity::Tool(tool) = event { on_tool(&tool); }
                }
                for delta in parser.parse_line(&mut state, &line) {
                    match delta {
                        Delta::Thinking(delta) => on_thinking(&delta),
                        Delta::Text(delta) => on_text(&delta),
                    }
                }
            }
            Ok(None) => break,
            Err(error) => {
                read_error = Some(error);
                let _ = child.kill().await;
                break;
            }
        }
    }
    let status = child.wait().await?;
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if let Some(error) = read_error {
        return Err(error.into());
    }
    parser.finish(&mut state, status.success(), &stderr);
    Ok(state)
}

#[derive(Serialize)]
pub struct CodexStatus {
    installed: bool,
    logged_in: bool,
    version: Option<String>,
    executable: Option<String>,
    workspace: String,
    message: String,
}

pub async fn check_status(app: &tauri::AppHandle) -> Result<CodexStatus, String> {
    let mut status = CodexStatus {
        installed: false,
        logged_in: false,
        version: None,
        executable: None,
        workspace: workspace_dir(app)?.to_string_lossy().into_owned(),
        message: String::new(),
    };
    let path = match find_cli() {
        Ok(path) => path,
        Err(message) => {
            status.message = message;
            return Ok(status);
        }
    };
    status.executable = Some(path.to_string_lossy().into_owned());
    let version = tokio::time::timeout(
        Duration::from_secs(10),
        cli_command(&path)
            .arg("--version")
            .stdin(Stdio::null())
            .output(),
    )
    .await;
    match version {
        Ok(Ok(output)) if output.status.success() => {
            status.installed = true;
            status.version = Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
        _ => {
            status.message = "找到 Codex，但无法启动；请检查 Node.js / Codex 安装后刷新。".into();
            return Ok(status);
        }
    }
    let login = tokio::time::timeout(
        Duration::from_secs(10),
        cli_command(&path)
            .args(["login", "status"])
            .stdin(Stdio::null())
            .output(),
    )
    .await;
    // login status may print part of an API key. Never return its output to UI.
    match login {
        Ok(Ok(output)) => {
            status.logged_in = output.status.success();
            status.message = if status.logged_in {
                "已连接，使用本机 Codex 的登录信息和模型配置。"
            } else {
                "尚未登录，请点击登录 Codex，完成后刷新状态。"
            }
            .into();
        }
        _ => status.message = "登录状态检测失败或超时，请在终端检查 codex login status。".into(),
    }
    Ok(status)
}

pub fn open_login() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // The user explicitly opens this interactive login window from the UI.
        let executable = find_cli()?.to_string_lossy().replace('\'', "''");
        let script = format!("& '{executable}' login");
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NoExit", "-Command", &script])
            .spawn()
            .map_err(|error| format!("无法打开 Codex 登录终端：{error}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("请在本机终端运行 codex login，然后刷新连接状态。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chinese_messages_without_repeating_completed_items() {
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        parser.parse_line(
            &mut state,
            r#"{"type":"thread.started","thread_id":"0199a213-81c0-7800-8aa1-bbab2a035a53"}"#,
        );
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"先检查配置"}}"#),
            vec![Delta::Thinking("先检查配置".into())]
        );
        let answer = r#"{"type":"item.completed","item":{"id":"a1","type":"agent_message","text":"连接正常。"}}"#;
        assert_eq!(
            parser.parse_line(&mut state, answer),
            vec![Delta::Text("连接正常。".into())]
        );
        assert!(parser.parse_line(&mut state, answer).is_empty());
        parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"cmd1","type":"command_execution","aggregated_output":"private command output"}}"#);
        parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"a2","type":"agent_message","text":"可以继续对话。"}}"#);
        parser.parse_line(&mut state, r#"{"type":"turn.completed"}"#);
        parser.finish(&mut state, true, "harmless warning");
        assert!(!state.is_error);
        assert_eq!(state.full_text, "连接正常。\n\n可以继续对话。");
        assert_eq!(state.full_thinking, "先检查配置");
        assert_eq!(state.text_len, state.full_text.len());
        assert_eq!(state.thinking_len, state.full_thinking.len());
        assert!(state.session_id.is_some());
    }

    #[test]
    fn ordered_deltas_keep_reasoning_before_the_answer_that_follows_it() {
        // 事件顺序取自真实 codex exec --json 输出。
        let lines = [
            r#"{"type":"thread.started","thread_id":"01a13874-fe8d-7ad1-a47d-dd45d8cb2e6c"}"#,
            r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 3 unrecognized configuration settings."}}"#,
            r#"{"type":"turn.started"}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"我会先查看当前目录。"}}"#,
            r#"{"type":"item.started","item":{"id":"item_3","type":"command_execution","command":"Get-ChildItem","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_4","type":"reasoning","text":"**Exploring command execution**\n\n先确认工具行为。"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_5","type":"command_execution","command":"Get-ChildItem","aggregated_output":"failed","exit_code":-1,"status":"failed"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_8","type":"agent_message","text":"任务未完成。"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":96970}}"#,
        ];
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        let mut deltas = Vec::new();
        for line in lines {
            deltas.extend(parser.parse_line(&mut state, line));
        }
        parser.finish(&mut state, true, "");
        assert!(!state.is_error);
        assert_eq!(
            deltas,
            vec![
                Delta::Text("我会先查看当前目录。".into()),
                Delta::Thinking("**Exploring command execution**\n\n先确认工具行为。".into()),
                Delta::Text("任务未完成。".into()),
            ]
        );
        assert_eq!(state.full_text, "我会先查看当前目录。\n\n任务未完成。");
    }

    #[test]
    fn streamed_reasoning_only_reports_new_text_and_never_repeats() {
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.started","item":{"id":"r1","type":"reasoning","text":"第一步"}}"#),
            vec![Delta::Thinking("第一步".into())]
        );
        // 同一份文本再次推送（updated）不产生任何增量。
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一步"}}"#)
            .is_empty());
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一步，第二步"}}"#),
            vec![Delta::Thinking("，第二步".into())]
        );
        // completed 只补尚未流出的尾部。
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一步，第二步，结论"}}"#),
            vec![Delta::Thinking("，结论".into())]
        );
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一步，第二步，结论"}}"#)
            .is_empty());
        assert_eq!(state.full_thinking, "第一步，第二步，结论");
        assert_eq!(state.thinking_len, state.full_thinking.len());
    }

    #[test]
    fn shorter_reasoning_updates_never_replay_the_middle_of_the_summary() {
        // 回归：先收到全文、再收到更短的 updated 时，缓存不能被改短，
        // 否则定稿事件会按最长公共前缀把中段当成新增内容重复上报。
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一段思考内容，结论"}}"#),
            vec![Delta::Thinking("第一段思考内容，结论".into())]
        );
        assert!(
            parser
                .parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一段思考内容"}}"#)
                .is_empty(),
            "shorter update must be ignored"
        );
        assert!(
            parser
                .parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一段思考内容，结论"}}"#)
                .is_empty(),
            "final event must not replay the middle"
        );
        assert_eq!(state.full_thinking, "第一段思考内容，结论");
    }

    #[test]
    fn settled_reasoning_appends_only_the_tail_beyond_streamed_text() {
        // 钉住"定稿比已流出内容更长"的分支：只补尾部，不重复已流出的部分。
        // （上一条用例只覆盖了"定稿 == 已缓存"的零增量情形。）
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"r1","type":"reasoning","text":"第一段"}}"#),
            vec![Delta::Thinking("第一段".into())]
        );
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"第一段，第二段"}}"#),
            vec![Delta::Thinking("，第二段".into())]
        );
        assert_eq!(state.full_thinking, "第一段，第二段");
        assert_eq!(state.thinking_len, state.full_thinking.len());
    }

    #[test]
    fn agent_message_is_only_settled_by_item_completed() {
        // 回归：若某版本用 started/updated 增量推送正文，定稿事件不能被丢弃，
        // 否则 full_text（即最终回复）会被截断。
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.updated","item":{"id":"m1","type":"agent_message","text":"前半段"}}"#)
            .is_empty());
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.started","item":{"id":"m1","type":"agent_message","text":"前半段"}}"#)
            .is_empty());
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"前半段，后半段。"}}"#),
            vec![Delta::Text("前半段，后半段。".into())]
        );
        assert_eq!(state.full_text, "前半段，后半段。");
        // 定稿重复到达时仍然只算一次。
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"m1","type":"agent_message","text":"前半段，后半段。"}}"#)
            .is_empty());
        assert_eq!(state.full_text, "前半段，后半段。");
    }

    #[test]
    fn reasoning_falls_back_to_summary_text_and_raw_content() {
        let mut parser = EventParser::default();
        let mut state = StreamParseState::new();
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"","summary_text":"摘要思考"}}"#),
            vec![Delta::Thinking("摘要思考".into())]
        );
        assert_eq!(
            parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r2","type":"reasoning","raw_content":[{"type":"text","text":"原始甲"},{"type":"text","text":"原始乙"}]}}"#),
            vec![Delta::Thinking("原始甲\n原始乙".into())]
        );
        // 不同条目之间插入空行，同一条目内部保持连续。
        assert_eq!(state.full_thinking, "摘要思考\n\n原始甲\n原始乙");
        assert!(parser
            .parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r3","type":"reasoning"}}"#)
            .is_empty());
    }

    #[test]
    fn exec_args_ask_for_detailed_reasoning_summaries() {
        let args = exec_args(None, None);
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c", "model_reasoning_summary=\"detailed\""]),
            "reasoning summary config missing: {args:?}"
        );
        let sandbox = args.iter().position(|arg| arg == "--sandbox").unwrap();
        assert_eq!(args[sandbox + 1], "workspace-write");
        assert!(args.iter().any(|arg| arg == "approval_policy=\"never\""));
        assert!(args.iter().any(|arg| arg == "--json"));
        assert!(args.iter().any(|arg| arg == "--skip-git-repo-check"));
    }

    #[test]
    fn detects_failed_and_truncated_turns_even_after_partial_output() {
        for (line, expected) in [
            (
                r#"{"type":"turn.failed","error":{"message":"authentication failed"}}"#,
                "authentication failed",
            ),
            (
                r#"{"type":"error","message":"connection lost"}"#,
                "connection lost",
            ),
        ] {
            let mut parser = EventParser::default();
            let mut state = StreamParseState::new();
            state.full_text = "partial response".into();
            parser.parse_line(&mut state, line);
            parser.finish(&mut state, false, "stderr fallback");
            assert!(state.is_error);
            assert_eq!(state.error_msg.as_deref(), Some(expected));
        }
        let mut state = StreamParseState::new();
        EventParser::default().finish(&mut state, true, "");
        assert!(state.is_error, "EOF without turn.completed is not success");
    }

    #[test]
    fn successful_retry_ignores_transient_errors_and_non_json_lines() {
        let mut state = StreamParseState::new();
        let mut parser = EventParser::default();
        parser.parse_line(&mut state, "CLI warning");
        parser.parse_line(&mut state, r#"{"type":"error","message":"retrying"}"#);
        parser.parse_line(&mut state, r#"{"type":"turn.completed"}"#);
        parser.finish(&mut state, true, "");
        assert!(!state.is_error);
        parser.finish(&mut state, false, "process failed");
        assert!(state.is_error, "nonzero exit must still fail");
    }

    #[test]
    fn resumes_only_its_own_session_and_resets_it() {
        let adapter = CodexAdapter::default();
        assert!(!exec_args(None, None).iter().any(|arg| arg == "resume"));
        adapter.update_session("0199a213-81c0-7800-8aa1-bbab2a035a53");
        let args = exec_args(adapter.session_id.lock().unwrap().as_deref(), None);
        assert!(args
            .windows(2)
            .any(|args| args == ["resume", "0199a213-81c0-7800-8aa1-bbab2a035a53"]));
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(!args
            .iter()
            .any(|arg| arg == "--last" || arg.contains("dangerously")));
        adapter.reset_session();
        adapter.update_session("invalid session & shell command");
        assert!(adapter.session_id.lock().unwrap().is_none());
    }

    #[test]
    fn explicit_model_is_passed_before_the_resume_subcommand() {
        let args = exec_args(Some("0199a213-81c0-7800-8aa1-bbab2a035a53"), Some("gpt-5-codex"));
        let model = args.iter().position(|arg| arg == "-m").expect("缺少 -m");
        assert_eq!(args[model + 1], "gpt-5-codex");
        let resume = args.iter().position(|arg| arg == "resume").expect("缺少 resume");
        assert!(model < resume, "-m 必须在 resume 子命令之前: {args:?}");
        assert_eq!(args.last().map(String::as_str), Some("-"));
        // 空模型字符串表示跟随本机配置，不追加参数。
        assert!(!exec_args(None, Some("  ")).iter().any(|arg| arg == "-m"));
    }

    // Opt in with: cargo test --lib codex::tests::local_cli_round_trip -- --ignored
    // Uses the installed CLI/account, so it is excluded from normal unit tests.
    #[tokio::test]
    #[ignore = "requires a locally installed and authenticated Codex CLI"]
    async fn local_cli_round_trip() {
        let workspace =
            std::env::temp_dir().join(format!("desktop-pet-codex-{}", uuid::Uuid::new_v4()));
        let adapter = CodexAdapter::default();
        let marker = format!("桌宠校验-{}", uuid::Uuid::new_v4());
        let prompt = "这是桌宠接入测试。只回答用户要求的内容，不要调用工具或读取文件。";
        let mut first = adapter
            .spawn_in_workspace(
                &workspace,
                &format!("记住并只回复这个校验码：{marker}"),
                prompt,
            )
            .await
            .expect("start local Codex");
        let first_pid = first.id();
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            read_stream(&mut first, |_| {}, |_| {}),
        )
        .await;
        if result.is_err() {
            if let Some(pid) = first_pid {
                crate::kill_process_tree(pid);
            }
        }
        let first = result
            .expect("first Codex turn timed out")
            .expect("read first turn");
        assert!(
            !first.is_error,
            "local Codex failed; check CLI login/provider configuration"
        );
        assert!(
            first.full_text.contains(&marker),
            "Chinese stdin prompt was not preserved"
        );
        adapter.update_session(first.session_id.as_deref().expect("thread ID"));

        let mut second = adapter
            .spawn_in_workspace(&workspace, "只回复我上一条消息中的完整校验码。", prompt)
            .await
            .expect("resume local Codex");
        let second_pid = second.id();
        let result = tokio::time::timeout(
            Duration::from_secs(120),
            read_stream(&mut second, |_| {}, |_| {}),
        )
        .await;
        if result.is_err() {
            if let Some(pid) = second_pid {
                crate::kill_process_tree(pid);
            }
        }
        let second = result
            .expect("resumed Codex turn timed out")
            .expect("read resumed turn");
        assert!(
            !second.is_error,
            "resumed Codex failed; check CLI configuration"
        );
        assert_eq!(first.session_id, second.session_id);
        assert!(
            second.full_text.contains(&marker),
            "resumed turn lost conversation context"
        );
        adapter.reset_session();
        // Only remove the empty directory we created, never recurse into CLI output.
        let _ = std::fs::remove_dir(workspace);
    }
}
