use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

const COMPUTER_USE_ENABLED_KEY: &str = "computer_use_enabled";

#[derive(Clone, Debug)]
pub(crate) struct ForegroundWindowSnapshot {
    hwnd: i64,
}

pub(crate) async fn screenshot(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let display = arg_i64(args, "display").unwrap_or(0).clamp(0, 16);
    let max_width = arg_i64(args, "max_width")
        .or_else(|| arg_i64(args, "maxWidth"))
        .unwrap_or(1024)
        .clamp(320, 4096);

    run_screenshot(display, max_width).await
}

pub(crate) async fn browser_snapshot(args: &Value) -> Result<String, String> {
    screenshot(args).await
}

pub(crate) async fn mouse(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let action = required_str(args, "action")?.to_ascii_lowercase();
    let allowed = [
        "move",
        "click",
        "double_click",
        "right_click",
        "drag",
        "scroll",
    ];
    if !allowed.contains(&action.as_str()) {
        return Err(format!("Unsupported mouse action: {action}"));
    }

    let mut envs = vec![("CU_ACTION".to_string(), action.clone())];
    for key in ["x", "y", "to_x", "to_y", "dx", "dy", "amount"] {
        if let Some(value) = arg_i64(args, key) {
            envs.push((
                format!("CU_{}", key.to_ascii_uppercase()),
                value.to_string(),
            ));
        }
    }
    envs.push((
        "CU_BUTTON".to_string(),
        args.get("button")
            .and_then(|value| value.as_str())
            .unwrap_or("left")
            .to_ascii_lowercase(),
    ));

    run_mouse(&envs).await
}

pub(crate) async fn keyboard(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let action = required_str(args, "action")?.to_ascii_lowercase();
    match action.as_str() {
        "type" => {
            let text = required_str(args, "text")?;
            validate_keyboard_text(text)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
            run_keyboard_type(&encoded).await
        }
        "press" => {
            let key = required_str(args, "key")?;
            let keys = vec![key.to_string()];
            let _ = sendkeys_for_keys(&keys)?;
            run_keyboard_keys(&keys, "press").await
        }
        "hotkey" => {
            let keys = args
                .get("keys")
                .and_then(|value| value.as_array())
                .ok_or_else(|| "Missing keys array for keyboard hotkey".to_string())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(|item| item.to_string())
                        .ok_or_else(|| "Keyboard hotkey keys must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let _ = sendkeys_for_keys(&keys)?;
            run_keyboard_keys(&keys, "hotkey").await
        }
        _ => Err(format!("Unsupported keyboard action: {action}")),
    }
}

pub(crate) async fn wait(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let ms = arg_i64(args, "ms").unwrap_or(500).clamp(50, 10_000) as u64;
    tokio::time::sleep(Duration::from_millis(ms)).await;
    Ok(json!({
        "ok": true,
        "kind": "wait",
        "ms": ms,
        "summary": format!("Waited {ms} ms")
    })
    .to_string())
}

pub(crate) async fn browser_open(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let url = args
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let browser = args
        .get("browser")
        .and_then(|value| value.as_str())
        .unwrap_or("default")
        .to_ascii_lowercase();

    if !url.trim().is_empty() {
        validate_http_url(url)?;
    }

    let target = match browser.as_str() {
        "chrome" | "google_chrome" => Some("chrome"),
        "edge" | "msedge" => Some("msedge"),
        "default" | "" => None,
        other => return Err(format!("Unsupported browser: {other}")),
    };
    if target.is_none() && url.trim().is_empty() {
        return Err("Provide a URL when opening the default browser.".to_string());
    }

    run_browser_open(target, url).await
}

pub(crate) async fn browser_navigate(args: &Value) -> Result<String, String> {
    browser_open(args).await
}

pub(crate) async fn window_list(_args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    run_window_list().await
}

pub(crate) async fn window_focus(args: &Value) -> Result<String, String> {
    ensure_enabled()?;
    let title = args
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let pid = arg_i64(args, "pid");
    if title.is_empty() && pid.is_none() {
        return Err("Provide either title or pid for window_focus".to_string());
    }

    let mut envs = Vec::new();
    if !title.is_empty() {
        envs.push(("CU_TITLE".to_string(), title));
    }
    if let Some(pid) = pid {
        envs.push(("CU_PID".to_string(), pid.to_string()));
    }
    run_window_focus(&envs).await
}

pub(crate) fn should_restore_focus_after_confirmation(tool_name: &str) -> bool {
    matches!(tool_name, "computer_mouse" | "computer_keyboard")
}

#[cfg(target_os = "windows")]
pub(crate) async fn capture_foreground_window() -> Option<ForegroundWindowSnapshot> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return None;
    }

    Some(ForegroundWindowSnapshot {
        hwnd: hwnd as isize as i64,
    })
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn capture_foreground_window() -> Option<ForegroundWindowSnapshot> {
    None
}

#[cfg(target_os = "windows")]
pub(crate) async fn restore_foreground_window(snapshot: Option<&ForegroundWindowSnapshot>) {
    use std::time::Duration;
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
        ShowWindowAsync, SW_RESTORE,
    };

    let Some(snapshot) = snapshot else {
        return;
    };
    if snapshot.hwnd == 0 {
        return;
    }

    let hwnd_value = snapshot.hwnd;
    {
        let hwnd = hwnd_value as isize as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() || unsafe { IsWindow(hwnd) } == 0 {
            return;
        }

        unsafe {
            ShowWindowAsync(hwnd, SW_RESTORE);
        }
    }
    tokio::time::sleep(Duration::from_millis(90)).await;

    {
        let current_thread = unsafe { GetCurrentThreadId() };
        let mut target_pid = 0u32;
        let hwnd = hwnd_value as isize as windows_sys::Win32::Foundation::HWND;
        if hwnd.is_null() {
            return;
        }
        let target_thread = unsafe { GetWindowThreadProcessId(hwnd, &mut target_pid) };
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_thread = if foreground.is_null() {
            0
        } else {
            unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) }
        };

        unsafe {
            if target_thread != 0 && target_thread != current_thread {
                AttachThreadInput(current_thread, target_thread, 1);
            }
            if foreground_thread != 0 && foreground_thread != current_thread {
                AttachThreadInput(current_thread, foreground_thread, 1);
            }

            SetForegroundWindow(hwnd);

            if foreground_thread != 0 && foreground_thread != current_thread {
                AttachThreadInput(current_thread, foreground_thread, 0);
            }
            if target_thread != 0 && target_thread != current_thread {
                AttachThreadInput(current_thread, target_thread, 0);
            }
        }
    }

    tokio::time::sleep(Duration::from_millis(120)).await;
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn restore_foreground_window(_snapshot: Option<&ForegroundWindowSnapshot>) {}

fn ensure_enabled() -> Result<(), String> {
    if computer_use_enabled() {
        Ok(())
    } else {
        Err("Computer use is disabled. Enable it in settings before allowing AI to view the screen or control input.".to_string())
    }
}

fn computer_use_enabled() -> bool {
    let Some(mut db_path) = dirs::data_dir() else {
        return false;
    };
    db_path.push("ai-desktop-pet");
    db_path.push("pet.db");

    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        return false;
    };
    let value = conn.query_row(
        "SELECT value FROM pet_settings WHERE key = ?1",
        [COMPUTER_USE_ENABLED_KEY],
        |row| row.get::<_, String>(0),
    );

    matches!(
        value.ok().as_deref().map(str::trim),
        Some("true" | "1" | "yes" | "on")
    )
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing '{key}' parameter"))
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number.round() as i64))
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<i64>().ok())
            })
    })
}

fn validate_http_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(format!(
            "Unsupported URL scheme for browser control: {scheme}"
        )),
    }
}

fn validate_keyboard_text(text: &str) -> Result<(), String> {
    if text.chars().count() > 4000 {
        return Err("Keyboard text is too long; maximum is 4000 characters per call.".to_string());
    }

    let lower = text.to_ascii_lowercase();
    let sensitive_markers = [
        "password",
        "passwd",
        "api_key",
        "apikey",
        "secret",
        "token=",
        "access_token",
        "refresh_token",
        "authorization:",
        "密码",
        "验证码",
        "密钥",
        "令牌",
        "访问令牌",
        "刷新令牌",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return Err(
            "Refusing to type text that looks like a password, token, or secret.".to_string(),
        );
    }

    let compact = text.trim();
    if compact.starts_with("sk-") && compact.len() > 20 {
        return Err("Refusing to type text that looks like an API key.".to_string());
    }

    if (4..=8).contains(&compact.len()) && compact.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("Refusing to type short numeric codes automatically.".to_string());
    }

    Ok(())
}

fn sendkeys_for_keys(keys: &[String]) -> Result<String, String> {
    if keys.is_empty() {
        return Err("Keyboard keys cannot be empty".to_string());
    }

    let mut prefixes = String::new();
    let mut body = String::new();
    for key in keys {
        let normalized = key.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "ctrl" | "control" => prefixes.push('^'),
            "alt" => prefixes.push('%'),
            "shift" => prefixes.push('+'),
            "win" | "meta" | "cmd" => {
                return Err(
                    "Windows/meta key is not supported by the safe hotkey mapper.".to_string(),
                )
            }
            _ => body.push_str(&sendkeys_key(&normalized)?),
        }
    }

    if body.is_empty() {
        return Err("Keyboard hotkey needs at least one non-modifier key".to_string());
    }

    Ok(format!("{prefixes}{body}"))
}

fn sendkeys_key(key: &str) -> Result<String, String> {
    let mapped = match key {
        "enter" | "return" => "{ENTER}".to_string(),
        "tab" => "{TAB}".to_string(),
        "esc" | "escape" => "{ESC}".to_string(),
        "backspace" => "{BACKSPACE}".to_string(),
        "delete" | "del" => "{DELETE}".to_string(),
        "space" => " ".to_string(),
        "up" => "{UP}".to_string(),
        "down" => "{DOWN}".to_string(),
        "left" => "{LEFT}".to_string(),
        "right" => "{RIGHT}".to_string(),
        "home" => "{HOME}".to_string(),
        "end" => "{END}".to_string(),
        "pageup" | "pgup" => "{PGUP}".to_string(),
        "pagedown" | "pgdn" => "{PGDN}".to_string(),
        "insert" | "ins" => "{INSERT}".to_string(),
        key if key.len() == 1 && key.chars().all(|ch| ch.is_ascii_alphanumeric()) => {
            key.to_string()
        }
        key if key.starts_with('f') => {
            let number = key[1..]
                .parse::<u8>()
                .map_err(|_| format!("Unsupported key: {key}"))?;
            if (1..=24).contains(&number) {
                format!("{{F{number}}}")
            } else {
                return Err(format!("Unsupported key: {key}"));
            }
        }
        _ => return Err(format!("Unsupported key: {key}")),
    };
    Ok(mapped)
}

#[cfg(target_os = "windows")]
async fn run_screenshot(display: i64, max_width: i64) -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$screens = [System.Windows.Forms.Screen]::AllScreens
$display = [int]$env:CU_DISPLAY
if ($display -lt 0 -or $display -ge $screens.Length) { $display = 0 }
$maxWidth = [int]$env:CU_MAX_WIDTH
$bounds = $screens[$display].Bounds
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bounds.Size)
$final = $bitmap
$scaled = $false
if ($maxWidth -gt 0 -and $bounds.Width -gt $maxWidth) {
  $newWidth = $maxWidth
  $newHeight = [int][Math]::Round($bounds.Height * ($newWidth / [double]$bounds.Width))
  $resized = New-Object System.Drawing.Bitmap $newWidth, $newHeight
  $resizeGraphics = [System.Drawing.Graphics]::FromImage($resized)
  $resizeGraphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $resizeGraphics.DrawImage($bitmap, 0, 0, $newWidth, $newHeight)
  $resizeGraphics.Dispose()
  $final = $resized
  $scaled = $true
}
$stream = New-Object System.IO.MemoryStream
$jpegCodec = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() |
  Where-Object { $_.MimeType -eq 'image/jpeg' } |
  Select-Object -First 1
$encoderParams = New-Object System.Drawing.Imaging.EncoderParameters 1
$encoderParams.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), ([Int64]85)
$final.Save($stream, $jpegCodec, $encoderParams)
$base64 = [Convert]::ToBase64String($stream.ToArray())
$result = [pscustomobject]@{
  ok = $true
  kind = 'screenshot'
  format = 'jpeg'
  quality = 85
  display = $display
  width = $bounds.Width
  height = $bounds.Height
  image_width = $final.Width
  image_height = $final.Height
  scaled = $scaled
  image_url = "data:image/jpeg;base64,$base64"
  summary = "Screenshot captured from display $display ($($bounds.Width)x$($bounds.Height))"
}
$encoderParams.Dispose()
$stream.Dispose()
if ($final -ne $bitmap) { $final.Dispose() }
$graphics.Dispose()
$bitmap.Dispose()
$result | ConvertTo-Json -Compress -Depth 4
"#;
    run_powershell(
        script,
        &[
            ("CU_DISPLAY".to_string(), display.to_string()),
            ("CU_MAX_WIDTH".to_string(), max_width.to_string()),
        ],
        20,
    )
    .await
}

#[cfg(not(target_os = "windows"))]
async fn run_screenshot(_display: i64, _max_width: i64) -> Result<String, String> {
    Err("Computer use is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_mouse(envs: &[(String, String)]) -> Result<String, String> {
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL,
    };

    let action = env_str(envs, "CU_ACTION").unwrap_or("move").to_string();
    let (cur_x, cur_y) = current_cursor_position()?;
    let x = env_i32(envs, "CU_X").unwrap_or(cur_x);
    let y = env_i32(envs, "CU_Y").unwrap_or(cur_y);

    match action.as_str() {
        "move" => move_cursor_to(x, y)?,
        "click" => {
            move_cursor_to(x, y)?;
            if env_str(envs, "CU_BUTTON") == Some("right") {
                send_mouse_input(MOUSEEVENTF_RIGHTDOWN, 0)?;
                tokio::time::sleep(Duration::from_millis(35)).await;
                send_mouse_input(MOUSEEVENTF_RIGHTUP, 0)?;
            } else {
                send_mouse_input(MOUSEEVENTF_LEFTDOWN, 0)?;
                tokio::time::sleep(Duration::from_millis(35)).await;
                send_mouse_input(MOUSEEVENTF_LEFTUP, 0)?;
            }
        }
        "double_click" => {
            move_cursor_to(x, y)?;
            send_mouse_input(MOUSEEVENTF_LEFTDOWN, 0)?;
            tokio::time::sleep(Duration::from_millis(30)).await;
            send_mouse_input(MOUSEEVENTF_LEFTUP, 0)?;
            tokio::time::sleep(Duration::from_millis(60)).await;
            send_mouse_input(MOUSEEVENTF_LEFTDOWN, 0)?;
            tokio::time::sleep(Duration::from_millis(30)).await;
            send_mouse_input(MOUSEEVENTF_LEFTUP, 0)?;
        }
        "right_click" => {
            move_cursor_to(x, y)?;
            send_mouse_input(MOUSEEVENTF_RIGHTDOWN, 0)?;
            tokio::time::sleep(Duration::from_millis(35)).await;
            send_mouse_input(MOUSEEVENTF_RIGHTUP, 0)?;
        }
        "drag" => {
            let to_x = env_i32(envs, "CU_TO_X").unwrap_or(x + env_i32(envs, "CU_DX").unwrap_or(0));
            let to_y = env_i32(envs, "CU_TO_Y").unwrap_or(y + env_i32(envs, "CU_DY").unwrap_or(0));
            move_cursor_to(x, y)?;
            send_mouse_input(MOUSEEVENTF_LEFTDOWN, 0)?;
            tokio::time::sleep(Duration::from_millis(90)).await;
            move_cursor_to(to_x, to_y)?;
            tokio::time::sleep(Duration::from_millis(90)).await;
            send_mouse_input(MOUSEEVENTF_LEFTUP, 0)?;
        }
        "scroll" => {
            let amount = env_i32(envs, "CU_AMOUNT").unwrap_or(env_i32(envs, "CU_DY").unwrap_or(-3));
            send_mouse_input(MOUSEEVENTF_WHEEL, amount.saturating_mul(120) as u32)?;
        }
        _ => return Err(format!("Unsupported mouse action: {action}")),
    }

    let (final_x, final_y) = current_cursor_position().unwrap_or((x, y));
    Ok(json!({
        "ok": true,
        "kind": "mouse",
        "action": action,
        "x": final_x,
        "y": final_y,
        "summary": format!("Mouse action completed: {action}")
    })
    .to_string())
}

#[cfg(not(target_os = "windows"))]
async fn run_mouse(_envs: &[(String, String)]) -> Result<String, String> {
    Err("Computer use is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_keyboard_type(encoded_text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded_text)
        .map_err(|e| format!("Invalid keyboard text encoding: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("Keyboard text is not UTF-8: {e}"))?;
    type_text_native(&text)?;
    Ok(json!({
        "ok": true,
        "kind": "keyboard",
        "action": "type",
        "chars": text.chars().count(),
        "summary": format!("Typed {} characters", text.chars().count())
    })
    .to_string())
}

#[cfg(not(target_os = "windows"))]
async fn run_keyboard_type(_encoded_text: &str) -> Result<String, String> {
    Err("Computer use is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_keyboard_keys(keys: &[String], action: &str) -> Result<String, String> {
    press_keys_native(keys)?;
    Ok(json!({
        "ok": true,
        "kind": "keyboard",
        "action": action,
        "keys": keys,
        "summary": format!("Keyboard action completed: {action}")
    })
    .to_string())
}

#[cfg(not(target_os = "windows"))]
async fn run_keyboard_keys(_keys: &[String], _action: &str) -> Result<String, String> {
    Err("Computer use is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
fn env_str<'a>(envs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    envs.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

#[cfg(target_os = "windows")]
fn env_i32(envs: &[(String, String)], key: &str) -> Option<i32> {
    env_str(envs, key).and_then(|value| value.trim().parse::<i32>().ok())
}

#[cfg(target_os = "windows")]
fn win32_error(action: &str) -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    format!("{action} failed with Win32 error {code}")
}

#[cfg(target_os = "windows")]
fn current_cursor_position() -> Result<(i32, i32), String> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err(win32_error("GetCursorPos"));
    }
    Ok((point.x, point.y))
}

#[cfg(target_os = "windows")]
fn ensure_point_in_virtual_screen(x: i32, y: i32) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        return Err("Unable to read virtual screen bounds".to_string());
    }
    let right = left.saturating_add(width);
    let bottom = top.saturating_add(height);
    if x < left || x >= right || y < top || y >= bottom {
        return Err(format!(
            "Coordinates outside virtual screen: {x},{y} (bounds {left},{top} {width}x{height})"
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn move_cursor_to(x: i32, y: i32) -> Result<(), String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

    ensure_point_in_virtual_screen(x, y)?;
    if unsafe { SetCursorPos(x, y) } == 0 {
        return Err(win32_error("SetCursorPos"));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn send_win_inputs(
    inputs: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
) -> Result<(), String> {
    use std::mem::size_of;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT};

    for chunk in inputs.chunks(512) {
        let sent = unsafe {
            SendInput(
                chunk.len() as u32,
                chunk.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent != chunk.len() as u32 {
            return Err(win32_error("SendInput"));
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn mouse_input(
    flags: u32,
    mouse_data: u32,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn send_mouse_input(flags: u32, mouse_data: u32) -> Result<(), String> {
    send_win_inputs(&[mouse_input(flags, mouse_data)])
}

#[cfg(target_os = "windows")]
fn keyboard_input(
    vk: u16,
    scan: u16,
    flags: u32,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(target_os = "windows")]
fn type_text_native(text: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    let mut inputs = Vec::with_capacity(text.encode_utf16().count() * 2);
    for unit in text.encode_utf16() {
        inputs.push(keyboard_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(keyboard_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send_win_inputs(&inputs)
}

#[cfg(target_os = "windows")]
fn press_keys_native(keys: &[String]) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

    let mut normalized = Vec::with_capacity(keys.len());
    for key in keys {
        let key = key.trim().to_ascii_lowercase();
        normalized.push((key.clone(), vk_for_key(&key)?));
    }

    let mut inputs = Vec::with_capacity(normalized.len() * 2);
    for (_, vk) in &normalized {
        inputs.push(keyboard_input(*vk, 0, 0));
    }
    for (_, vk) in normalized.iter().rev() {
        inputs.push(keyboard_input(*vk, 0, KEYEVENTF_KEYUP));
    }
    send_win_inputs(&inputs)
}

#[cfg(target_os = "windows")]
fn vk_for_key(key: &str) -> Result<u16, String> {
    let vk = match key {
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "shift" => 0x10,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "space" => 0x20,
        "up" => 0x26,
        "down" => 0x28,
        "left" => 0x25,
        "right" => 0x27,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "insert" | "ins" => 0x2D,
        key if key.len() == 1 && key.chars().all(|ch| ch.is_ascii_alphabetic()) => {
            key.as_bytes()[0].to_ascii_uppercase() as u16
        }
        key if key.len() == 1 && key.chars().all(|ch| ch.is_ascii_digit()) => {
            key.as_bytes()[0] as u16
        }
        key if key.starts_with('f') => {
            let number = key[1..]
                .parse::<u16>()
                .map_err(|_| format!("Unsupported key: {key}"))?;
            if (1..=24).contains(&number) {
                0x70 + number - 1
            } else {
                return Err(format!("Unsupported key: {key}"));
            }
        }
        _ => return Err(format!("Unsupported key: {key}")),
    };
    Ok(vk)
}

#[cfg(target_os = "windows")]
async fn run_browser_open(target: Option<&str>, url: &str) -> Result<String, String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "start", ""]);
    if let Some(target) = target {
        cmd.arg(target);
    }
    if !url.trim().is_empty() {
        cmd.arg(url.trim());
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn()
        .map_err(|e| format!("Failed to open browser: {e}"))?;

    Ok(json!({
        "ok": true,
        "kind": "browser",
        "action": "open",
        "browser": target.unwrap_or("default"),
        "url": url,
        "summary": if url.trim().is_empty() {
            "Browser opened".to_string()
        } else {
            format!("Browser opened at {url}")
        }
    })
    .to_string())
}

#[cfg(not(target_os = "windows"))]
async fn run_browser_open(_target: Option<&str>, _url: &str) -> Result<String, String> {
    Err("Browser computer use is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_window_list() -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = 'Stop'
$items = Get-Process |
  Where-Object { $_.MainWindowHandle -ne 0 -and -not [string]::IsNullOrWhiteSpace($_.MainWindowTitle) } |
  Sort-Object ProcessName, Id |
  Select-Object @{Name='pid';Expression={$_.Id}}, @{Name='process';Expression={$_.ProcessName}}, @{Name='title';Expression={$_.MainWindowTitle}}
([pscustomobject]@{
  ok = $true
  kind = 'window_list'
  windows = @($items)
  summary = "Listed $(@($items).Count) windows"
}) | ConvertTo-Json -Compress -Depth 5
"#;
    run_powershell(script, &[], 10).await
}

#[cfg(not(target_os = "windows"))]
async fn run_window_list() -> Result<String, String> {
    Err("Window listing is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_window_focus(envs: &[(String, String)]) -> Result<String, String> {
    let script = r#"
$ErrorActionPreference = 'Stop'
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WindowNative {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
}
"@
$pidText = $env:CU_PID
$title = $env:CU_TITLE
if (-not [string]::IsNullOrWhiteSpace($pidText)) {
  $proc = Get-Process -Id ([int]$pidText) -ErrorAction Stop
} else {
  $proc = Get-Process |
    Where-Object { $_.MainWindowHandle -ne 0 -and $_.MainWindowTitle -like "*$title*" } |
    Sort-Object @{Expression={$_.MainWindowTitle.Length}; Ascending=$true} |
    Select-Object -First 1
}
if ($null -eq $proc -or $proc.MainWindowHandle -eq 0) {
  throw "No matching foreground-capable window was found"
}
[WindowNative]::ShowWindowAsync($proc.MainWindowHandle, 9) | Out-Null
Start-Sleep -Milliseconds 80
$focused = [WindowNative]::SetForegroundWindow($proc.MainWindowHandle)
([pscustomobject]@{
  ok = [bool]$focused
  kind = 'window_focus'
  pid = $proc.Id
  process = $proc.ProcessName
  title = $proc.MainWindowTitle
  summary = "Focused window: $($proc.MainWindowTitle)"
}) | ConvertTo-Json -Compress -Depth 4
"#;
    run_powershell(script, envs, 10).await
}

#[cfg(not(target_os = "windows"))]
async fn run_window_focus(_envs: &[(String, String)]) -> Result<String, String> {
    Err("Window focus is currently implemented for Windows only.".to_string())
}

#[cfg(target_os = "windows")]
async fn run_powershell(
    script: &str,
    envs: &[(String, String)],
    timeout_secs: u64,
) -> Result<String, String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| format!("PowerShell action timed out after {timeout_secs}s"))?
        .map_err(|e| format!("Failed to run PowerShell action: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            "PowerShell action failed without error details".to_string()
        } else {
            detail
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_hotkeys_to_sendkeys() {
        let keys = vec!["ctrl".to_string(), "l".to_string()];
        assert_eq!(sendkeys_for_keys(&keys).unwrap(), "^l");

        let keys = vec!["ctrl".to_string(), "shift".to_string(), "esc".to_string()];
        assert_eq!(sendkeys_for_keys(&keys).unwrap(), "^+{ESC}");
    }

    #[test]
    fn rejects_sensitive_keyboard_text() {
        assert!(validate_keyboard_text("password=abc123").is_err());
        assert!(validate_keyboard_text("验证码 123456").is_err());
        assert!(validate_keyboard_text("sk-abcdefghijklmnopqrstuvwxyz").is_err());
        assert!(validate_keyboard_text("123456").is_err());
        assert!(validate_keyboard_text("hello from computer use").is_ok());
    }

    #[test]
    fn browser_urls_must_be_http_or_https() {
        assert!(validate_http_url("https://example.com").is_ok());
        assert!(validate_http_url("http://example.com").is_ok());
        assert!(validate_http_url("file:///C:/Windows/win.ini").is_err());
    }

    #[test]
    fn focus_restore_only_wraps_input_dependent_tools() {
        assert!(should_restore_focus_after_confirmation("computer_keyboard"));
        assert!(should_restore_focus_after_confirmation("computer_mouse"));
        assert!(!should_restore_focus_after_confirmation(
            "computer_screenshot"
        ));
        assert!(!should_restore_focus_after_confirmation("web_search"));
    }
}
