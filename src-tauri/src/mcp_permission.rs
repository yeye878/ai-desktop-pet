//! Claude Code 的授权确认桥。
//!
//! claude -p 在遇到需要授权的工具调用时，会把决定权交给 --permission-prompt-tool
//! 指定的 MCP 工具。桌宠在本机回环地址上跑一个只暴露这一个工具的 MCP HTTP 服务：
//! claude 用 --mcp-config 拿到地址，工具被调用时桌宠弹出与直连 API 相同的确认窗口，
//! 再把用户的决定按 {behavior: "allow" | "deny", message} 回给它。
//!
//! 服务只在 127.0.0.1 上监听，路径里带一次性随机 token，避免同机其他进程驱动弹窗。

use crate::direct_api::ToolConfirmPayload;
use crate::interactions::WaitResult;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

/// MCP 工具名（claude 侧的完整写法是 mcp__<server>__<tool>）。
pub const TOOL_NAME: &str = "confirm_permission";
pub const SERVER_NAME: &str = "pet";
/// claude 侧要填进 --permission-prompt-tool 的完整名字。
pub const PROMPT_TOOL: &str = "mcp__pet__confirm_permission";
const PROTOCOL_VERSION: &str = "2024-11-05";
/// 授权等待上限：到点按拒绝处理，失败关闭。
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);
/// 单次请求体上限，防止畸形请求把内存吃满。
const MAX_BODY: usize = 512 * 1024;

/// claude 交给权限工具的调用内容。
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub input: Value,
    pub tool_use_id: Option<String>,
}

/// 用户的决定。
#[derive(Clone, Debug, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Deny(String),
}

impl PermissionDecision {
    /// claude 的权限工具结果契约。
    fn to_json(&self) -> Value {
        match self {
            PermissionDecision::Allow => json!({ "behavior": "allow" }),
            PermissionDecision::Deny(message) => json!({ "behavior": "deny", "message": message }),
        }
    }
}

/// 谁来决定一次授权请求。生产路径是 UiPermission，测试注入自己的实现。
#[async_trait]
pub trait PermissionAnswerer: Send + Sync {
    async fn decide(&self, request: PermissionRequest) -> PermissionDecision;
}

/// 走桌宠确认弹窗（与直连 API、dsh 同一套）。
///
/// 桥的生命周期比单轮长，所以取消令牌在每次询问时从 AppState 现取：用户按下的
/// 停止按钮作用于当前这一轮。
pub struct UiPermission {
    app: tauri::AppHandle,
}

impl UiPermission {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    async fn current_cancel(&self) -> CancellationToken {
        let state = self.app.state::<crate::AppState>();
        let token = state.abort_token.lock().await.clone();
        token.unwrap_or_default()
    }
}

#[async_trait]
impl PermissionAnswerer for UiPermission {
    async fn decide(&self, request: PermissionRequest) -> PermissionDecision {
        let payload = confirm_payload(&request);
        let cancel = self.current_cancel().await;
        match crate::interactions::confirm_with_ui(&self.app, &cancel, payload, CONFIRM_TIMEOUT).await
        {
            WaitResult::Answered(true) => PermissionDecision::Allow,
            WaitResult::Answered(false) => PermissionDecision::Deny("用户在桌宠里拒绝了这次调用".into()),
            WaitResult::TimedOut => PermissionDecision::Deny("等待用户确认超时，已按拒绝处理".into()),
            WaitResult::Cancelled => PermissionDecision::Deny("本轮已取消".into()),
            WaitResult::Closed => PermissionDecision::Deny("确认窗口已关闭".into()),
        }
    }
}

/// 把 MCP 调用翻译成确认弹窗的载荷：命令与路径照直连 API 的取值方式提取。
pub fn confirm_payload(request: &PermissionRequest) -> ToolConfirmPayload {
    let input = &request.input;
    let command = input["command"].as_str().map(str::to_string);
    let path = input["file_path"]
        .as_str()
        .or_else(|| input["path"].as_str())
        .or_else(|| input["notebook_path"].as_str())
        .map(str::to_string);
    let mut summary = command
        .clone()
        .or_else(|| path.clone())
        .unwrap_or_else(|| request.tool_name.clone());
    if let Some(description) = input["description"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
    {
        summary.push_str(&format!(
            "（{}）",
            description.chars().take(80).collect::<String>()
        ));
    }
    let arguments = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
    ToolConfirmPayload {
        id: format!("confirm_{}", uuid::Uuid::new_v4()),
        tool_name: request.tool_name.clone(),
        arguments,
        summary,
        command,
        path,
    }
}

/// claude 读取的 MCP 配置文件名（写在应用数据目录里）。
const CONFIG_FILENAME: &str = "claude-mcp-permission.json";

/// claude 用的 MCP 配置文件路径；写文件比在命令行里塞 JSON 更稳。
pub fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(CONFIG_FILENAME))
        .map_err(|error| error.to_string())
}

/// 把桥写进 claude 的 MCP 配置文件，返回路径。
pub fn write_config(app: &tauri::AppHandle, bridge: &BridgeHandle) -> Result<PathBuf, String> {
    let path = config_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, bridge.mcp_config_json())
        .map_err(|error| format!("无法写入 {}：{error}", path.display()))?;
    Ok(path)
}

/// 运行中的桥：端口、路径 token，以及给 claude 用的 MCP 配置文本。
pub struct BridgeHandle {
    pub port: u16,
    token: String,
}

impl BridgeHandle {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp/{}", self.port, self.token)
    }

    /// --mcp-config 接受 JSON 文件或字符串；桌宠写文件，避免命令行转义问题。
    pub fn mcp_config_json(&self) -> String {
        json!({
            "mcpServers": {
                SERVER_NAME: { "type": "http", "url": self.url() }
            }
        })
        .to_string()
    }
}

/// 启动桥服务。绑定回环地址的随机端口，返回的句柄带 token 与端口。
pub async fn serve(answerer: Arc<dyn PermissionAnswerer>) -> Result<BridgeHandle, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("无法启动授权确认服务：{error}"))?;
    let port = listener.local_addr().map_err(|error| error.to_string())?.port();
    let token = uuid::Uuid::new_v4().to_string();
    let expected = format!("/mcp/{token}");
    let state: SharedState = Arc::new((answerer, expected));
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = state.clone();
            tokio::spawn(async move {
                let _ = handle_connection(stream, state).await;
            });
        }
    });
    Ok(BridgeHandle { port, token })
}

type SharedState = Arc<(Arc<dyn PermissionAnswerer>, String)>;

async fn handle_connection(mut stream: TcpStream, state: SharedState) -> Result<(), String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let (header_end, content_length) = loop {
        let count = stream.read(&mut chunk).await.map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..count]);
        if let Some(position) = find_header_end(&buffer) {
            let head = String::from_utf8_lossy(&buffer[..position]).to_string();
            break (position, content_length_of(&head));
        }
        if buffer.len() > MAX_BODY {
            let _ = respond(&mut stream, 413, "text/plain", "").await;
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let count = stream.read(&mut chunk).await.map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..count]);
    }

    if !target.starts_with(state.1.as_str()) {
        let _ = respond(&mut stream, 404, "text/plain", "not found").await;
        return Ok(());
    }
    if method != "POST" {
        let _ = respond(&mut stream, 405, "text/plain", "method not allowed").await;
        return Ok(());
    }
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        let _ = respond(&mut stream, 400, "text/plain", "invalid json").await;
        return Ok(());
    };

    match dispatch(&state.0, message).await {
        Some(reply) => {
            let text = reply.to_string();
            let _ = respond(&mut stream, 200, "application/json", &text).await;
        }
        // 通知（例如 notifications/initialized）不需要响应体。
        None => {
            let _ = respond(&mut stream, 202, "text/plain", "").await;
        }
    }
    Ok(())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length_of(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if !key.eq_ignore_ascii_case("content-length") {
                return None;
            }
            value.trim().parse::<usize>().ok()
        })
        .unwrap_or(0)
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "OK",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(body.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream.flush().await.map_err(|error| error.to_string())
}

/// 处理一条 JSON-RPC 消息；通知返回 None（按 MCP 约定不回包）。
async fn dispatch(answerer: &Arc<dyn PermissionAnswerer>, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or_default().to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    // 通知（没有 id）不需要回包。
    let id = match id {
        Some(value) if value != Value::Null => value,
        _ => return None,
    };

    let result: Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "ai-desktop-pet", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": [{
                "name": TOOL_NAME,
                "description": "Ask the desktop pet user to approve or deny a tool call. Returns {\"behavior\":\"allow\"} when the user approves and {\"behavior\":\"deny\",\"message\":...} when they do not.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "tool_name": { "type": "string", "description": "Name of the tool requesting permission." },
                        "input": { "type": "object", "description": "The tool input as the model produced it." },
                        "tool_use_id": { "type": "string", "description": "Identifier of the tool-use block." }
                    },
                    "required": ["tool_name"]
                }
            }]
        })),
        "tools/call" => {
            if params["name"].as_str() != Some(TOOL_NAME) {
                Err((-32602, format!("unknown tool {}", params["name"])))
            } else {
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                let request = PermissionRequest {
                    tool_name: arguments["tool_name"].as_str().unwrap_or_default().to_string(),
                    input: arguments.get("input").cloned().unwrap_or(json!({})),
                    tool_use_id: arguments["tool_use_id"].as_str().map(str::to_string),
                };
                let decision = answerer.decide(request).await;
                Ok(json!({
                    "content": [{ "type": "text", "text": decision.to_json().to_string() }],
                    "isError": false
                }))
            }
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder {
        decision: PermissionDecision,
        seen: Arc<Mutex<Vec<PermissionRequest>>>,
    }

    #[async_trait]
    impl PermissionAnswerer for Recorder {
        async fn decide(&self, request: PermissionRequest) -> PermissionDecision {
            self.seen.lock().unwrap().push(request);
            self.decision.clone()
        }
    }

    fn request(tool: &str, input: Value) -> PermissionRequest {
        PermissionRequest {
            tool_name: tool.into(),
            input,
            tool_use_id: Some("toolu_1".into()),
        }
    }

    #[test]
    fn builds_a_confirm_payload_for_bash_and_write_tools() {
        let payload = confirm_payload(&request(
            "Bash",
            json!({ "command": "npm test", "description": "Run the test suite" }),
        ));
        assert_eq!(payload.tool_name, "Bash");
        assert_eq!(payload.command.as_deref(), Some("npm test"));
        assert!(payload.summary.contains("npm test"));
        assert!(payload.summary.contains("Run the test suite"));
        assert!(payload.arguments.contains("npm test"));
        assert!(payload.id.starts_with("confirm_"));

        let payload = confirm_payload(&request("Write", json!({ "file_path": "C:/a.txt" })));
        assert_eq!(payload.path.as_deref(), Some("C:/a.txt"));
        assert_eq!(payload.summary, "C:/a.txt");
        assert!(payload.command.is_none());
    }

    #[test]
    fn serializes_the_decision_contract_claude_expects() {
        assert_eq!(
            PermissionDecision::Allow.to_json().to_string(),
            "{\"behavior\":\"allow\"}"
        );
        let denied = PermissionDecision::Deny("不".into()).to_json();
        assert_eq!(denied["behavior"], "deny");
        assert_eq!(denied["message"], "不");
    }

    #[tokio::test]
    async fn answers_the_mcp_handshake_and_forwards_permission_calls() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let answerer = Arc::new(Recorder {
            decision: PermissionDecision::Allow,
            seen: seen.clone(),
        });
        let bridge = serve(answerer).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let init = client
            .post(bridge.url())
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05" } }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "ai-desktop-pet");

        let tools = client
            .post(bridge.url())
            .json(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(tools["result"]["tools"][0]["name"], TOOL_NAME);

        let call = client
            .post(bridge.url())
            .json(&json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": TOOL_NAME, "arguments": { "tool_name": "Bash", "input": { "command": "echo hi" } } }
            }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(call["result"]["content"][0]["type"], "text");
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("allow"));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0].tool_name, "Bash");

        let unknown = client
            .post(bridge.url())
            .json(&json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);

        let wrong = client
            .post(format!("http://127.0.0.1:{}/mcp/wrong", bridge.port))
            .json(&json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 404);

        let config = bridge.mcp_config_json();
        assert!(config.contains("mcpServers"));
        assert!(config.contains(&bridge.port.to_string()));
    }

    // ── 实机验证：真 claude CLI + 假模型 + 真桥 ─────────────────────────────
    //
    // cargo test --lib mcp_permission::tests::claude_cli_asks_the_pet -- --ignored --nocapture

    fn find_on_path(names: &[&str]) -> Option<std::path::PathBuf> {
        let mut directories: Vec<std::path::PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        if let Some(app_data) = dirs::data_dir() {
            directories.push(app_data.join("npm"));
        }
        for directory in directories.iter().filter(|path| path.is_absolute()) {
            for name in names {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        None
    }

    #[tokio::test]
    #[ignore = "spawns the local claude CLI plus a mock model server"]
    async fn claude_cli_asks_the_pet_before_running_a_tool() {
        let Some(node) = find_on_path(&["node.exe", "node"]) else {
            eprintln!("跳过：本机没有 node");
            return;
        };
        let Some(claude) = find_on_path(&["claude.cmd", "claude.exe", "claude"]) else {
            eprintln!("跳过：本机没有 claude CLI");
            return;
        };
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("claude-mock-provider.cjs");
        assert!(script.is_file(), "缺少假模型脚本 {}", script.display());

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".cli-selftest")
            .join(uuid::Uuid::new_v4().to_string());
        let config_dir = root.join("claude-config");
        std::fs::create_dir_all(&config_dir).unwrap();

        // 1) 假模型
        let port_file = root.join("port.txt");
        let mut mock = tokio::process::Command::new(&node)
            .arg(&script)
            .arg("--port-file")
            .arg(&port_file)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("start mock provider");
        let mut port = None;
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&port_file) {
                if let Ok(value) = text.trim().parse::<u16>() {
                    port = Some(value);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let port = port.expect("mock provider did not report a port");

        // 2) 桥：记录请求并允许
        let seen = Arc::new(Mutex::new(Vec::new()));
        let answerer = Arc::new(Recorder {
            decision: PermissionDecision::Allow,
            seen: seen.clone(),
        });
        let bridge = serve(answerer).await.unwrap();
        let config = root.join("mcp.json");
        std::fs::write(&config, bridge.mcp_config_json()).unwrap();

        // 3) claude：与 ClaudeAdapter 相同的参数，只把模型指到假服务
        let marker = format!("PET-MARKER-CLAUDE-{}", uuid::Uuid::new_v4());
        let mut child = tokio::process::Command::new(&claude)
            .arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--append-system-prompt")
            .arg("这是桌宠接入测试，回答简短。")
            .arg("--max-budget-usd")
            .arg("0.5")
            .arg("--mcp-config")
            .arg(&config)
            .arg("--permission-prompt-tool")
            .arg(PROMPT_TOOL)
            .arg("--permission-prompts")
            .arg("host")
            // 测试里固定成 manual，保证任何工具调用都会询问；桌宠本身不传这个参数，
            // 用户的权限模式（新版本默认 auto，只对危险操作询问）保持不变。
            .arg("--permission-mode")
            .arg("manual")
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("ANTHROPIC_AUTH_TOKEN", "pet-mock-token")
            .env("CLAUDE_CONFIG_DIR", &config_dir)
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("start claude CLI");
        {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child.stdin.take().unwrap();
            let prompt = format!("{marker} 请执行 echo pet-claude-check 并告诉我结果。");
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take();
        let reader = tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });
        let errors = tokio::spawn(async move {
            let mut text = String::new();
            if let Some(mut stderr) = stderr {
                use tokio::io::AsyncReadExt;
                let _ = stderr.read_to_string(&mut text).await;
            }
            text
        });

        let collected = tokio::time::timeout(Duration::from_secs(180), reader)
            .await
            .expect("claude did not finish in time")
            .unwrap();
        let status = child.wait().await;
        let errors = errors.await.unwrap_or_default();
        println!("claude 退出状态：{status:?}");
        println!(
            "claude stdout(前 1200 字)：{}",
            collected.chars().take(1200).collect::<String>()
        );
        println!(
            "claude stderr(前 600 字)：{}",
            errors.chars().take(600).collect::<String>()
        );

        let calls = seen.lock().unwrap().clone();
        println!(
            "桥收到的授权请求：{:?}",
            calls
                .iter()
                .map(|call| (
                    call.tool_name.clone(),
                    call.input["command"].as_str().unwrap_or("").to_string()
                ))
                .collect::<Vec<_>>()
        );
        assert!(!calls.is_empty(), "claude 必须把权限提示交给桌宠的 MCP 工具");
        assert!(
            matches!(calls[0].tool_name.as_str(), "Bash" | "PowerShell" | "shell"),
            "应当是 shell 工具，实际 {}",
            calls[0].tool_name
        );
        assert!(
            calls[0].input["command"]
                .as_str()
                .unwrap_or("")
                .contains("pet-claude-check"),
            "确认弹窗要拿到真实命令：{:?}",
            calls[0].input
        );
        assert!(
            collected.contains(&marker),
            "最终回答里应当有标记：{}",
            collected.chars().take(800).collect::<String>()
        );

        let _ = mock.kill().await;
        let _ = std::fs::remove_dir_all(&root);
    }
}
