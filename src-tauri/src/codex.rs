//! Local Codex CLI integration. Authentication and model selection stay in Codex.
use crate::openclaw::StreamParseState;
use serde::Serialize;
use std::{collections::HashSet, path::PathBuf, process::Stdio, sync::Mutex, time::Duration};
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

fn exec_args(session_id: Option<&str>) -> Vec<String> {
    // exec cannot ask for approval in the desktop chat. Keep a bounded writable
    // workspace and fail commands that would need additional permission.
    let mut args: Vec<String> = [
        "exec",
        "--sandbox",
        "workspace-write",
        "-c",
        "approval_policy=\"never\"",
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect();
    if let Some(id) = session_id {
        args.extend(["resume".into(), id.into()]);
    }
    args.extend(["--json".into(), "--skip-git-repo-check".into(), "-".into()]);
    args
}

impl CodexAdapter {
    pub async fn spawn_streaming(
        &self,
        app: &tauri::AppHandle,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        let workspace = workspace_dir(app)?;
        self.spawn_in_workspace(&workspace, message, system_prompt)
            .await
    }

    async fn spawn_in_workspace(
        &self,
        workspace: &std::path::Path,
        message: &str,
        system_prompt: &str,
    ) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
        std::fs::create_dir_all(&workspace)?;
        let session = self.session_id.lock().unwrap().clone();
        let mut child = cli_command(&find_cli()?)
            .args(exec_args(session.as_deref()))
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

#[derive(Default)]
struct EventParser {
    completed_items: HashSet<String>,
    turn_completed: bool,
    last_error: Option<String>,
}

impl EventParser {
    fn parse_line(&mut self, state: &mut StreamParseState, line: &str) -> Option<String> {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return None;
        };
        match event["type"].as_str().unwrap_or_default() {
            "thread.started" => {
                state.session_id = event["thread_id"].as_str().map(str::to_string);
            }
            "item.completed" => {
                let item = &event["item"];
                if let Some(id) = item["id"].as_str() {
                    if !self.completed_items.insert(id.to_string()) {
                        return None;
                    }
                }
                let text = item["text"].as_str().unwrap_or_default();
                if text.is_empty() {
                    return None;
                }
                match item["type"].as_str() {
                    Some("agent_message") => {
                        if !state.full_text.is_empty() {
                            state.full_text.push_str("\n\n");
                        }
                        state.full_text.push_str(text);
                        state.text_len = state.full_text.len();
                    }
                    Some("reasoning") => {
                        let delta = if state.full_thinking.is_empty() {
                            text.to_string()
                        } else {
                            format!("\n\n{text}")
                        };
                        state.full_thinking.push_str(&delta);
                        state.thinking_len = state.full_thinking.len();
                        return Some(delta);
                    }
                    _ => {}
                }
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
        None
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
                let text_len = state.full_text.len();
                if let Some(delta) = parser.parse_line(&mut state, &line) {
                    on_thinking(&delta);
                }
                if state.full_text.len() > text_len {
                    on_text(&state.full_text[text_len..]);
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
        assert_eq!(parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"先检查配置"}}"#).as_deref(), Some("先检查配置"));
        let answer = r#"{"type":"item.completed","item":{"id":"a1","type":"agent_message","text":"连接正常。"}}"#;
        parser.parse_line(&mut state, answer);
        parser.parse_line(&mut state, answer);
        parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"cmd1","type":"command_execution","aggregated_output":"private command output"}}"#);
        parser.parse_line(&mut state, r#"{"type":"item.completed","item":{"id":"a2","type":"agent_message","text":"可以继续对话。"}}"#);
        parser.parse_line(&mut state, r#"{"type":"turn.completed"}"#);
        parser.finish(&mut state, true, "harmless warning");
        assert!(!state.is_error);
        assert_eq!(state.full_text, "连接正常。\n\n可以继续对话。");
        assert_eq!(state.full_thinking, "先检查配置");
        assert!(state.session_id.is_some());
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
        assert!(!exec_args(None).iter().any(|arg| arg == "resume"));
        adapter.update_session("0199a213-81c0-7800-8aa1-bbab2a035a53");
        let args = exec_args(adapter.session_id.lock().unwrap().as_deref());
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
