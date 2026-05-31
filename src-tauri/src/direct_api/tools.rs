use serde_json::json;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

fn canonicalize_requested_path(path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir()
            .map_err(|e| format!("获取当前目录失败: {e}"))?
            .join(raw)
    };

    if absolute.exists() {
        return absolute
            .canonicalize()
            .map_err(|e| format!("路径无效: {e}"));
    }

    let mut ancestor = absolute.as_path();
    let mut missing_components: Vec<OsString> = Vec::new();

    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("路径不存在: {path}"))?;
        missing_components.push(name.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("路径不存在: {path}"))?;
    }

    let mut resolved = ancestor
        .canonicalize()
        .map_err(|e| format!("父目录无效: {e}"))?;

    for component in missing_components.iter().rev() {
        resolved.push(component);
    }

    Ok(resolved)
}

fn canonical_allowed_roots() -> Vec<PathBuf> {
    [
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::desktop_dir(),
        dirs::audio_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        Some(std::env::temp_dir()),
    ]
    .into_iter()
    .flatten()
    .filter_map(|root| root.canonicalize().ok())
    .collect()
}

fn is_under_allowed_root(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// 检查路径是否在允许的目录范围内
fn is_path_allowed(path: &str) -> Result<PathBuf, String> {
    let canonical = canonicalize_requested_path(path)?;
    let allowed_roots = canonical_allowed_roots();

    if is_under_allowed_root(&canonical, &allowed_roots) {
        Ok(canonical)
    } else {
        Err(format!(
            "路径不在允许范围内，仅可访问用户文档、桌面、下载、媒体和临时目录: {}",
            path
        ))
    }
}

/// 检查命令是否包含危险操作
fn is_command_safe(command: &str) -> Result<(), String> {
    let dangerous_patterns = [
        "rm -rf /",
        "rmdir /s /q C:\\",
        "format ",
        "del /f /s /q C:\\",
        "shutdown",
        "restart",
        "reg delete",
        "reg add",
        "net user",
        "net localgroup",
    ];

    let cmd_lower = command.to_lowercase();
    for pattern in &dangerous_patterns {
        if cmd_lower.contains(&pattern.to_lowercase()) {
            return Err(format!("命令包含危险操作，已被拦截: {}", pattern));
        }
    }

    Ok(())
}

pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "读取本地文件的内容（最大支持读取 512KB）",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "要读取的文件的绝对路径" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "写入或覆盖本地文件内容，如果父目录不存在会自动创建",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "要写入的绝对路径" },
                        "content": { "type": "string", "description": "要写入的文件内容" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_directory",
                "description": "列出指定目录下的子文件与目录（最多显示 200 个条目）",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "目录的绝对路径" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "执行本地 Shell 命令（Windows 下使用 cmd /C 执行），输出上限 64KB，超时 30 秒。此工具为敏感工具，执行前会弹窗让用户确认。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "要执行的命令行" }
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "实时网页搜索，搜索最新的资讯、天气或技术文档，返回包含标题、链接及简短描述的结果列表",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "要搜索的查询词" }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "open_app",
                "description": "打开本地应用程序。支持应用名称（如 notepad、chrome、vscode）或可执行文件的完整路径。启动后立即返回。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "应用名称（如 notepad, chrome, vscode, calc, explorer）或可执行文件的完整路径" },
                        "args": { "type": "string", "description": "传递给应用的可选参数（如要打开的文件路径）" }
                    },
                    "required": ["app"]
                }
            }
        }),
    ]
}

pub async fn execute_tool(name: &str, args: &serde_json::Value) -> String {
    let timeout_duration = std::time::Duration::from_secs(30);
    match name {
        "read_file" => match tokio::time::timeout(timeout_duration, exec_read_file(args)).await {
            Ok(result) => result.unwrap_or_else(|e| e),
            Err(_) => "读取文件超时 (30秒)".to_string(),
        },
        "write_file" => match tokio::time::timeout(timeout_duration, exec_write_file(args)).await {
            Ok(result) => result.unwrap_or_else(|e| e),
            Err(_) => "写入文件超时 (30秒)".to_string(),
        },
        "list_directory" => {
            match tokio::time::timeout(timeout_duration, exec_list_directory(args)).await {
                Ok(result) => result.unwrap_or_else(|e| e),
                Err(_) => "列出目录超时 (30秒)".to_string(),
            }
        }
        "run_command" => exec_run_command(args).await.unwrap_or_else(|e| e),
        "web_search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                "搜索词不能为空".to_string()
            } else {
                web_search(query).await.unwrap_or_else(|e| e)
            }
        }
        "open_app" => exec_open_app(args).await.unwrap_or_else(|e| e),
        _ => format!("未知工具: {name}"),
    }
}

async fn exec_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let path_buf = is_path_allowed(path)?;
    if !path_buf.is_file() {
        return Err(format!("路径不是一个文件: {path}"));
    }

    let content = tokio::fs::read_to_string(&path_buf)
        .await
        .map_err(|e| format!("读取文件失败: {e}"))?;

    if content.len() > 524288 {
        let safe_end = content.floor_char_boundary(524288);
        Ok(format!(
            "{}\n\n... [文件内容已截断，仅显示前 512KB]",
            &content[..safe_end]
        ))
    } else {
        Ok(content)
    }
}

async fn exec_write_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let content = args["content"].as_str().ok_or("缺少 'content' 参数")?;
    let path_buf = is_path_allowed(path)?;

    if let Some(parent) = path_buf.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建父目录失败: {e}"))?;
    }

    tokio::fs::write(&path_buf, content)
        .await
        .map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(format!("文件写入成功: {} ({} 字节)", path, content.len()))
}

async fn exec_list_directory(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let path_buf = is_path_allowed(path)?;
    if !path_buf.is_dir() {
        return Err(format!("路径不是一个目录: {path}"));
    }

    let mut dir = tokio::fs::read_dir(&path_buf)
        .await
        .map_err(|e| format!("读取目录失败: {e}"))?;

    let mut items = Vec::new();
    while let Some(entry) = dir
        .next_entry()
        .await
        .map_err(|e| format!("读取目录条目失败: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = entry.metadata().await;
        let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

        let type_str = if is_dir { "目录" } else { "文件" };
        if is_dir {
            items.push(format!("[{}] {}", type_str, name));
        } else {
            items.push(format!("[{}] {} ({} 字节)", type_str, name, size));
        }

        if items.len() >= 200 {
            items.push("... [已截断，仅列出前 200 个条目]".to_string());
            break;
        }
    }

    if items.is_empty() {
        Ok("目录为空".to_string())
    } else {
        Ok(items.join("\n"))
    }
}

async fn exec_run_command(args: &serde_json::Value) -> Result<String, String> {
    let command = args["command"].as_str().ok_or("缺少 'command' 参数")?;

    // 检查命令安全性
    is_command_safe(command)?;

    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };

    #[cfg(target_os = "windows")]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("执行命令失败: {e}"))?;

    let mut stdout = child.stdout.take().expect("无法打开标准输出");
    let mut stderr = child.stderr.take().expect("无法打开标准错误");

    let stdout_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        use tokio::io::AsyncReadExt;
        let _ = stdout.read_to_end(&mut buffer).await;
        buffer
    });

    let stderr_task = tokio::spawn(async move {
        let mut buffer = Vec::new();
        use tokio::io::AsyncReadExt;
        let _ = stderr.read_to_end(&mut buffer).await;
        buffer
    });

    let timeout_duration = std::time::Duration::from_secs(30);

    match tokio::time::timeout(timeout_duration, child.wait()).await {
        Ok(Ok(status)) => {
            let stdout_bytes = stdout_task.await.unwrap_or_default();
            let stderr_bytes = stderr_task.await.unwrap_or_default();

            let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();

            let status_code = status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "未知".to_string());

            let mut result = format!("执行状态 (退出码 {}):\n", status_code);
            if !stdout.is_empty() {
                result.push_str(&format!("--- 标准输出 ---\n{}\n", stdout));
            }
            if !stderr.is_empty() {
                result.push_str(&format!("--- 标准错误 ---\n{}\n", stderr));
            }

            if result.len() > 65536 {
                let safe_end = result.floor_char_boundary(65536);
                result = format!("{}\n\n... [输出已截断，仅保留前 64KB]", &result[..safe_end]);
            }

            Ok(result)
        }
        Ok(Err(e)) => Err(format!("命令执行失败: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            Err("命令执行超时 (限制 30 秒)".to_string())
        }
    }
}

pub async fn web_search(query: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {e}"))?;

    let html = res
        .text()
        .await
        .map_err(|e| format!("读取搜索响应数据失败: {e}"))?;

    let mut results = Vec::new();
    let mut search_pos = 0;

    while let Some(title_start) = html[search_pos..].find("<a class=\"result__a\" href=\"") {
        let abs_title_start = search_pos + title_start;
        let href_start = abs_title_start + "<a class=\"result__a\" href=\"".len();

        let Some(href_end_offset) = html[href_start..].find('"') else {
            search_pos = href_start;
            continue;
        };
        let raw_href = &html[href_start..href_start + href_end_offset];

        let tag_end_offset = html[href_start + href_end_offset..].find('>');
        let Some(tag_end) = tag_end_offset.map(|o| href_start + href_end_offset + o + 1) else {
            search_pos = href_start;
            continue;
        };

        let Some(anchor_end_offset) = html[tag_end..].find("</a>") else {
            search_pos = tag_end;
            continue;
        };
        let raw_title = &html[tag_end..tag_end + anchor_end_offset];

        let title = strip_html_tags(raw_title);
        let url = extract_actual_url(raw_href);

        // Find the snippet
        let next_snippet_start = tag_end + anchor_end_offset;
        let snippet = if let Some(snippet_offset) =
            html[next_snippet_start..].find("class=\"result__snippet\"")
        {
            let abs_snippet_class = next_snippet_start + snippet_offset;
            if abs_snippet_class - next_snippet_start < 1500 {
                if let Some(snippet_tag_end_offset) = html[abs_snippet_class..].find('>') {
                    let snippet_content_start = abs_snippet_class + snippet_tag_end_offset + 1;
                    if let Some(snippet_end_offset) = html[snippet_content_start..].find("</") {
                        let raw_snippet = &html
                            [snippet_content_start..snippet_content_start + snippet_end_offset];
                        strip_html_tags(raw_snippet)
                    } else {
                        "无描述".to_string()
                    }
                } else {
                    "无描述".to_string()
                }
            } else {
                "无描述".to_string()
            }
        } else {
            "无描述".to_string()
        };

        results.push(format!(
            "* 标题: {}\n  链接: {}\n  摘要: {}\n",
            title, url, snippet
        ));

        search_pos = next_snippet_start;
        if results.len() >= 5 {
            break;
        }
    }

    if results.is_empty() {
        if html.contains("ddg-captcha") || html.contains("robot") {
            return Err("触发反爬验证码，搜索被阻断，请稍后再试。".to_string());
        }
        Ok("未找到相关的网页搜索结果。".to_string())
    } else {
        Ok(results.join("\n"))
    }
}

fn strip_html_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&nbsp;", " ")
}

fn extract_actual_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let start = pos + 5;
        let end = href[start..]
            .find('&')
            .map(|idx| start + idx)
            .unwrap_or(href.len());
        let encoded = &href[start..end];
        url::form_urlencoded::parse(encoded.as_bytes())
            .map(|(k, _)| k.into_owned())
            .collect::<String>()
    } else {
        href.to_string()
    }
}

fn resolve_app_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let alias_map: &[(&str, &str)] = &[
        // Windows 内置
        ("notepad", "notepad.exe"),
        ("\u{8BB0}\u{4E8B}\u{672C}", "notepad.exe"),
        ("calculator", "calc.exe"),
        ("\u{8BA1}\u{7B97}\u{5668}", "calc.exe"),
        ("calc", "calc.exe"),
        ("explorer", "explorer.exe"),
        ("\u{6587}\u{4EF6}\u{7BA1}\u{7406}\u{5668}", "explorer.exe"),
        ("mspaint", "mspaint.exe"),
        ("\u{753B}\u{56FE}", "mspaint.exe"),
        ("snippingtool", "snippingtool.exe"),
        ("\u{622A}\u{56FE}", "snippingtool.exe"),
        // 浏览器
        ("chrome", "chrome"),
        ("edge", "msedge"),
        ("firefox", "firefox"),
        // 开发工具
        ("vscode", "code"),
        ("code", "code"),
        ("visual studio code", "code"),
        ("cursor", "cursor"),
        // 通讯工具
        ("wechat", "WeChat"),
        ("\u{5FAE}\u{4FE1}", "WeChat"),
        ("qq", "QQ"),
        ("dingtalk", "DingTalk"),
        ("\u{9489}\u{9489}", "DingTalk"),
        ("feishu", "Lark"),
        ("\u{98DE}\u{4E66}", "Lark"),
        ("teams", "ms-teams"),
        // 办公
        ("word", "winword"),
        ("excel", "excel"),
        ("ppt", "powerpnt"),
        ("powerpoint", "powerpnt"),
        // 其他
        ("spotify", "spotify"),
        ("obs", "obs64"),
    ];
    for (alias, executable) in alias_map {
        if lower == *alias {
            return executable.to_string();
        }
    }
    name.to_string()
}

fn get_db_path_for_tools() -> Result<std::path::PathBuf, String> {
    let mut path = dirs::data_dir().ok_or("无法获取数据目录")?;
    path.push("ai-desktop-pet");
    path.push("pet.db");
    Ok(path)
}

async fn exec_open_app(args: &serde_json::Value) -> Result<String, String> {
    let app = args["app"].as_str().ok_or("缺少 'app' 参数")?;
    let app_args = args["args"].as_str().unwrap_or("");
    let mut resolved = resolve_app_name(app);

    // 检查数据库中是否存在用户拖入并注册的快捷方式应用路径
    if let Ok(db_path) = get_db_path_for_tools() {
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let stmt = conn.prepare("SELECT value FROM pet_memory WHERE category = 'app_path' AND LOWER(key) = LOWER(?1)");
            if let Ok(mut s) = stmt {
                let rows = s.query_map([app], |row| row.get::<_, String>(0));
                if let Ok(mut r) = rows {
                    if let Some(Ok(val)) = r.next() {
                        resolved = val;
                    }
                }
            }
        }
    }

    let mut cmd = tokio::process::Command::new("cmd");
    cmd.args(["/C", "start", "", &resolved]);
    if !app_args.is_empty() {
        cmd.arg(app_args);
    }

    #[cfg(target_os = "windows")]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    cmd.spawn().map_err(|e| format!("启动应用失败: {e}"))?;
    Ok(format!("已启动: {} ({})", app, resolved))
}
