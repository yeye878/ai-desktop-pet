use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use image::{GenericImageView, ImageEncoder};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};

const DEBUG_PORT_START: u16 = 9222;
const DEBUG_PORT_END: u16 = 9232;
const BROWSER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_EXTRACT_CHARS: usize = 24_000;
const MAX_EXTRACT_LINKS: usize = 60;

struct ManagedBrowser {
    child: tokio::process::Child,
    port: u16,
}

static MANAGED_BROWSER: Mutex<Option<ManagedBrowser>> = Mutex::new(None);
static CDP_MSG_ID: AtomicU64 = AtomicU64::new(1);

fn managed_browser_state() -> &'static Mutex<Option<ManagedBrowser>> {
    &MANAGED_BROWSER
}

fn profile_dir() -> Result<PathBuf, String> {
    let Some(mut dir) = dirs::data_dir() else {
        return Err("Unable to resolve app data directory".to_string());
    };
    dir.push("ai-desktop-pet");
    dir.push("browser-profile");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create browser profile dir: {e}"))?;
    Ok(dir)
}

fn find_browser_executable(preferred: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut push = |dir: Option<std::ffi::OsString>, names: &[&str]| {
            if let Some(dir) = dir {
                for name in names {
                    candidates.push(PathBuf::from(&dir).join(name));
                }
            }
        };
        let prefer_edge = matches!(preferred, "default" | "edge" | "msedge");
        let prefer_chrome = matches!(preferred, "chrome" | "google_chrome");
        let edge_names = [
            r"Microsoft\Edge\Application\msedge.exe",
            r"Microsoft\Edge\Application\msedge.exe",
        ];
        let chrome_names = [
            r"Google\Chrome\Application\chrome.exe",
            r"Google\Chrome\Application\chrome.exe",
        ];
        if prefer_edge {
            push(std::env::var_os("ProgramFiles(x86)"), &edge_names[..1]);
            push(std::env::var_os("ProgramFiles"), &edge_names[1..]);
            push(std::env::var_os("ProgramFiles(x86)"), &chrome_names[..1]);
            push(std::env::var_os("ProgramFiles"), &chrome_names[1..]);
            push(std::env::var_os("LocalAppData"), &chrome_names[..1]);
        } else if prefer_chrome {
            push(std::env::var_os("ProgramFiles(x86)"), &chrome_names[..1]);
            push(std::env::var_os("ProgramFiles"), &chrome_names[1..]);
            push(std::env::var_os("LocalAppData"), &chrome_names[..1]);
            push(std::env::var_os("ProgramFiles(x86)"), &edge_names[..1]);
            push(std::env::var_os("ProgramFiles"), &edge_names[1..]);
        } else {
            push(std::env::var_os("ProgramFiles(x86)"), &edge_names[..1]);
            push(std::env::var_os("ProgramFiles"), &edge_names[1..]);
            push(std::env::var_os("ProgramFiles(x86)"), &chrome_names[..1]);
            push(std::env::var_os("ProgramFiles"), &chrome_names[1..]);
            push(std::env::var_os("LocalAppData"), &chrome_names[..1]);
        }
        candidates
            .into_iter()
            .find(|path| path.is_file())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = preferred;
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
            "/usr/bin/microsoft-edge-stable",
            "/usr/bin/msedge",
        ];
        candidates
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
    }
}

async fn tcp_port_free(port: u16) -> bool {
    use tokio::net::TcpStream;
    tokio::time::timeout(Duration::from_millis(400), TcpStream::connect(("127.0.0.1", port)))
        .await
        .is_err()
}

async fn spawn_browser(preferred: &str) -> Result<(tokio::process::Child, u16), String> {
    let exe = find_browser_executable(preferred).ok_or_else(|| {
        "未找到 Chrome/Edge 浏览器可执行文件。请安装 Chrome 或 Edge 后重试。".to_string()
    })?;

    let profile = profile_dir()?;
    let mut last_error = "No debug port available".to_string();
    for port in DEBUG_PORT_START..=DEBUG_PORT_END {
        if !tcp_port_free(port).await {
            continue;
        }
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-allow-origins=*")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--window-size=1280,900")
            .arg("--no-pings")
            .arg("--disable-component-update")
            .arg("--disable-features=msEdgeFirstRunExperience")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        match cmd.spawn() {
            Ok(child) => return Ok((child, port)),
            Err(e) => {
                last_error = format!("Failed to launch browser on port {port}: {e}");
                continue;
            }
        }
    }
    Err(format!(
        "{last_error} (debug ports {DEBUG_PORT_START}-{DEBUG_PORT_END} 均不可用)"
    ))
}

async fn debug_http_json(path: &str, port: u16) -> Result<Value, String> {
    let url = format!("http://127.0.0.1:{port}{path}");
    let response = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Failed to reach browser debug endpoint: {e}"))?;
    response
        .json::<Value>()
        .await
        .map_err(|e| format!("Invalid browser debug response: {e}"))
}

async fn ensure_browser_running(preferred: &str) -> Result<u16, String> {
    {
        let mut state = managed_browser_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = state.as_mut() {
            match existing.child.try_wait() {
                Ok(None) => return Ok(existing.port),
                _ => *state = None,
            }
        }
    }

    let (mut child, port) = spawn_browser(preferred).await?;
    let deadline = tokio::time::Instant::now() + BROWSER_LAUNCH_TIMEOUT;
    let mut attempts: Vec<String> = Vec::new();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "浏览器进程启动后立即退出 (exit {status}). 请检查是否已安装 Chrome/Edge。"
            ));
        }
        match debug_http_json("/json/version", port).await {
            Ok(_) => {
                let mut state = managed_browser_state()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *state = Some(ManagedBrowser { child, port });
                return Ok(port);
            }
            Err(e) => attempts.push(e),
        }
        if tokio::time::Instant::now() >= deadline {
            let detail = attempts
                .last()
                .cloned()
                .unwrap_or_else(|| "调试端口未响应".to_string());
            return Err(format!("浏览器启动超时: {detail} (端口 {port})"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn pick_page_target<'a>(pages: &'a [Value]) -> Option<&'a Value> {
    let is_page = |item: &Value| item.get("type").and_then(|t| t.as_str()) == Some("page");
    pages
        .iter()
        .filter(|item| is_page(item))
        .max_by_key(|item| {
            let url = item
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if url.is_empty() || url.starts_with("about:") {
                0
            } else {
                1
            }
        })
}

async fn page_ws_url(port: u16) -> Result<String, String> {
    let list = debug_http_json("/json/list", port).await?;
    let pages = list.as_array().cloned().unwrap_or_default();
    if let Some(target) = pick_page_target(&pages) {
        if let Some(ws) = target.get("webSocketDebuggerUrl").and_then(|u| u.as_str()) {
            return Ok(ws.to_string());
        }
    }
    let response = reqwest::Client::new()
        .put(format!("http://127.0.0.1:{port}/json/new?about%3Ablank"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Failed to create browser tab: {e}"))?;
    let created = response
        .json::<Value>()
        .await
        .map_err(|e| format!("Invalid tab creation response: {e}"))?;
    created
        .get("webSocketDebuggerUrl")
        .and_then(|u| u.as_str())
        .map(|u| u.to_string())
        .ok_or_else(|| "Created tab did not expose a debug URL".to_string())
}

async fn cdp_command(port: u16, method: &str, params: Value) -> Result<Value, String> {
    let ws_url = page_ws_url(port).await?;
    let id = CDP_MSG_ID.fetch_add(1, Ordering::Relaxed);
    let message = json!({
        "id": id,
        "method": method,
        "params": params,
    });

    tokio::time::timeout(Duration::from_secs(20), async move {
        let (socket, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("Failed to connect to browser: {e}"))?;
        let (mut sink, mut stream) = socket.split();

        sink.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&message)
                .map_err(|e| format!("Failed to encode CDP message: {e}"))?
                .into(),
        ))
        .await
        .map_err(|e| format!("Failed to send CDP message: {e}"))?;

        loop {
            match stream.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                    let value: Value = serde_json::from_str(&text)
                        .map_err(|e| format!("Invalid CDP response: {e}"))?;
                    if value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(error) = value.get("error") {
                            return Err(format!(
                                "CDP {} failed: {}",
                                method,
                                error.get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("unknown error")
                            ));
                        }
                        return Ok(
                            value.get("result").cloned().unwrap_or_else(|| json!({}))
                        );
                    }
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload))) => {
                    let _ = sink
                        .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                        .await;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(format!("Browser websocket error: {e}")),
                None => return Err("Browser websocket closed unexpectedly".to_string()),
            }
        }
    })
    .await
    .map_err(|_| format!("CDP {method} timed out"))?
}

async fn evaluate(port: u16, expression: &str) -> Result<Value, String> {
    let result = cdp_command(
        port,
        "Runtime.evaluate",
        json!({
            "expression": expression,
            "returnByValue": true,
        }),
    )
    .await?;
    if let Some(details) = result.get("exceptionDetails") {
        let text = details
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(|d| d.as_str())
            .unwrap_or("unknown evaluation error");
        return Err(format!("页面脚本执行失败: {text}"));
    }
    result
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .ok_or_else(|| "页面脚本没有返回值".to_string())
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

async fn wait_for_ready_state(port: u16) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + NAVIGATION_TIMEOUT;
    let mut last_state = "unknown".to_string();
    while tokio::time::Instant::now() < deadline {
        if let Ok(value) = evaluate(port, "document.readyState").await {
            if let Some(state) = value.as_str() {
                last_state = state.to_string();
                if state == "complete" {
                    return Ok("complete".to_string());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(last_state)
}

async fn navigate_and_wait(port: u16, url: &str) -> Result<Value, String> {
    let navigation = cdp_command(
        port,
        "Page.navigate",
        json!({ "url": url, "transitionType": "typed" }),
    )
    .await?;
    if let Some(error_text) = navigation
        .get("errorText")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty() && *t != "net::ERR_ABORTED")
    {
        return Err(format!("页面加载失败: {error_text}"));
    }
    let state = wait_for_ready_state(port).await?;
    let page = page_state(port).await.unwrap_or_default();
    Ok(json!({
        "ok": true,
        "kind": "browser",
        "action": "navigate",
        "url": url,
        "load_state": state,
        "title": page.get("title").cloned().unwrap_or(Value::Null),
        "summary": format!("已导航到 {url} (load_state: {state})")
    }))
}

async fn page_state(port: u16) -> Result<Value, String> {
    let value = evaluate(
        port,
        "(() => ({title: document.title, url: location.href, ready: document.readyState}))()",
    )
    .await?;
    Ok(value)
}

pub(crate) async fn browser_open(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let browser = args
        .get("browser")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .trim()
        .to_ascii_lowercase();
    if !url.is_empty() {
        super::validate_http_url(&url)?;
    }

    let port = ensure_browser_running(&browser).await?;
    if url.is_empty() {
        let page = page_state(port).await.unwrap_or_default();
        return Ok(json!({
            "ok": true,
            "kind": "browser",
            "action": "open",
            "port": port,
            "url": page.get("url").cloned().unwrap_or(Value::Null),
            "title": page.get("title").cloned().unwrap_or(Value::Null),
            "summary": "AI 托管浏览器已启动 (独立 Chrome/Edge 实例)".to_string()
        })
        .to_string());
    }

    let result = navigate_and_wait(port, &url).await?;
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

pub(crate) async fn browser_navigate(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "缺少 'url' 参数".to_string())?;
    super::validate_http_url(url)?;

    let port = ensure_browser_running("default").await?;
    let result = navigate_and_wait(port, url).await?;
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

pub(crate) async fn browser_snapshot(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let max_width = args
        .get("max_width")
        .or_else(|| args.get("maxWidth"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1024)
        .clamp(320, 4096);

    let port = ensure_browser_running("default").await?;
    let viewport = evaluate(
        port,
        "(() => ({w: window.innerWidth, h: window.innerHeight, dpr: window.devicePixelRatio}))()",
    )
    .await?;
    let viewport_w = viewport.get("w").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
    let viewport_h = viewport.get("h").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
    if viewport_w == 0 || viewport_h == 0 {
        return Err("无法读取浏览器视口尺寸".to_string());
    }

    let captured = cdp_command(
        port,
        "Page.captureScreenshot",
        json!({
            "format": "jpeg",
            "quality": 80,
            "fromSurface": true,
            "captureBeyondViewport": false
        }),
    )
    .await?;
    let data = captured
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "浏览器截图返回为空".to_string())?;
    let bytes = STANDARD
        .decode(data)
        .map_err(|e| format!("浏览器截图解码失败: {e}"))?;

    let (image_w, image_h, jpeg_bytes, scaled) = if max_width > 0 && (bytes.len() > 0) {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("浏览器截图解析失败: {e}"))?
            .to_rgb8();
        let (w, h) = img.dimensions();
        if w as i64 > max_width {
            let new_w = max_width as u32;
            let new_h = (((h as f64) * (new_w as f64) / (w as f64)).round() as u32).max(1);
            let resized =
                image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Lanczos3);
            let mut buf: Vec<u8> = Vec::new();
            {
                let mut cursor = std::io::Cursor::new(&mut buf);
                let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 80)
                    .write_image(
                        &resized,
                        new_w,
                        new_h,
                        image::ExtendedColorType::Rgb8,
                    );
            }
            (new_w, new_h, buf, true)
        } else {
            let mut buf: Vec<u8> = Vec::new();
            {
                let mut cursor = std::io::Cursor::new(&mut buf);
                let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 80)
                    .write_image(&img, w, h, image::ExtendedColorType::Rgb8);
            }
            (w, h, buf, false)
        }
    } else {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("浏览器截图解析失败: {e}"))?;
        let (w, h) = img.dimensions();
        (w, h, bytes, false)
    };

    let scale_x = viewport_w as f64 / image_w.max(1) as f64;
    let scale_y = viewport_h as f64 / image_h.max(1) as f64;
    let base64 = STANDARD.encode(&jpeg_bytes);
    let page = page_state(port).await.unwrap_or_default();
    let title = page.get("title").and_then(|t| t.as_str()).unwrap_or("");

    let result = json!({
        "ok": true,
        "kind": "browser_snapshot",
        "format": "jpeg",
        "quality": 80,
        "display": 0,
        "width": viewport_w,
        "height": viewport_h,
        "screen_x": 0,
        "screen_y": 0,
        "screen_width": viewport_w,
        "screen_height": viewport_h,
        "image_width": image_w,
        "image_height": image_h,
        "scaled": scaled,
        "scale_x": scale_x,
        "scale_y": scale_y,
        "coordinate_space": "page_viewport",
        "image_coordinate_space": "image",
        "page_url": page.get("url").cloned().unwrap_or(Value::Null),
        "page_title": json!(title),
        "image_url": format!("data:image/jpeg;base64,{}", base64),
        "summary": format!(
            "浏览器视口截图 {}x{} (图像 {}x{}，标题: {})",
            viewport_w, viewport_h, image_w, image_h, title
        ),
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

pub(crate) async fn browser_extract(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let include_links = args
        .get("include_links")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|v| (v as usize).clamp(500, MAX_EXTRACT_CHARS))
        .unwrap_or(MAX_EXTRACT_CHARS);

    let port = ensure_browser_running("default").await?;
    let expression = r#"(() => {
      const text = (document.body && document.body.innerText || '').replace(/[ \t]+/g, ' ');
      const links = Array.from(document.querySelectorAll('a[href]'))
        .map(a => ({ text: (a.textContent || '').trim(), href: a.href }))
        .filter(l => l.text && /^https?:/.test(l.href));
      return { title: document.title, url: location.href, text: text.slice(0, 40000), links: links.slice(0, 100) };
    })()"#;
    let value = evaluate(port, expression).await?;

    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let url = value
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut text = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let truncated = text.chars().count() > max_chars;
    if truncated {
        let safe_end = text.floor_char_boundary(max_chars);
        text = format!("{}... [内容已截断，共 {} 字符]", &text[..safe_end], max_chars);
    }

    let links = if include_links {
        value
            .get("links")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(MAX_EXTRACT_LINKS)
            .map(|link| {
                let text = link
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>();
                let href = link
                    .get("href")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                json!({ "text": text, "href": href })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let result = json!({
        "ok": true,
        "kind": "browser_extract",
        "title": title,
        "url": url,
        "text": text,
        "text_chars": text.chars().count(),
        "links": links,
        "summary": format!("已提取页面文本 ({} 字符)，标题: {}", text.chars().count(), title)
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

async fn element_center(port: u16, selector: &str) -> Result<(f64, f64, String), String> {
    let expression = format!(
        "(() => {{ const el = document.querySelector({}); if (!el) return null; const r = el.getBoundingClientRect(); const text = (el.innerText || el.textContent || '').trim().slice(0, 80); return {{ x: r.x + r.width / 2, y: r.y + r.height / 2, tag: el.tagName, text, visible: r.width > 0 && r.height > 0 }}; }})()",
        js_string(selector)
    );
    let value = evaluate(port, &expression).await?;
    if value.is_null() {
        return Err(format!("未找到匹配选择器的元素: {selector}"));
    }
    let visible = value.get("visible").and_then(|v| v.as_bool()).unwrap_or(false);
    let x = value
        .get("x")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "无法计算元素坐标".to_string())?;
    let y = value
        .get("y")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "无法计算元素坐标".to_string())?;
    if !visible {
        return Err(format!("元素不可见或尺寸为 0: {selector}"));
    }
    let label = value
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((x, y, label))
}

async fn dispatch_click(port: u16, x: f64, y: f64) -> Result<(), String> {
    for event_type in ["mousePressed", "mouseReleased"] {
        cdp_command(
            port,
            "Input.dispatchMouseEvent",
            json!({
                "type": event_type,
                "x": x.round(),
                "y": y.round(),
                "button": "left",
                "clickCount": 1
            }),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn browser_click(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let x = args.get("x").and_then(|v| v.as_f64());
    let y = args.get("y").and_then(|v| v.as_f64());

    let port = ensure_browser_running("default").await?;
    let (click_x, click_y, label) = match selector {
        Some(selector) => element_center(port, selector).await?,
        None => {
            let (x, y) = match (x, y) {
                (Some(x), Some(y)) => (x, y),
                _ => return Err("browser_click 需要 selector 或 x/y 坐标".to_string()),
            };
            (x, y, String::new())
        }
    };
    dispatch_click(port, click_x, click_y).await?;
    tokio::time::sleep(Duration::from_millis(120)).await;

    let result = json!({
        "ok": true,
        "kind": "browser",
        "action": "click",
        "x": click_x.round() as i64,
        "y": click_y.round() as i64,
        "selector": selector.unwrap_or(""),
        "label": label,
        "summary": if label.is_empty() {
            format!("已点击浏览器坐标 ({}, {})", click_x.round() as i64, click_y.round() as i64)
        } else {
            format!("已点击元素「{}」({}, {})", label, click_x.round() as i64, click_y.round() as i64)
        }
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

pub(crate) async fn browser_type(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "缺少 'text' 参数".to_string())?;
    super::validate_keyboard_text(text)?;
    if text.chars().count() > 4000 {
        return Err("输入文本过长；单次最多 4000 字符。".to_string());
    }
    let selector = args
        .get("selector")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let port = ensure_browser_running("default").await?;
    if let Some(selector) = selector {
        let (x, y, _) = element_center(port, selector).await?;
        dispatch_click(port, x, y).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    cdp_command(port, "Input.insertText", json!({ "text": text })).await?;
    let result = json!({
        "ok": true,
        "kind": "browser",
        "action": "type",
        "chars": text.chars().count(),
        "selector": selector.unwrap_or(""),
        "summary": format!("已在浏览器输入 {} 个字符", text.chars().count())
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

fn key_descriptor(key: &str) -> Option<(String, String, u32, Option<String>)> {
    let key = key.trim().to_ascii_lowercase();
    let (key_name, code, vk): (String, String, u32) = match key.as_str() {
        "enter" | "return" => ("Enter".into(), "Enter".into(), 13),
        "tab" => ("Tab".into(), "Tab".into(), 9),
        "esc" | "escape" => ("Escape".into(), "Escape".into(), 27),
        "backspace" => ("Backspace".into(), "Backspace".into(), 8),
        "delete" | "del" => ("Delete".into(), "Delete".into(), 46),
        "space" => (" ".into(), "Space".into(), 32),
        "up" => ("ArrowUp".into(), "ArrowUp".into(), 38),
        "down" => ("ArrowDown".into(), "ArrowDown".into(), 40),
        "left" => ("ArrowLeft".into(), "ArrowLeft".into(), 37),
        "right" => ("ArrowRight".into(), "ArrowRight".into(), 39),
        "home" => ("Home".into(), "Home".into(), 36),
        "end" => ("End".into(), "End".into(), 35),
        "pageup" | "pgup" => ("PageUp".into(), "PageUp".into(), 33),
        "pagedown" | "pgdn" => ("PageDown".into(), "PageDown".into(), 34),
        "insert" | "ins" => ("Insert".into(), "Insert".into(), 45),
        _ => {
            let bytes = key.as_bytes();
            if key.len() == 1 && bytes[0].is_ascii_alphabetic() {
                let upper = bytes[0].to_ascii_uppercase() as char;
                (
                    upper.to_string(),
                    format!("Key{upper}"),
                    bytes[0].to_ascii_uppercase() as u32,
                )
            } else if key.len() == 1 && bytes[0].is_ascii_digit() {
                let ch = key.chars().next().unwrap();
                (ch.to_string(), format!("Digit{ch}"), bytes[0] as u32)
            } else if key.starts_with('f')
                && key[1..].parse::<u16>().is_ok_and(|n| (1..=24).contains(&n))
            {
                let number: u16 = key[1..].parse().unwrap();
                (
                    format!("F{number}"),
                    format!("F{number}"),
                    0x70 + number as u32 - 1,
                )
            } else {
                return None;
            }
        }
    };
    let text = if key_name.chars().count() == 1 && !key_name.chars().all(|c| c.is_ascii_control()) {
        Some(key_name.clone())
    } else {
        None
    };
    Some((key_name, code, vk, text))
}

pub(crate) async fn browser_press(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "缺少 'key' 参数".to_string())?;
    let (key_name, code, vk, text) = key_descriptor(key)
        .ok_or_else(|| format!("不支持的按键: {key}"))?;

    let port = ensure_browser_running("default").await?;
    let mut down_params = json!({
        "type": "keyDown",
        "key": key_name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk
    });
    if let Some(text) = &text {
        down_params["text"] = json!(text);
        down_params["unmodifiedText"] = json!(text);
    }
    cdp_command(port, "Input.dispatchKeyEvent", down_params).await?;
    let up_params = json!({
        "type": "keyUp",
        "key": key_name,
        "code": code,
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk
    });
    cdp_command(port, "Input.dispatchKeyEvent", up_params).await?;

    let result = json!({
        "ok": true,
        "kind": "browser",
        "action": "press",
        "key": key.to_string(),
        "summary": format!("已在浏览器按下 {key} 键")
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

pub(crate) async fn browser_scroll(args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("down")
        .trim()
        .to_ascii_lowercase();
    let pixels = args
        .get("pixels")
        .and_then(|v| v.as_i64())
        .unwrap_or(600)
        .clamp(-5000, 5000);

    let js = match direction.as_str() {
        "up" => format!("window.scrollBy(0, -{pixels})"),
        "down" => format!("window.scrollBy(0, {pixels})"),
        "left" => format!("window.scrollBy(-{pixels}, 0)"),
        "right" => format!("window.scrollBy({pixels}, 0)"),
        "top" => "window.scrollTo(0, 0)".to_string(),
        "bottom" => {
            "window.scrollTo(0, Math.max(0, (document.documentElement.scrollHeight || document.body.scrollHeight || 0) - window.innerHeight))".to_string()
        }
        _ => return Err("direction 必须是 up/down/left/right/top/bottom 之一".to_string()),
    };

    let port = ensure_browser_running("default").await?;
    cdp_command(port, "Runtime.evaluate", json!({ "expression": js })).await?;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let position = evaluate(
        port,
        "(() => ({x: window.scrollX, y: window.scrollY}))()",
    )
    .await?;
    let scroll_x = position.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
    let scroll_y = position.get("y").and_then(|v| v.as_i64()).unwrap_or(0);

    let result = json!({
        "ok": true,
        "kind": "browser",
        "action": "scroll",
        "direction": direction,
        "pixels": pixels,
        "scroll_x": scroll_x,
        "scroll_y": scroll_y,
        "summary": format!("已滚动页面 ({} {}px，当前位置 {},{})", direction, pixels, scroll_x, scroll_y)
    });
    Ok(serde_json::to_string(&result).unwrap_or_default())
}

async fn history_navigate(port: u16, back: bool) -> Result<String, String> {
    let expression = if back {
        "window.history.length > 1 ? (history.back(), true) : false"
    } else {
        "history.forward()"
    };
    cdp_command(
        port,
        "Runtime.evaluate",
        json!({ "expression": expression }),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let page = page_state(port).await.unwrap_or_default();
    let title = page.get("title").and_then(|t| t.as_str()).unwrap_or("");
    let url = page
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    Ok(json!({
        "ok": true,
        "kind": "browser",
        "action": if back { "back" } else { "forward" },
        "url": url,
        "title": title,
        "summary": if back { format!("已返回上一页: {url}") } else { format!("已前进到下一页: {url}") }
    })
    .to_string())
}

pub(crate) async fn browser_back(_args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let port = ensure_browser_running("default").await?;
    history_navigate(port, true).await
}

pub(crate) async fn browser_forward(_args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let port = ensure_browser_running("default").await?;
    history_navigate(port, false).await
}

pub(crate) async fn browser_status(_args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let port = match ensure_browser_running("default").await {
        Ok(port) => port,
        Err(e) => {
            return Ok(json!({
                "ok": true,
                "kind": "browser_status",
                "running": false,
                "summary": format!("浏览器未运行: {e}")
            })
            .to_string())
        }
    };
    let page = page_state(port).await.unwrap_or_default();
    let title = page.get("title").and_then(|t| t.as_str()).unwrap_or("");
    let url = page
        .get("url")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    Ok(json!({
        "ok": true,
        "kind": "browser_status",
        "running": true,
        "port": port,
        "url": url,
        "title": title,
        "summary": format!("浏览器运行中，当前页面: {title} ({url})")
    })
    .to_string())
}

pub(crate) async fn browser_close(_args: &Value) -> Result<String, String> {
    super::ensure_enabled()?;
    let managed = {
        let mut state = managed_browser_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.take() {
            Some(managed) => managed,
            None => {
                return Ok(json!({
                    "ok": true,
                    "kind": "browser",
                    "action": "close",
                    "summary": "AI 托管浏览器未在运行".to_string()
                })
                .to_string())
            }
        }
    };

    let port = managed.port;
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        async {
            let _ = cdp_command(port, "Browser.close", json!({})).await;
        },
    )
    .await;

    let mut child = managed.child;
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;

    Ok(json!({
        "ok": true,
        "kind": "browser",
        "action": "close",
        "summary": "AI 托管浏览器已关闭".to_string()
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_press_keys() {
        assert_eq!(
            key_descriptor("Enter").unwrap(),
            ("Enter".to_string(), "Enter".to_string(), 13, None)
        );
        assert_eq!(
            key_descriptor("ctrl+l").map(|_| true).unwrap_or(false),
            false
        );
        assert_eq!(
            key_descriptor("a").unwrap(),
            ("A".to_string(), "KeyA".to_string(), 65, Some("A".to_string()))
        );
        assert_eq!(
            key_descriptor("f5").unwrap(),
            ("F5".to_string(), "F5".to_string(), 116, None)
        );
        assert_eq!(
            key_descriptor("arrowup").map(|_| true).unwrap_or(false),
            false
        );
    }

    #[test]
    fn rejects_unsupported_keys() {
        assert!(key_descriptor("win").is_none());
        assert!(key_descriptor("f99").is_none());
        assert!(key_descriptor("").is_none());
    }

    #[test]
    fn prefers_real_pages_over_blank_tabs() {
        let pages = vec![
            json!({ "type": "page", "url": "about:blank" }),
            json!({ "type": "page", "url": "https://example.com" }),
        ];
        let picked = pick_page_target(&pages).unwrap();
        assert_eq!(picked["url"], "https://example.com");
    }

    #[test]
    fn jsescape_embeds_selector_safely() {
        assert_eq!(js_string("a[href='/x']"), "\"a[href='/x']\"");
        assert_eq!(js_string("a\"b"), "\"a\\\"b\"");
    }

    #[tokio::test]
    #[ignore]
    async fn smoke_test_launches_browser_and_navigates() {
        let port = ensure_browser_running("default").await.expect("browser launch");
        let url = "https://example.com";
        let result = navigate_and_wait(port, url).await.expect("navigate");
        assert_eq!(result["url"], url);

        let page = page_state(port).await.unwrap();
        assert!(page["url"].as_str().unwrap_or("").contains("example.com"));

        let extract = evaluate(
            port,
            "(() => ({ title: document.title, has_text: (document.body.innerText || '').length > 0 }))()",
        )
        .await
        .unwrap();
        assert!(extract["has_text"].as_bool().unwrap_or(false));

        let snap = cdp_command(
            port,
            "Page.captureScreenshot",
            json!({ "format": "jpeg", "quality": 80, "fromSurface": true }),
        )
        .await
        .unwrap();
        assert!(snap["data"].as_str().unwrap_or("").len() > 100);

        let _ = browser_close(&json!({})).await;
    }

    #[tokio::test]
    #[ignore]
    async fn smoke_test_click_type_press_flow() {
        let port = ensure_browser_running("default").await.expect("browser launch");
        let html = "<html><body><input id='q' placeholder='search'><button id='go' onclick=\"document.getElementById('q').value += '_clicked'\">Go</button></body></html>";
        let encoded = urlencoding::encode(html);
        let data_url = format!("data:text/html,{encoded}");
        navigate_and_wait(port, &data_url).await.expect("navigate data url");

        let (x, y, _) = element_center(port, "#q").await.expect("find input");
        dispatch_click(port, x, y).await.expect("focus input");
        cdp_command(port, "Input.insertText", json!({ "text": "hello" }))
            .await
            .expect("insert text");

        let value = evaluate(port, "document.getElementById('q').value").await.unwrap();
        assert_eq!(value.as_str().unwrap_or(""), "hello");

        let (bx, by, _) = element_center(port, "#go").await.expect("find button");
        dispatch_click(port, bx, by).await.expect("click button");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let value = evaluate(port, "document.getElementById('q').value").await.unwrap();
        assert_eq!(value.as_str().unwrap_or(""), "hello_clicked");

        cdp_command(
            port,
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyDown",
                "key": "Escape",
                "code": "Escape",
                "windowsVirtualKeyCode": 27,
                "nativeVirtualKeyCode": 27
            }),
        )
        .await
        .expect("press escape down");
        cdp_command(
            port,
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": "Escape",
                "code": "Escape",
                "windowsVirtualKeyCode": 27,
                "nativeVirtualKeyCode": 27
            }),
        )
        .await
        .expect("press escape up");

        let _ = browser_close(&json!({})).await;
    }
}
