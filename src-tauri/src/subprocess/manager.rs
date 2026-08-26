use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin},
    sync::Mutex,
    time::{sleep, Duration},
};

use crate::direct_api::is_path_allowed;

pub const EVENT_SUBPROC_STDOUT: &str = "subproc-stdout";
pub const EVENT_SUBPROC_STDERR: &str = "subproc-stderr";
pub const EVENT_SUBPROC_EXIT: &str = "subproc-exit";

const MAX_CONCURRENT: usize = 8;
const MAX_STDOUT_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(100);
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_millis(1_500);

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnParams {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub append_newline: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub buffer_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StdoutPayload {
    pub id: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StderrPayload {
    pub id: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitPayload {
    pub id: String,
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub label: String,
}

struct ChildHandle {
    child: Child,
    stdin: Option<ChildStdin>,
    info: ProcessInfo,
    buffer_bytes: usize,
}

pub struct SubprocessManager {
    procs: Mutex<HashMap<String, Arc<Mutex<ChildHandle>>>>,
}

impl SubprocessManager {
    pub fn new() -> Self {
        Self {
            procs: Mutex::new(HashMap::new()),
        }
    }

    async fn count_alive(&self) -> usize {
        let procs = self.procs.lock().await;
        let mut n = 0;
        for h in procs.values() {
            let h = h.lock().await;
            if h.info.status == "running" || h.info.status == "stopping" {
                n += 1;
            }
        }
        n
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

fn command_is_allowed(command: &str) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("命令不能为空".to_string());
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("..") || lower.contains("|") || lower.contains("&") || lower.contains(";") {
        return Err("命令包含非法字符".to_string());
    }
    let exe = PathBuf::from(trimmed);
    let file_name = exe
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let allowed = [
        "node.exe",
        "node",
        "python.exe",
        "python",
        "python3.exe",
        "python3",
        "pip.exe",
        "pip",
        "npm.cmd",
        "npm",
        "git.exe",
        "git",
        "ping.exe",
        "ping",
        "cmd.exe",
        "powershell.exe",
    ];
    if !allowed.iter().any(|a| file_name == *a) {
        return Err(format!("命令 {} 不在白名单中", file_name));
    }
    Ok(())
}

fn validate_cwd(cwd: &Option<String>) -> Result<Option<PathBuf>, String> {
    match cwd {
        None => Ok(None),
        Some(c) => {
            if c.trim().is_empty() {
                return Ok(None);
            }
            let canonical = is_path_allowed(c)?;
            Ok(Some(canonical))
        }
    }
}

pub async fn spawn_subprocess(
    app: AppHandle,
    manager: &SubprocessManager,
    params: SpawnParams,
) -> Result<ProcessInfo, String> {
    command_is_allowed(&params.command)?;
    let cwd = validate_cwd(&params.cwd)?;

    if manager.count_alive().await >= MAX_CONCURRENT {
        return Err(format!("已达到最大并发数 {}", MAX_CONCURRENT));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let label = if params.label.trim().is_empty() {
        PathBuf::from(&params.command)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("process")
            .to_string()
    } else {
        params.label.trim().to_string()
    };

    let mut cmd = tokio::process::Command::new(&params.command);
    cmd.args(&params.args);
    if let Some(cwd) = &cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in &params.env {
        cmd.env(k, v);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("启动进程失败: {}", e))?;
    let pid = child.id();

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let info = ProcessInfo {
        id: id.clone(),
        label: label.clone(),
        command: params.command.clone(),
        args: params.args.clone(),
        pid,
        started_at: unix_now(),
        status: "running".to_string(),
        exit_code: None,
        buffer_truncated: false,
    };

    let handle = Arc::new(Mutex::new(ChildHandle {
        child,
        stdin,
        info: info.clone(),
        buffer_bytes: 0,
    }));

    {
        let mut procs = manager.procs.lock().await;
        procs.insert(id.clone(), handle.clone());
    }

    if let Some(stdout) = stdout {
        spawn_reader(app.clone(), id.clone(), stdout, false, handle.clone());
    }
    if let Some(stderr) = stderr {
        spawn_reader(app.clone(), id.clone(), stderr, true, handle.clone());
    }

    spawn_waiter(app.clone(), id.clone(), label.clone(), handle);

    Ok(info)
}

fn spawn_reader<R>(
    app: AppHandle,
    id: String,
    stream: R,
    is_stderr: bool,
    handle: Arc<Mutex<ChildHandle>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buf = String::new();
        loop {
            buf.clear();
            match reader.read_line(&mut buf).await {
                Ok(0) => break,
                Ok(_) => {
                    let line = buf.trim_end_matches(['\r', '\n']).to_string();
                    {
                        let mut h = handle.lock().await;
                        h.buffer_bytes = h.buffer_bytes.saturating_add(line.len());
                        if h.buffer_bytes > MAX_STDOUT_BUFFER_BYTES {
                            h.info.buffer_truncated = true;
                            continue;
                        }
                    }
                    let payload = if is_stderr {
                        serde_json::to_value(StderrPayload {
                            id: id.clone(),
                            line: line.clone(),
                        })
                        .ok()
                    } else {
                        serde_json::to_value(StdoutPayload {
                            id: id.clone(),
                            line: line.clone(),
                        })
                        .ok()
                    };
                    if let Some(p) = payload {
                        let event = if is_stderr {
                            EVENT_SUBPROC_STDERR
                        } else {
                            EVENT_SUBPROC_STDOUT
                        };
                        let _ = app.emit(event, p);
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn spawn_waiter(app: AppHandle, id: String, label: String, handle: Arc<Mutex<ChildHandle>>) {
    tokio::spawn(async move {
        let status = wait_for_child_exit(&handle).await;
        let (code, signal) = match status {
            Ok(s) => (s.code(), None),
            Err(_) => (None, None),
        };
        {
            let mut h = handle.lock().await;
            h.info.status = "exited".to_string();
            h.info.exit_code = code;
        }
        let _ = app.emit(
            EVENT_SUBPROC_EXIT,
            ExitPayload {
                id: id.clone(),
                code,
                signal,
                label,
            },
        );
        let manager = app.state::<crate::AppState>();
        let mut procs = manager.subprocs.procs.lock().await;
        procs.remove(&id);
    });
}

async fn wait_for_child_exit(
    handle: &Arc<Mutex<ChildHandle>>,
) -> std::io::Result<std::process::ExitStatus> {
    loop {
        let status = {
            let mut h = handle.lock().await;
            h.child.try_wait()
        }?;
        if let Some(status) = status {
            return Ok(status);
        }
        sleep(PROCESS_POLL_INTERVAL).await;
    }
}

pub async fn send_stdin(
    manager: &SubprocessManager,
    id: String,
    text: String,
    append_newline: bool,
) -> Result<(), String> {
    let procs = manager.procs.lock().await;
    let handle = procs
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("进程 {} 不存在", id))?;
    drop(procs);

    let mut h = handle.lock().await;
    let stdin = h
        .stdin
        .as_mut()
        .ok_or_else(|| "进程 stdin 已关闭".to_string())?;
    let payload = if append_newline {
        format!("{}\n", text)
    } else {
        text
    };
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| format!("写入 stdin 失败: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("flush 失败: {}", e))?;
    Ok(())
}

pub async fn kill_subprocess(
    manager: &SubprocessManager,
    id: String,
    force: bool,
) -> Result<(), String> {
    let procs = manager.procs.lock().await;
    let handle = procs
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("进程 {} 不存在", id))?;
    drop(procs);

    if force {
        let mut h = handle.lock().await;
        h.child
            .kill()
            .await
            .map_err(|e| format!("kill 失败: {}", e))?;
        h.info.status = "stopping".to_string();
    } else {
        {
            let mut h = handle.lock().await;
            if h.info.status != "running" {
                return Ok(());
            }
            h.info.status = "stopping".to_string();
            h.stdin.take();
        }
        sleep(GRACEFUL_STOP_TIMEOUT).await;
        let mut h = handle.lock().await;
        let still_running = h.child.try_wait().is_ok_and(|status| status.is_none());
        if still_running {
            h.child.kill().await.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub async fn list_subprocesses(manager: &SubprocessManager) -> Result<Vec<ProcessInfo>, String> {
    let procs = manager.procs.lock().await;
    let mut out = Vec::with_capacity(procs.len());
    for h in procs.values() {
        let h = h.lock().await;
        out.push(h.info.clone());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[tokio::test]
    async fn waiting_for_exit_does_not_block_process_control() {
        let mut command = if cfg!(target_os = "windows") {
            let mut command = tokio::process::Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 5"]);
            command
        } else {
            let mut command = tokio::process::Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("spawn test process");
        let stdin = child.stdin.take();
        let handle = Arc::new(Mutex::new(ChildHandle {
            child,
            stdin,
            info: ProcessInfo {
                id: "test".to_string(),
                label: "test".to_string(),
                command: "test".to_string(),
                args: Vec::new(),
                pid: None,
                started_at: unix_now(),
                status: "running".to_string(),
                exit_code: None,
                buffer_truncated: false,
            },
            buffer_bytes: 0,
        }));

        let waiter_handle = Arc::clone(&handle);
        let waiter = tokio::spawn(async move { wait_for_child_exit(&waiter_handle).await });
        sleep(Duration::from_millis(150)).await;

        let lock = tokio::time::timeout(Duration::from_millis(250), handle.lock())
            .await
            .expect("process control lock must remain available while waiting");
        drop(lock);

        {
            let mut locked = handle.lock().await;
            locked.child.kill().await.expect("kill test process");
        }
        waiter.await.expect("waiter task").expect("wait for exit");
    }

    #[test]
    fn command_allowlist_rejects_shell_chaining() {
        assert!(command_is_allowed("powershell.exe; calc.exe").is_err());
        assert!(command_is_allowed("cmd.exe & whoami").is_err());
        assert!(command_is_allowed("node.exe").is_ok());
    }
}
