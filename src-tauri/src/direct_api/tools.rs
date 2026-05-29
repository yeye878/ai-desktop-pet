use serde_json::json;

pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "读取本地文件的内容（最大支持读取 64KB）",
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
                "description": "执行本地 Shell 命令（Windows 下使用 cmd /C 执行）。此工具为敏感工具，执行前会弹窗让用户确认。",
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
    ]
}

pub async fn execute_tool(name: &str, args: &serde_json::Value) -> String {
    match name {
        "read_file" => exec_read_file(args).await.unwrap_or_else(|e| e),
        "write_file" => exec_write_file(args).await.unwrap_or_else(|e| e),
        "list_directory" => exec_list_directory(args).await.unwrap_or_else(|e| e),
        "run_command" => exec_run_command(args).await.unwrap_or_else(|e| e),
        "web_search" => {
            let query = args["query"].as_str().unwrap_or("");
            if query.is_empty() {
                "搜索词不能为空".to_string()
            } else {
                web_search(query).await.unwrap_or_else(|e| e)
            }
        }
        _ => format!("未知工具: {name}"),
    }
}

async fn exec_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let path_buf = std::path::PathBuf::from(path);
    if !path_buf.is_file() {
        return Err(format!("路径不是一个文件: {path}"));
    }

    let content = tokio::fs::read_to_string(&path_buf)
        .await
        .map_err(|e| format!("读取文件失败: {e}"))?;

    if content.len() > 65536 {
        Ok(format!(
            "{}\n\n... [文件内容已截断，仅显示前 64KB]",
            &content[..65536]
        ))
    } else {
        Ok(content)
    }
}

async fn exec_write_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let content = args["content"].as_str().ok_or("缺少 'content' 参数")?;
    let path_buf = std::path::PathBuf::from(path);

    if let Some(parent) = path_buf.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建父目录失败: {e}"))?;
    }

    tokio::fs::write(&path_buf, content)
        .await
        .map_err(|e| format!("写入文件失败: {e}"))?;

    Ok(format!(
        "文件写入成功: {} ({} 字节)",
        path,
        content.len()
    ))
}

async fn exec_list_directory(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let path_buf = std::path::PathBuf::from(path);
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

            if result.len() > 8192 {
                result = format!(
                    "{}\n\n... [输出已截断，仅保留前 8KB]",
                    &result[..8192]
                );
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
        let snippet = if let Some(snippet_offset) = html[next_snippet_start..].find("class=\"result__snippet\"") {
            let abs_snippet_class = next_snippet_start + snippet_offset;
            if abs_snippet_class - next_snippet_start < 1500 {
                if let Some(snippet_tag_end_offset) = html[abs_snippet_class..].find('>') {
                    let snippet_content_start = abs_snippet_class + snippet_tag_end_offset + 1;
                    if let Some(snippet_end_offset) = html[snippet_content_start..].find("</") {
                        let raw_snippet = &html[snippet_content_start..snippet_content_start + snippet_end_offset];
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

        results.push(format!("* 标题: {}\n  链接: {}\n  摘要: {}\n", title, url, snippet));

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
        let end = href[start..].find('&').map(|idx| start + idx).unwrap_or(href.len());
        let encoded = &href[start..end];
        url::form_urlencoded::parse(encoded.as_bytes())
            .map(|(k, _)| k.into_owned())
            .collect::<String>()
    } else {
        href.to_string()
    }
}
