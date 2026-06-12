use crate::{computer_use, system};
use chrono::{Duration as ChronoDuration, Local, NaiveDate};
use serde_json::json;
use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DEFAULT_SEARCH_RESULTS: usize = 5;
const MAX_SEARCH_RESULTS: usize = 8;
const SEARCH_CANDIDATE_LIMIT: usize = 12;
const MAX_SEARCH_SNIPPET_CHARS: usize = 240;
const MAX_WEBPAGE_CHARS: usize = 12_000;
const MAX_WEBPAGE_EXCERPTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchProvider {
    DuckDuckGoHtml,
    BingRss,
    BingHtml,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Clone)]
struct SearchOptions {
    query: String,
    max_results: usize,
    site: Option<String>,
    exclude_domains: Vec<String>,
    recency_days: Option<u32>,
}

impl SearchProvider {
    fn name(self) -> &'static str {
        match self {
            SearchProvider::DuckDuckGoHtml => "DuckDuckGo",
            SearchProvider::BingRss => "Bing RSS",
            SearchProvider::BingHtml => "Bing",
        }
    }

    fn url(self) -> &'static str {
        match self {
            SearchProvider::DuckDuckGoHtml => "https://html.duckduckgo.com/html/",
            SearchProvider::BingRss | SearchProvider::BingHtml => "https://cn.bing.com/search",
        }
    }
}

fn provider_order(preferred_provider: &str) -> [SearchProvider; 3] {
    match preferred_provider.trim().to_lowercase().as_str() {
        "duckduckgo" | "duck" | "ddg" => [
            SearchProvider::DuckDuckGoHtml,
            SearchProvider::BingRss,
            SearchProvider::BingHtml,
        ],
        _ => [
            SearchProvider::BingRss,
            SearchProvider::BingHtml,
            SearchProvider::DuckDuckGoHtml,
        ],
    }
}

fn infer_recency_days(query: &str) -> Option<u32> {
    let lower = query.trim().to_lowercase();
    if lower.is_empty() {
        return None;
    }

    if contains_any(
        &lower,
        &[
            "今天",
            "今日",
            "当天",
            "日报",
            "早报",
            "晚报",
            "速览",
            "快讯",
            "breaking",
            "today",
            "daily briefing",
            "daily brief",
        ],
    ) {
        return Some(2);
    }

    if contains_any(&lower, &["昨天", "昨日", "yesterday"]) {
        return Some(3);
    }

    if contains_any(&lower, &["本周", "这周", "近一周", "最近一周", "this week"]) {
        return Some(7);
    }

    if contains_any(
        &lower,
        &["本月", "这个月", "近一个月", "最近一个月", "this month"],
    ) {
        return Some(31);
    }

    if contains_any(&lower, &["今年", "this year"]) {
        return Some(365);
    }

    if contains_any(
        &lower,
        &[
            "最近", "最新", "近期", "新闻", "动态", "现状", "进展", "更新", "latest", "recent",
            "news", "current", "update",
        ],
    ) {
        return Some(7);
    }

    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

impl SearchOptions {
    fn new(query: &str) -> Self {
        Self {
            query: query.trim().to_string(),
            max_results: DEFAULT_SEARCH_RESULTS,
            site: None,
            exclude_domains: Vec::new(),
            recency_days: infer_recency_days(query),
        }
    }

    fn from_tool_args(args: &serde_json::Value) -> Result<Self, String> {
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "搜索词不能为空".to_string())?;

        let max_results = args["max_results"]
            .as_u64()
            .map(|value| (value as usize).clamp(1, MAX_SEARCH_RESULTS))
            .unwrap_or(DEFAULT_SEARCH_RESULTS);

        let site = args["site"].as_str().and_then(normalize_domain_filter);

        let exclude_domains = args["exclude_domains"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .filter_map(normalize_domain_filter)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let recency_days = args["recency_days"]
            .as_u64()
            .map(|value| (value as u32).clamp(1, 365))
            .or_else(|| infer_recency_days(query));

        Ok(Self {
            query: query.to_string(),
            max_results,
            site,
            exclude_domains,
            recency_days,
        })
    }

    fn effective_query(&self) -> String {
        self.effective_query_for_date(Local::now().date_naive())
    }

    fn effective_query_for_date(&self, today: NaiveDate) -> String {
        let query = self.query.trim();
        let lower = query.to_lowercase();
        let mut parts = vec![query.to_string()];

        if let Some(days) = self.recency_days {
            if !lower.contains("after:") && !lower.contains("before:") {
                let start_date = today - ChronoDuration::days(days as i64);
                parts.push(format!("after:{}", start_date.format("%Y-%m-%d")));
            }
        }

        if let Some(site) = self.site.as_deref() {
            if !lower.contains("site:") {
                parts.push(format!("site:{site}"));
            }
        }

        parts.join(" ")
    }
}

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
    let mut roots = Vec::new();
    for root in [
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::desktop_dir(),
        dirs::audio_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        std::env::current_dir().ok(),
        Some(std::env::temp_dir()),
    ]
    .into_iter()
    .flatten()
    {
        push_canonical_root(&mut roots, root);
    }
    roots
}

fn push_canonical_root(roots: &mut Vec<PathBuf>, root: PathBuf) {
    if let Ok(canonical) = root.canonicalize() {
        if !roots.iter().any(|existing| existing == &canonical) {
            roots.push(canonical);
        }
    }
}

fn canonical_blocked_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    {
        for name in [
            "WINDIR",
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ] {
            if let Some(value) = std::env::var_os(name) {
                push_canonical_root(&mut roots, PathBuf::from(value));
            }
        }

        if let Some(system_drive) = std::env::var_os("SystemDrive") {
            let drive = system_drive.to_string_lossy();
            for suffix in [
                "$Recycle.Bin",
                "Recovery",
                "System Volume Information",
                "Windows.old",
            ] {
                push_canonical_root(&mut roots, PathBuf::from(format!("{drive}\\{suffix}")));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        for root in [
            "/bin", "/boot", "/dev", "/etc", "/proc", "/root", "/sbin", "/sys", "/usr",
        ] {
            push_canonical_root(&mut roots, PathBuf::from(root));
        }
    }

    roots
}

fn is_under_allowed_root(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn has_sensitive_component(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".ssh"
                | ".gnupg"
                | ".aws"
                | ".azure"
                | ".kube"
                | ".docker"
                | ".password-store"
                | "id_rsa"
                | "id_dsa"
                | "id_ecdsa"
                | "id_ed25519"
                | "ntuser.dat"
        ) || name == ".env"
            || name.starts_with(".env.")
            || name.ends_with(".pem")
            || name.ends_with(".pfx")
            || name.ends_with(".key")
    })
}

/// 检查路径是否避开系统目录和常见敏感凭据路径。
pub fn is_path_allowed(path: &str) -> Result<PathBuf, String> {
    let canonical = canonicalize_requested_path(path)?;
    let blocked_roots = canonical_blocked_roots();

    if is_under_allowed_root(&canonical, &blocked_roots) {
        return Err(format!("路径位于系统目录中，已拒绝访问: {}", path));
    }

    if has_sensitive_component(&canonical) {
        return Err(format!(
            "路径疑似包含敏感凭据或密钥文件，已拒绝访问: {}",
            path
        ));
    }

    Ok(canonical)
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
                "description": "读取本地文件的内容（最大 512KB），支持文本文件、.docx、.pdf 和 .pptx/.pptm/.ppsx 格式；会拒绝系统目录和常见敏感凭据路径",
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
                "description": "实时网页搜索。请使用具体查询词、关键实体、日期、版本号或 site 限定，避免宽泛搜索；结果会本地去重、去噪并按相关性重排。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "具体搜索查询词，优先包含专名、版本、日期、错误码、文件名或关键短语" },
                        "max_results": { "type": "integer", "description": "可选，返回结果数量，默认 5，上限 8" },
                        "recency_days": { "type": "integer", "description": "可选，只关注最近 N 天内容；新闻、日报、最新动态优先设置为 1-7，长期资料可不传" },
                        "site": { "type": "string", "description": "可选，只搜索指定域名，例如 openai.com、docs.rs、github.com" },
                        "exclude_domains": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "可选，排除明显无关或低质量的域名"
                        }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "read_webpage",
                "description": "读取指定网页正文。若正在验证某个问题，请传入 query，工具会只返回最相关正文片段以减少噪音；未传 query 时返回压缩后的正文开头。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "要读取的网页 URL（必须是 http 或 https 链接）" },
                        "query": { "type": "string", "description": "可选，用于提取相关片段的问题或关键词" }
                    },
                    "required": ["url"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "查询桌宠内置天气功能的当前天气。默认使用设置页保存的城市/地区与 wttr.in 兼容天气 API；可用 location 临时查询其他城市。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": { "type": "string", "description": "可选，城市或地区，例如 Tokyo、上海；不提供时使用天气设置中的位置，设置为空则由天气源自动定位" }
                    }
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
        json!({
            "type": "function",
            "function": {
                "name": "computer_screenshot",
                "description": "Capture the user's desktop as an image for visual inspection. Requires Computer Use to be enabled in settings. Use before mouse/keyboard actions when the current UI state matters.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "display": { "type": "integer", "description": "Optional display index. Defaults to 0." },
                        "max_width": { "type": "integer", "description": "Optional max image width for compression. Defaults to 1024." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "computer_mouse",
                "description": "Control the mouse after observing the screen. Actions: move, click, double_click, right_click, drag, scroll. Coordinates are absolute screen pixels.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["move", "click", "double_click", "right_click", "drag", "scroll"] },
                        "x": { "type": "integer", "description": "Absolute screen x coordinate." },
                        "y": { "type": "integer", "description": "Absolute screen y coordinate." },
                        "to_x": { "type": "integer", "description": "Drag destination x coordinate." },
                        "to_y": { "type": "integer", "description": "Drag destination y coordinate." },
                        "dx": { "type": "integer", "description": "Relative x delta for drag when to_x is omitted." },
                        "dy": { "type": "integer", "description": "Relative y delta for drag or scroll." },
                        "amount": { "type": "integer", "description": "Mouse wheel notches for scroll. Positive scrolls up, negative scrolls down." },
                        "button": { "type": "string", "enum": ["left", "right"], "description": "Mouse button for click. Defaults to left." }
                    },
                    "required": ["action"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "computer_keyboard",
                "description": "Type text, press a key, or send a hotkey to the focused app. Use one type call for a complete known text string instead of typing character-by-character. Sensitive-looking secrets and short numeric verification codes are refused.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["type", "press", "hotkey"] },
                        "text": { "type": "string", "description": "Text to type when action is type." },
                        "key": { "type": "string", "description": "Single key for press, e.g. enter, tab, escape, a, f5." },
                        "keys": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Keys for hotkey, e.g. [\"ctrl\", \"l\"] or [\"ctrl\", \"shift\", \"esc\"]."
                        }
                    },
                    "required": ["action"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "computer_wait",
                "description": "Wait briefly for UI changes after an app launch, click, navigation, or keyboard action.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ms": { "type": "integer", "description": "Milliseconds to wait, clamped between 50 and 10000." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "window_list",
                "description": "List visible desktop windows with pid, process name, and title. Useful before focusing QQ, WeChat, browsers, or other native apps.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "window_focus",
                "description": "Focus a visible desktop window by pid or by partial title match.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pid": { "type": "integer", "description": "Process id from window_list." },
                        "title": { "type": "string", "description": "Partial window title to match." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_open",
                "description": "Open the default browser, Chrome, or Edge. If url is provided it must be http or https.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Optional http/https URL to open." },
                        "browser": { "type": "string", "enum": ["default", "chrome", "edge"], "description": "Browser target. Defaults to default." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_navigate",
                "description": "Navigate by opening an http/https URL in the selected browser. Use browser_snapshot or computer_screenshot afterward to inspect the result.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "http/https URL to open." },
                        "browser": { "type": "string", "enum": ["default", "chrome", "edge"], "description": "Browser target. Defaults to default." }
                    },
                    "required": ["url"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "browser_snapshot",
                "description": "Capture the desktop after browser navigation. This is a screenshot alias used for browser workflows.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "display": { "type": "integer", "description": "Optional display index. Defaults to 0." },
                        "max_width": { "type": "integer", "description": "Optional max image width for compression. Defaults to 1024." }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "create_scheduled_task",
                "description": "创建一个本地定时提醒任务。适合用户要求“稍后提醒我”“每天提醒我”“定时任务”等场景。需要明确标题和提醒时间；可以用 due_at Unix 秒，或 delay_minutes/小时/天这样的相对时间。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string", "description": "任务标题，例如：提交周报" },
                        "note": { "type": "string", "description": "可选备注或提醒详情" },
                        "due_at": { "type": "integer", "description": "可选，Unix 秒时间戳，表示第一次提醒时间" },
                        "delay_minutes": { "type": "number", "description": "可选，从现在开始多少分钟后提醒；当没有 due_at 时使用" },
                        "delay_hours": { "type": "number", "description": "可选，从现在开始多少小时后提醒；当没有 due_at/delay_minutes 时使用" },
                        "delay_days": { "type": "number", "description": "可选，从现在开始多少天后提醒；当没有更精确字段时使用" },
                        "repeat": { "type": "string", "enum": ["once", "daily", "weekly", "monthly"], "description": "重复规则，默认 once" }
                    },
                    "required": ["title"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_scheduled_tasks",
                "description": "列出用户已经设置的本地定时提醒任务。",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "search_memory",
                "description": "搜索长期记忆库中的已保存记忆。可按关键词（在标题和内容中匹配）和/或分类筛选。返回匹配的记忆条目列表，包含ID、分类、标题、内容和创建时间。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "搜索关键词，在记忆的标题（key）和内容（value）中模糊匹配" },
                        "category": { "type": "string", "description": "可选，按分类筛选记忆（如 preference、fact、habit 等）" },
                        "limit": { "type": "integer", "description": "可选，返回结果数量上限，默认 10，最大 30" }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "save_memory",
                "description": "将一条信息保存到长期记忆库中。适合保存用户的偏好、事实、习惯或任何需要跨会话记住的内容。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "category": { "type": "string", "description": "记忆分类，如 preference（偏好）、fact（事实）、habit（习惯）、note（备注）等" },
                        "key": { "type": "string", "description": "记忆的简短标题或标识（如 '用户喜欢的颜色'）" },
                        "value": { "type": "string", "description": "要保存的记忆内容" }
                    },
                    "required": ["category", "key", "value"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "file_search",
                "description": "在指定目录下递归搜索匹配文件名模式的文件，支持通配符（如 *.pdf、*report*、*.pptx、*.py）。不指定目录时搜索用户常用目录和当前项目；指定目录时会拒绝系统目录和常见敏感凭据路径。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "文件名匹配模式，支持 * 和 ? 通配符，如 *.pdf、*report*、*.pptx、*.py" },
                        "directory": { "type": "string", "description": "可选，搜索起始目录；不指定时搜索用户常用目录和当前项目。" },
                        "max_results": { "type": "integer", "description": "可选，最大返回结果数，默认 50，上限 200" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "delete_memory",
                "description": "根据记忆 ID 删除长期记忆库中的一条记忆。这是不可逆操作。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "要删除的记忆的 ID" }
                    },
                    "required": ["id"]
                }
            }
        }),
    ]
}

pub async fn execute_tool(
    name: &str,
    args: &serde_json::Value,
    preferred_search_provider: &str,
) -> String {
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
            let options = match SearchOptions::from_tool_args(args) {
                Ok(options) => options,
                Err(message) => return message,
            };
            let has_advanced_options = args.get("max_results").is_some()
                || args.get("site").is_some()
                || args.get("exclude_domains").is_some()
                || args.get("recency_days").is_some();
            if has_advanced_options {
                web_search_with_options(&options, preferred_search_provider)
                    .await
                    .unwrap_or_else(|e| e)
            } else {
                web_search(&options.query, preferred_search_provider)
                    .await
                    .unwrap_or_else(|e| e)
            }
        }
        "read_webpage" => {
            let url = args["url"].as_str().unwrap_or("");
            if url.is_empty() {
                "URL 不能为空".to_string()
            } else {
                let focus_query = args["query"]
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                match tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    read_webpage(url, focus_query),
                )
                .await
                {
                    Ok(result) => result.unwrap_or_else(|e| e),
                    Err(_) => "读取网页超时 (15秒)".to_string(),
                }
            }
        }
        "get_weather" => exec_get_weather(args).await.unwrap_or_else(|e| e),
        "open_app" => exec_open_app(args).await.unwrap_or_else(|e| e),
        "computer_screenshot" => computer_use::screenshot(args).await.unwrap_or_else(|e| e),
        "computer_mouse" => computer_use::mouse(args).await.unwrap_or_else(|e| e),
        "computer_keyboard" => computer_use::keyboard(args).await.unwrap_or_else(|e| e),
        "computer_wait" => computer_use::wait(args).await.unwrap_or_else(|e| e),
        "window_list" => computer_use::window_list(args).await.unwrap_or_else(|e| e),
        "window_focus" => computer_use::window_focus(args).await.unwrap_or_else(|e| e),
        "browser_open" => computer_use::browser_open(args).await.unwrap_or_else(|e| e),
        "browser_navigate" => computer_use::browser_navigate(args)
            .await
            .unwrap_or_else(|e| e),
        "browser_snapshot" => computer_use::browser_snapshot(args)
            .await
            .unwrap_or_else(|e| e),
        "create_scheduled_task" => exec_create_scheduled_task(args).await.unwrap_or_else(|e| e),
        "list_scheduled_tasks" => exec_list_scheduled_tasks().await.unwrap_or_else(|e| e),
        "file_search" => exec_file_search(args).await.unwrap_or_else(|e| e),
        "search_memory" => exec_search_memory(args).await.unwrap_or_else(|e| e),
        "save_memory" => exec_save_memory(args).await.unwrap_or_else(|e| e),
        "delete_memory" => exec_delete_memory(args).await.unwrap_or_else(|e| e),
        _ => format!("未知工具: {name}"),
    }
}

async fn exec_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    const MAX_READ_BYTES: u64 = 512 * 1024;
    let path_buf = is_path_allowed(path)?;
    if !path_buf.is_file() {
        return Err(format!("路径不是一个文件: {path}"));
    }
    let size = tokio::fs::metadata(&path_buf)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if size > MAX_READ_BYTES {
        return Err(format!(
            "文件过大：{} 字节（上限 {} 字节 / 512KB）",
            size, MAX_READ_BYTES
        ));
    }

    // Office/PDF 二进制或压缩格式需要特殊处理
    let ext = path_buf
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("docx") => return read_docx_text(&path_buf),
        Some("pdf") => return read_pdf_text(&path_buf),
        Some("pptx") | Some("pptm") | Some("ppsx") => return read_pptx_text(&path_buf),
        Some("ppt") => {
            return Err("旧版 .ppt 二进制格式暂不支持，请将文件另存为 .pptx 后再读取".to_string())
        }
        _ => {}
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

/// 从 .docx 文件中提取纯文本内容
fn read_docx_text(path: &Path) -> Result<String, String> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| format!("打开 docx 文件失败: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("解析 docx 压缩包失败: {e}"))?;

    let mut xml_content = String::new();
    {
        let mut entry = archive
            .by_name("word/document.xml")
            .map_err(|_| "docx 文件中未找到 word/document.xml".to_string())?;
        entry
            .read_to_string(&mut xml_content)
            .map_err(|e| format!("读取 document.xml 失败: {e}"))?;
    }

    let text = extract_open_xml_text(&xml_content);
    finish_extracted_text("docx", text, "docx 文件中未提取到文本内容")
}

/// 从 .pptx/.pptm/.ppsx 文件中提取每页幻灯片和备注的纯文本内容
fn read_pptx_text(path: &Path) -> Result<String, String> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|e| format!("打开 pptx 文件失败: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("解析 pptx 压缩包失败: {e}"))?;

    let slide_names = sorted_zip_part_names(&mut archive, "ppt/slides/slide");
    if slide_names.is_empty() {
        return Err("pptx 文件中未找到幻灯片 XML 内容".to_string());
    }

    let note_names = sorted_zip_part_names(&mut archive, "ppt/notesSlides/notesSlide");
    let mut sections = Vec::new();

    for name in slide_names {
        let mut xml_content = String::new();
        {
            let mut entry = archive
                .by_name(&name)
                .map_err(|e| format!("读取 pptx 幻灯片失败 ({name}): {e}"))?;
            entry
                .read_to_string(&mut xml_content)
                .map_err(|e| format!("读取 pptx 幻灯片 XML 失败 ({name}): {e}"))?;
        }

        let text = extract_open_xml_text(&xml_content);
        if !text.is_empty() {
            let slide_no = open_xml_part_number(&name).unwrap_or(sections.len() + 1);
            sections.push(format!("幻灯片 {slide_no}\n{text}"));
        }
    }

    for name in note_names {
        let mut xml_content = String::new();
        {
            let mut entry = archive
                .by_name(&name)
                .map_err(|e| format!("读取 pptx 备注页失败 ({name}): {e}"))?;
            entry
                .read_to_string(&mut xml_content)
                .map_err(|e| format!("读取 pptx 备注页 XML 失败 ({name}): {e}"))?;
        }

        let text = extract_open_xml_text(&xml_content);
        if !text.is_empty() {
            let note_no = open_xml_part_number(&name).unwrap_or(sections.len() + 1);
            sections.push(format!("备注 {note_no}\n{text}"));
        }
    }

    finish_extracted_text(
        "pptx",
        sections.join("\n\n"),
        "pptx 文件中未提取到文本内容（可能主要是图片/图表，不支持 OCR）",
    )
}

fn sorted_zip_part_names<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    prefix: &str,
) -> Vec<String> {
    let mut names = Vec::new();
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            let name = entry.name().replace('\\', "/");
            if name.starts_with(prefix) && name.ends_with(".xml") {
                names.push(name);
            }
        }
    }

    names.sort_by(|left, right| {
        open_xml_part_number(left)
            .unwrap_or(usize::MAX)
            .cmp(&open_xml_part_number(right).unwrap_or(usize::MAX))
            .then_with(|| left.cmp(right))
    });
    names
}

fn open_xml_part_number(name: &str) -> Option<usize> {
    let file_name = name.rsplit('/').next().unwrap_or(name);
    let stem = file_name.strip_suffix(".xml").unwrap_or(file_name);
    let digits_reversed: String = stem
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    if digits_reversed.is_empty() {
        return None;
    }
    digits_reversed
        .chars()
        .rev()
        .collect::<String>()
        .parse()
        .ok()
}

fn extract_open_xml_text(xml_content: &str) -> String {
    let mut text = String::new();
    let mut in_text = false;
    let mut segment = String::new();
    let mut chars = xml_content.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c == '<' {
            let mut tag = String::new();
            while let Some(&ch) = chars.peek() {
                tag.push(ch);
                chars.next();
                if ch == '>' {
                    break;
                }
            }
            let Some((is_closing, is_self_closing, local_name)) = parse_xml_tag(&tag) else {
                continue;
            };
            if is_self_closing && local_name == "t" {
                in_text = false;
                segment.clear();
            } else if !is_closing && local_name == "t" {
                in_text = true;
                segment.clear();
            } else if is_closing && local_name == "t" {
                in_text = false;
                text.push_str(&decode_xml_entities(&segment));
                segment.clear();
            } else if local_name == "tab" {
                if in_text {
                    segment.push(' ');
                }
            } else if local_name == "br" || (is_closing && local_name == "p") {
                text.push('\n');
            }
        } else {
            chars.next();
            if in_text {
                segment.push(c);
            }
        }
    }

    clean_extracted_text(&text)
}

fn parse_xml_tag(tag: &str) -> Option<(bool, bool, String)> {
    let trimmed = tag
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();

    if trimmed.is_empty() || trimmed.starts_with('?') || trimmed.starts_with('!') {
        return None;
    }

    let is_closing = trimmed.starts_with('/');
    let body = if is_closing {
        trimmed.trim_start_matches('/').trim()
    } else {
        trimmed
    };
    let is_self_closing = body.ends_with('/');
    let body = body.trim_end_matches('/').trim();
    let tag_name = body.split_whitespace().next()?;
    let local_name = tag_name.rsplit(':').next().unwrap_or(tag_name);
    Some((is_closing, is_self_closing, local_name.to_ascii_lowercase()))
}

fn decode_xml_entities(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '&' {
            output.push(ch);
            continue;
        }

        let mut entity = String::new();
        let mut found_end = false;
        while let Some(next) = chars.next() {
            if next == ';' {
                found_end = true;
                break;
            }
            entity.push(next);
            if entity.len() > 16 {
                break;
            }
        }

        if !found_end {
            output.push('&');
            output.push_str(&entity);
            continue;
        }

        let decoded = match entity.as_str() {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") => u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };

        if let Some(decoded) = decoded {
            output.push(decoded);
        } else {
            output.push('&');
            output.push_str(&entity);
            output.push(';');
        }
    }

    output
}

fn clean_extracted_text(text: &str) -> String {
    let mut cleaned = String::new();
    let mut prev_empty = false;
    for line in text.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            if !prev_empty {
                cleaned.push('\n');
                prev_empty = true;
            }
        } else {
            cleaned.push_str(&trimmed);
            cleaned.push('\n');
            prev_empty = false;
        }
    }
    cleaned.trim().to_string()
}

fn finish_extracted_text(kind: &str, text: String, empty_message: &str) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(empty_message.to_string());
    }

    if text.len() > 524288 {
        let safe_end = text.floor_char_boundary(524288);
        Ok(format!(
            "{}\n\n... [{} 内容已截断，仅显示前 512KB]",
            &text[..safe_end],
            kind
        ))
    } else {
        Ok(text)
    }
}

/// 从 PDF 文件中提取纯文本内容
fn read_pdf_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 文件失败: {e}"))?;

    // 使用 catch_unwind 防止畸形 PDF 导致进程崩溃
    let text = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes))
        .map_err(|_| "PDF 解析过程中发生异常，文件可能已损坏".to_string())?
        .map_err(|e| format!("解析 PDF 文件失败: {e}"))?;

    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("PDF 文件中未提取到文本内容（可能是扫描版/图片 PDF，不支持 OCR）".to_string());
    }

    if text.len() > 524288 {
        let safe_end = text.floor_char_boundary(524288);
        Ok(format!(
            "{}\n\n... [PDF 内容已截断，仅显示前 512KB]",
            &text[..safe_end]
        ))
    } else {
        Ok(text)
    }
}

async fn exec_write_file(args: &serde_json::Value) -> Result<String, String> {
    let path = args["path"].as_str().ok_or("缺少 'path' 参数")?;
    let content = args["content"].as_str().ok_or("缺少 'content' 参数")?;
    const MAX_WRITE_BYTES: usize = 512 * 1024;
    if content.len() > MAX_WRITE_BYTES {
        return Err(format!(
            "写入内容过大：{} 字节（上限 {} 字节 / 512KB）",
            content.len(),
            MAX_WRITE_BYTES
        ));
    }
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

pub async fn web_search(query: &str, preferred_provider: &str) -> Result<String, String> {
    let options = SearchOptions::new(query);
    web_search_with_options(&options, preferred_provider).await
}

async fn web_search_with_options(
    options: &SearchOptions,
    preferred_provider: &str,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| e.to_string())?;

    let providers = provider_order(preferred_provider);
    let mut failures = Vec::new();
    let mut source_names = Vec::new();
    let mut all_results = Vec::new();
    let effective_query = options.effective_query();

    for provider in providers {
        if source_names.len() >= 2 || all_results.len() >= SEARCH_CANDIDATE_LIMIT {
            break;
        }
        match search_with_provider(&client, provider, &effective_query).await {
            Ok(results) if !results.is_empty() => {
                source_names.push(provider.name());
                for result in results {
                    push_unique_search_result(&mut all_results, result);
                }
            }
            Ok(_) => failures.push(format!("{} 未找到结果", provider.name())),
            Err(e) => failures.push(e),
        }
    }

    let results = rank_search_results(
        &options.query,
        all_results,
        &options.exclude_domains,
        options.site.as_deref(),
        options.recency_days,
    );
    if !results.is_empty() {
        let limit = options.max_results.min(results.len());
        return Ok(format_search_results(
            &source_names.join(" + "),
            options,
            &results[..limit],
        ));
    }

    Err(format!(
        "所有搜索源都未返回可用结果：{}。建议稍后重试，或提供一个可直接访问的数据源链接。",
        failures.join("；")
    ))
}

/// 读取网页正文内容
async fn read_webpage(url: &str, focus_query: Option<&str>) -> Result<String, String> {
    // 验证 URL 格式
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL 必须以 http:// 或 https:// 开头".to_string());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let res = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.7")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = res.status();
    if !status.is_success() {
        return Err(format!("HTTP 错误: {}", status));
    }

    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    // 检查是否为 HTML 内容
    if !content_type.contains("text/html") && !content_type.contains("text/plain") {
        return Err(format!(
            "不支持的内容类型: {}。此工具仅支持 HTML 和纯文本网页。",
            content_type
        ));
    }

    let body = res.text().await.map_err(|e| format!("读取响应失败: {e}"))?;

    let text = compact_webpage_text(&html_to_text(&body));
    Ok(format_webpage_result(url, &text, focus_query))
}

/// 将 HTML 转换为纯文本
fn html_to_text(html: &str) -> String {
    let mut result = String::new();
    let mut in_script = false;
    let mut in_style = false;
    let mut in_tag = false;
    let mut last_was_space = false;
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '<' {
            // 检查是否是 script 或 style 标签
            let remaining: String = chars[i..].iter().take(20).collect();
            let lower = remaining.to_lowercase();
            if lower.starts_with("<script") {
                in_script = true;
            } else if lower.starts_with("</script") {
                in_script = false;
                // 跳过 </script> 标签
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                i += 1;
                continue;
            } else if lower.starts_with("<style") {
                in_style = true;
            } else if lower.starts_with("</style") {
                in_style = false;
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            in_tag = true;
            i += 1;
            continue;
        }

        if c == '>' {
            in_tag = false;
            // 在某些标签后添加换行
            if i > 0 {
                let prev_tag = chars[..i].iter().rev().take(20).collect::<String>();
                let prev_lower = prev_tag.to_lowercase();
                if prev_lower.contains("/p")
                    || prev_lower.contains("/div")
                    || prev_lower.contains("/li")
                    || prev_lower.contains("/tr")
                    || prev_lower.contains("/h1")
                    || prev_lower.contains("/h2")
                    || prev_lower.contains("/h3")
                    || prev_lower.contains("/h4")
                    || prev_lower.contains("/h5")
                    || prev_lower.contains("/h6")
                    || prev_lower.contains("br")
                {
                    if !last_was_space {
                        result.push('\n');
                        last_was_space = true;
                    }
                }
            }
            i += 1;
            continue;
        }

        if in_tag || in_script || in_style {
            i += 1;
            continue;
        }

        // 处理 HTML 实体
        if c == '&' {
            let remaining: String = chars[i..].iter().take(10).collect();
            if let Some(end) = remaining.find(';') {
                let entity = &remaining[..=end];
                let decoded = match entity.to_lowercase().as_str() {
                    "&nbsp;" => " ",
                    "&lt;" => "<",
                    "&gt;" => ">",
                    "&amp;" => "&",
                    "&quot;" => "\"",
                    "&apos;" => "'",
                    "&#39;" => "'",
                    _ => entity,
                };
                result.push_str(decoded);
                i += entity.len();
                last_was_space = decoded == " ";
                continue;
            }
        }

        // 处理空白字符
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }

        i += 1;
    }

    // 清理多余的空行
    let mut cleaned = String::new();
    let mut prev_empty = false;
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_empty {
                cleaned.push('\n');
                prev_empty = true;
            }
        } else {
            cleaned.push_str(trimmed);
            cleaned.push('\n');
            prev_empty = false;
        }
    }

    cleaned.trim().to_string()
}

fn compact_webpage_text(text: &str) -> String {
    let mut seen = HashSet::new();
    let mut lines = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        let line = line.trim();
        if line.is_empty() || is_boilerplate_line(line) {
            continue;
        }
        let fingerprint = line.to_lowercase();
        if !seen.insert(fingerprint) {
            continue;
        }
        lines.push(truncate_chars(line, 1_200));
    }

    lines.join("\n")
}

fn is_boilerplate_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    if line.chars().count() <= 1 {
        return true;
    }
    matches!(
        lower.as_str(),
        "home"
            | "menu"
            | "login"
            | "sign in"
            | "sign up"
            | "subscribe"
            | "privacy policy"
            | "terms of service"
            | "cookie policy"
            | "联系我们"
            | "关于我们"
            | "登录"
            | "注册"
            | "订阅"
            | "隐私政策"
            | "服务条款"
    ) || lower.contains("enable javascript")
        || lower.contains("accept cookies")
        || lower.contains("all rights reserved")
        || lower.contains("版权所有")
}

fn format_webpage_result(url: &str, text: &str, focus_query: Option<&str>) -> String {
    if let Some(query) = focus_query.map(str::trim).filter(|value| !value.is_empty()) {
        if let Some(excerpts) = relevant_webpage_excerpts(text, query, MAX_WEBPAGE_CHARS) {
            return format!(
                "来源: {url}\n聚焦问题: {query}\n提示: 以下为按问题筛出的相关正文片段，不是全文。\n\n{excerpts}"
            );
        }
    }

    let truncated = truncate_chars(text, MAX_WEBPAGE_CHARS);
    if truncated.chars().count() < text.chars().count() {
        format!(
            "来源: {url}\n\n{truncated}\n\n... [内容已压缩截断，仅显示前 {MAX_WEBPAGE_CHARS} 字符]"
        )
    } else {
        format!("来源: {url}\n\n{truncated}")
    }
}

fn relevant_webpage_excerpts(text: &str, query: &str, max_chars: usize) -> Option<String> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return None;
    }

    let mut scored = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            if line.chars().count() < 20 {
                return None;
            }
            let score = score_text_for_terms(line, query, &terms);
            (score > 0).then_some((score, index, line.to_string()))
        })
        .collect::<Vec<_>>();

    if scored.is_empty() {
        return None;
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(MAX_WEBPAGE_EXCERPTS);
    scored.sort_by_key(|(_, index, _)| *index);

    let mut output = String::new();
    for (excerpt_index, (_, _, line)) in scored.into_iter().enumerate() {
        let block = format!(
            "[片段 {}]\n{}\n\n",
            excerpt_index + 1,
            truncate_chars(&line, 1_500)
        );
        if output.chars().count() + block.chars().count() > max_chars {
            break;
        }
        output.push_str(&block);
    }

    if output.trim().is_empty() {
        None
    } else {
        Some(output.trim().to_string())
    }
}

fn score_text_for_terms(text: &str, query: &str, terms: &[String]) -> i32 {
    let lower = text.to_lowercase();
    let normalized_query = normalize_query_phrase(query);
    let mut score = 0;

    if !normalized_query.is_empty() && lower.contains(&normalized_query) {
        score += 20;
    }
    for term in terms {
        if lower.contains(term) {
            score += 4;
        }
    }
    if text.chars().count() > 1_500 {
        score -= 2;
    }
    score
}

async fn search_with_provider(
    client: &reqwest::Client,
    provider: SearchProvider,
    query: &str,
) -> Result<Vec<SearchResult>, String> {
    let mut req = client
        .get(provider.url())
        .header("Accept", "text/html,application/rss+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.7")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");

    req = match provider {
        SearchProvider::DuckDuckGoHtml => req.query(&[("q", query)]),
        SearchProvider::BingRss => req.query(&[("format", "rss"), ("q", query)]),
        SearchProvider::BingHtml => req.query(&[("q", query)]),
    };

    let res = req
        .send()
        .await
        .map_err(|e| format!("{} 请求失败: {e}", provider.name()))?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("{} 返回 HTTP {}", provider.name(), status));
    }

    let body = res
        .text()
        .await
        .map_err(|e| format!("读取 {} 响应失败: {e}", provider.name()))?;

    if provider == SearchProvider::DuckDuckGoHtml
        && (body.contains("ddg-captcha") || body.contains("anomaly-modal"))
    {
        return Err("DuckDuckGo 触发反爬验证码".to_string());
    }

    let mut results = match provider {
        SearchProvider::DuckDuckGoHtml => parse_duckduckgo_html(&body),
        SearchProvider::BingRss => parse_bing_rss(&body),
        SearchProvider::BingHtml => parse_bing_html(&body),
    };
    results.truncate(SEARCH_CANDIDATE_LIMIT);
    Ok(results)
}

fn format_search_results(
    provider_name: &str,
    options: &SearchOptions,
    results: &[SearchResult],
) -> String {
    let source = if provider_name.trim().is_empty() {
        "未知".to_string()
    } else {
        provider_name.to_string()
    };
    let mut output = format!(
        "搜索来源: {source}\n查询: {}\n提示: 结果已去重并按本地相关性重排。优先只读取标题/摘要直接匹配任务的 1-2 个网页；如果结果偏题，请换更具体的查询词或使用 site 限定。\n",
        options.effective_query()
    );
    for (index, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "\n[{}] 标题: {}\n链接: {}\n摘要: {}\n",
            index + 1,
            result.title,
            result.url,
            result.snippet
        ));
    }
    output
}

fn push_unique_search_result(results: &mut Vec<SearchResult>, result: SearchResult) {
    let normalized = comparable_url(&result.url);
    if results
        .iter()
        .any(|item| comparable_url(&item.url) == normalized)
    {
        return;
    }
    results.push(result);
}

fn rank_search_results(
    query: &str,
    results: Vec<SearchResult>,
    exclude_domains: &[String],
    site: Option<&str>,
    recency_days: Option<u32>,
) -> Vec<SearchResult> {
    let terms = query_terms(query);
    let site = site.and_then(normalize_domain_filter);
    let today = Local::now().date_naive();
    let mut scored = Vec::new();

    for (index, result) in results.into_iter().enumerate() {
        if !is_usable_search_result(&result) {
            continue;
        }
        let domain = result_domain(&result.url);
        if let Some(domain) = domain.as_deref() {
            if exclude_domains
                .iter()
                .any(|excluded| domain_matches_filter(domain, excluded))
            {
                continue;
            }
            if site
                .as_deref()
                .is_some_and(|site| !domain_matches_filter(domain, site))
            {
                continue;
            }
        }

        let score = score_search_result(&result, query, &terms, recency_days, today);
        scored.push((score, index, compact_search_result(result)));
    }

    let positive_count = scored.iter().filter(|(score, _, _)| *score > 0).count();
    if positive_count >= 3 {
        scored.retain(|(score, _, _)| *score > 0);
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, result)| result).collect()
}

fn score_search_result(
    result: &SearchResult,
    query: &str,
    terms: &[String],
    recency_days: Option<u32>,
    today: NaiveDate,
) -> i32 {
    let title = result.title.to_lowercase();
    let snippet = result.snippet.to_lowercase();
    let url = result.url.to_lowercase();
    let domain = result_domain(&result.url).unwrap_or_default();
    let normalized_query = normalize_query_phrase(query);

    let mut score = 0;
    if !normalized_query.is_empty() {
        if title.contains(&normalized_query) {
            score += 28;
        }
        if snippet.contains(&normalized_query) {
            score += 16;
        }
        if url.contains(&normalized_query.replace(' ', "-")) {
            score += 8;
        }
    }

    for term in terms {
        if title.contains(term) {
            score += 10;
        }
        if snippet.contains(term) {
            score += 5;
        }
        if domain.contains(term) {
            score += 4;
        } else if url.contains(term) {
            score += 2;
        }
    }

    if result.snippet == "无描述" {
        score -= 4;
    }
    if looks_like_search_or_listing_page(&result.url, &result.title) {
        score -= 10;
    }
    if is_low_signal_domain(&domain) {
        score -= 4;
    }
    score += score_result_freshness(result, recency_days, today);

    score
}

fn score_result_freshness(
    result: &SearchResult,
    recency_days: Option<u32>,
    today: NaiveDate,
) -> i32 {
    let Some(days) = recency_days else {
        return 0;
    };
    let text = format!("{} {} {}", result.title, result.snippet, result.url).to_lowercase();
    let current_year = today.format("%Y").to_string();
    let today_iso = today.format("%Y-%m-%d").to_string();

    if text.contains("小时前")
        || text.contains("分钟前")
        || text.contains("just now")
        || text.contains("hours ago")
        || text.contains("minutes ago")
        || text.contains(&today_iso)
    {
        return 28;
    }

    let mut score = 0;
    if text.contains(&current_year) {
        score += if days <= 7 { 12 } else { 8 };
    }

    let current_year_num = today.format("%Y").to_string().parse::<i32>().unwrap_or(0);
    for year in (current_year_num.saturating_sub(10))..current_year_num {
        if text.contains(&year.to_string()) {
            score -= if days <= 31 {
                20
            } else if days <= 365 {
                10
            } else {
                4
            };
        }
    }

    score
}

fn compact_search_result(result: SearchResult) -> SearchResult {
    SearchResult {
        title: compact_line(&result.title, 120),
        url: result.url,
        snippet: compact_line(&result.snippet, MAX_SEARCH_SNIPPET_CHARS),
    }
}

fn compact_line(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&compact, max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn normalize_query_phrase(query: &str) -> String {
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|part| !part.starts_with("site:"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn query_terms(query: &str) -> Vec<String> {
    let mut raw_terms = Vec::new();
    let mut current = String::new();
    let mut current_kind: Option<TermKind> = None;

    for ch in query.to_lowercase().chars() {
        let kind = if ch.is_ascii_alphanumeric() {
            Some(TermKind::Ascii)
        } else if is_cjk(ch) {
            Some(TermKind::Cjk)
        } else {
            None
        };

        if kind.is_none() || (current_kind.is_some() && current_kind != kind) {
            push_raw_term(&mut raw_terms, &mut current);
        }
        if let Some(kind) = kind {
            current.push(ch);
            current_kind = Some(kind);
        } else {
            current_kind = None;
        }
    }
    push_raw_term(&mut raw_terms, &mut current);

    let mut terms = Vec::new();
    for term in raw_terms {
        if should_skip_query_term(&term) {
            continue;
        }
        if term.chars().all(is_cjk) {
            push_unique_term(&mut terms, &term);
            add_cjk_windows(&mut terms, &term);
        } else {
            push_unique_term(&mut terms, &term);
        }
        if terms.len() >= 24 {
            break;
        }
    }
    terms
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TermKind {
    Ascii,
    Cjk,
}

fn push_raw_term(terms: &mut Vec<String>, current: &mut String) {
    let term = current.trim().to_string();
    if !term.is_empty() {
        terms.push(term);
    }
    current.clear();
}

fn push_unique_term(terms: &mut Vec<String>, term: &str) {
    if !terms.iter().any(|item| item == term) {
        terms.push(term.to_string());
    }
}

fn add_cjk_windows(terms: &mut Vec<String>, term: &str) {
    let chars = term.chars().collect::<Vec<_>>();
    if chars.len() <= 2 {
        return;
    }
    for window in chars.windows(2) {
        let item = window.iter().collect::<String>();
        if !should_skip_query_term(&item) {
            push_unique_term(terms, &item);
        }
    }
}

fn should_skip_query_term(term: &str) -> bool {
    let term = term.trim();
    if term.is_empty() || term.starts_with("site") || term.starts_with("http") {
        return true;
    }
    if term.is_ascii() && term.len() < 2 {
        return true;
    }
    matches!(
        term,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "what"
            | "when"
            | "where"
            | "how"
            | "is"
            | "are"
            | "was"
            | "were"
            | "一个"
            | "这个"
            | "那个"
            | "怎么"
            | "如何"
            | "什么"
            | "以及"
            | "或者"
            | "是否"
    )
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
    )
}

fn is_usable_search_result(result: &SearchResult) -> bool {
    let Ok(parsed) = url::Url::parse(&result.url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str().map(normalize_host) else {
        return false;
    };
    if matches!(
        host.as_str(),
        "bing.com" | "cn.bing.com" | "duckduckgo.com" | "html.duckduckgo.com"
    ) {
        return false;
    }
    let title = result.title.to_lowercase();
    !(title.contains("captcha") || title.contains("robot check"))
}

fn looks_like_search_or_listing_page(url: &str, title: &str) -> bool {
    let lower_url = url.to_lowercase();
    let lower_title = title.to_lowercase();
    lower_url.contains("/search?")
        || lower_url.contains("?q=")
        || lower_url.contains("/tag/")
        || lower_url.contains("/tags/")
        || lower_title == "search"
        || lower_title.ends_with(" search results")
        || lower_title.contains("搜索结果")
}

fn is_low_signal_domain(domain: &str) -> bool {
    [
        "pinterest.com",
        "facebook.com",
        "x.com",
        "twitter.com",
        "instagram.com",
        "tiktok.com",
    ]
    .iter()
    .any(|blocked| domain_matches_filter(domain, blocked))
}

fn result_domain(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(normalize_host))
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_start_matches("www.").to_ascii_lowercase()
}

fn normalize_domain_filter(value: &str) -> Option<String> {
    let mut value = value.trim().to_ascii_lowercase();
    value = value
        .trim_start_matches("site:")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .to_string();
    let domain = value
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('.');
    if domain.is_empty() || domain.contains(char::is_whitespace) {
        None
    } else {
        Some(domain.to_string())
    }
}

fn domain_matches_filter(domain: &str, filter: &str) -> bool {
    domain == filter || domain.ends_with(&format!(".{filter}"))
}

fn comparable_url(url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else {
        return url.trim().to_lowercase();
    };
    parsed.set_fragment(None);
    let tracking_keys = [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
    ];
    let query_pairs = parsed
        .query_pairs()
        .filter(|(key, _)| {
            !tracking_keys
                .iter()
                .any(|tracking| key.as_ref() == *tracking)
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    if !query_pairs.is_empty() {
        let mut pairs = parsed.query_pairs_mut();
        for (key, value) in query_pairs {
            pairs.append_pair(&key, &value);
        }
    }
    parsed.as_str().trim_end_matches('/').to_lowercase()
}

fn parse_duckduckgo_html(html: &str) -> Vec<SearchResult> {
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

        let title = clean_html_text(raw_title);
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
                        clean_html_text(raw_snippet)
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

        push_search_result(&mut results, title, url, snippet);

        search_pos = next_snippet_start;
        if results.len() >= SEARCH_CANDIDATE_LIMIT {
            break;
        }
    }

    results
}

fn parse_bing_rss(xml: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut search_pos = 0;

    while let Some(item_start_offset) = xml[search_pos..].find("<item>") {
        let item_start = search_pos + item_start_offset + "<item>".len();
        let Some(item_end_offset) = xml[item_start..].find("</item>") else {
            break;
        };
        let item_end = item_start + item_end_offset;
        let item = &xml[item_start..item_end];

        if let (Some(title), Some(url)) = (
            extract_tag_text(item, "title"),
            extract_tag_text(item, "link"),
        ) {
            let snippet =
                extract_tag_text(item, "description").unwrap_or_else(|| "无描述".to_string());
            push_search_result(&mut results, title, url, snippet);
        }

        search_pos = item_end + "</item>".len();
        if results.len() >= SEARCH_CANDIDATE_LIMIT {
            break;
        }
    }

    results
}

fn parse_bing_html(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut search_pos = 0;

    while let Some(item_start_offset) = html[search_pos..].find("<li class=\"b_algo\"") {
        let item_start = search_pos + item_start_offset;
        let item_end = html[item_start + 1..]
            .find("<li class=\"b_algo\"")
            .map(|offset| item_start + 1 + offset)
            .unwrap_or(html.len());
        let item = &html[item_start..item_end];

        let Some(h2_start) = item.find("<h2") else {
            search_pos = item_end;
            continue;
        };
        let h2 = &item[h2_start..];
        let Some((title, url)) = extract_first_anchor(h2) else {
            search_pos = item_end;
            continue;
        };

        let snippet = extract_bing_snippet(item).unwrap_or_else(|| "无描述".to_string());
        push_search_result(&mut results, title, url, snippet);

        search_pos = item_end;
        if results.len() >= SEARCH_CANDIDATE_LIMIT {
            break;
        }
    }

    results
}

fn push_search_result(
    results: &mut Vec<SearchResult>,
    title: String,
    url: String,
    snippet: String,
) {
    let title = title.trim().to_string();
    let url = normalize_result_url(url.trim());
    let snippet = snippet.trim().to_string();

    if title.is_empty() || url.is_empty() || results.iter().any(|item| item.url == url) {
        return;
    }

    results.push(SearchResult {
        title,
        url,
        snippet: if snippet.is_empty() {
            "无描述".to_string()
        } else {
            snippet
        },
    });
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

    decode_html_entities(&result)
}

fn clean_html_text(s: &str) -> String {
    strip_html_tags(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
        .replace("&#160;", " ")
        .replace("&nbsp;", " ")
}

fn extract_tag_text(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close).map(|offset| start + offset)?;
    Some(clean_html_text(&s[start..end]))
}

fn extract_first_anchor(s: &str) -> Option<(String, String)> {
    let anchor_start = s.find("<a ")?;
    let tag_end = s[anchor_start..]
        .find('>')
        .map(|offset| anchor_start + offset)?;
    let tag = &s[anchor_start..=tag_end];
    let raw_url = extract_attr(tag, "href")?;
    let content_start = tag_end + 1;
    let content_end = s[content_start..]
        .find("</a>")
        .map(|offset| content_start + offset)?;
    let title = clean_html_text(&s[content_start..content_end]);
    Some((title, raw_url))
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"').map(|offset| start + offset)?;
    Some(decode_html_entities(&tag[start..end]))
}

fn extract_bing_snippet(item: &str) -> Option<String> {
    let caption_start = item.find("b_caption")?;
    let caption = &item[caption_start..];
    let p_start = caption.find("<p")?;
    let p = &caption[p_start..];
    let content_start = p.find('>')? + 1;
    let content_end = p[content_start..]
        .find("</p>")
        .map(|offset| content_start + offset)?;
    Some(clean_html_text(&p[content_start..content_end]))
}

fn normalize_result_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        decode_html_entities(url)
    }
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
        decode_html_entities(href)
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

const WEATHER_CONFIG_KEY_FOR_TOOLS: &str = "weather_config";

fn unix_now_for_tools() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn load_weather_config_for_tools() -> Result<system::weather::WeatherConfig, String> {
    let db_path = get_db_path_for_tools()?;
    let conn =
        rusqlite::Connection::open(&db_path).map_err(|e| format!("打开天气设置数据库失败: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT value FROM pet_settings WHERE key = ?1")
        .map_err(|e| format!("读取天气设置失败: {e}"))?;
    let mut rows = stmt
        .query_map([WEATHER_CONFIG_KEY_FOR_TOOLS], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| format!("读取天气设置失败: {e}"))?;
    let raw = match rows.next() {
        Some(Ok(value)) => Some(value),
        Some(Err(error)) => return Err(format!("读取天气设置失败: {error}")),
        None => None,
    };

    let config = raw
        .and_then(|value| serde_json::from_str::<system::weather::WeatherConfig>(&value).ok())
        .map(system::weather::normalize_weather_config)
        .unwrap_or_default();
    Ok(config)
}

async fn exec_get_weather(args: &serde_json::Value) -> Result<String, String> {
    let mut config = load_weather_config_for_tools()?;
    if let Some(location) = args["location"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        config.location = location.to_string();
        config.enabled = true;
    }
    if !config.enabled {
        return Err("天气功能已关闭，请先在设置里启用天气栏与每日天气问候。".to_string());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("创建天气请求客户端失败: {e}"))?;
    let weather = system::weather::get_weather_with_client(&client, &config).await?;
    Ok(system::weather::format_weather_message(&weather))
}

fn normalize_task_repeat_for_tools(value: &str) -> &'static str {
    match value.trim().to_lowercase().as_str() {
        "daily" | "day" | "每天" => "daily",
        "weekly" | "week" | "每周" => "weekly",
        "monthly" | "month" | "每月" => "monthly",
        _ => "once",
    }
}

fn ensure_scheduled_tasks_table(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            note TEXT NOT NULL DEFAULT '',
            due_at INTEGER NOT NULL,
            repeat TEXT NOT NULL DEFAULT 'once',
            enabled INTEGER NOT NULL DEFAULT 1,
            last_triggered_at INTEGER,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );",
    )
    .map_err(|e| format!("初始化定时任务表失败: {e}"))
}

fn due_at_from_tool_args(args: &serde_json::Value) -> Result<i64, String> {
    let now = unix_now_for_tools();
    if let Some(due_at) = args["due_at"].as_i64() {
        return Ok(due_at);
    }

    let relative_seconds = args["delay_minutes"]
        .as_f64()
        .map(|value| value * 60.0)
        .or_else(|| args["delay_hours"].as_f64().map(|value| value * 3600.0))
        .or_else(|| args["delay_days"].as_f64().map(|value| value * 86_400.0));

    match relative_seconds {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            Ok(now.saturating_add(seconds.round() as i64))
        }
        _ => Err(
            "缺少有效提醒时间，请提供 due_at 或 delay_minutes/delay_hours/delay_days".to_string(),
        ),
    }
}

async fn exec_create_scheduled_task(args: &serde_json::Value) -> Result<String, String> {
    let title = args["title"].as_str().unwrap_or("").trim().to_string();
    if title.is_empty() {
        return Err("缺少任务标题 title".to_string());
    }

    let note = args["note"].as_str().unwrap_or("").trim().to_string();
    let due_at = due_at_from_tool_args(args)?;
    if due_at < unix_now_for_tools() + 5 {
        return Err("提醒时间必须晚于当前时间至少 5 秒".to_string());
    }
    let repeat = normalize_task_repeat_for_tools(args["repeat"].as_str().unwrap_or("once"));

    let db_path = get_db_path_for_tools()?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    ensure_scheduled_tasks_table(&conn)?;
    conn.execute(
        "INSERT INTO scheduled_tasks (title, note, due_at, repeat, enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, 1, strftime('%s', 'now'))",
        (&title, &note, due_at, repeat),
    )
    .map_err(|e| format!("保存定时任务失败: {e}"))?;

    let id = conn.last_insert_rowid();
    Ok(format!(
        "定时任务已创建: #{} {}，首次提醒 Unix 秒: {}，重复: {}",
        id, title, due_at, repeat
    ))
}

async fn exec_list_scheduled_tasks() -> Result<String, String> {
    let db_path = get_db_path_for_tools()?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;
    ensure_scheduled_tasks_table(&conn)?;

    let mut stmt = conn
        .prepare(
            "SELECT id, title, note, due_at, repeat, enabled, last_triggered_at
             FROM scheduled_tasks
             ORDER BY enabled DESC, due_at ASC, id DESC
             LIMIT 50",
        )
        .map_err(|e| format!("读取定时任务失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)? != 0,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .map_err(|e| format!("读取定时任务失败: {e}"))?;

    let mut lines = Vec::new();
    for row in rows {
        let (id, title, note, due_at, repeat, enabled, last_triggered_at) =
            row.map_err(|e| format!("读取定时任务失败: {e}"))?;
        lines.push(format!(
            "#{} [{}] {} | due_at={} | repeat={} | last={}",
            id,
            if enabled { "启用" } else { "停用" },
            if note.trim().is_empty() {
                title
            } else {
                format!("{title} - {note}")
            },
            due_at,
            repeat,
            last_triggered_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "未触发".to_string())
        ));
    }

    if lines.is_empty() {
        Ok("当前没有定时任务。".to_string())
    } else {
        Ok(lines.join("\n"))
    }
}

async fn exec_search_memory(args: &serde_json::Value) -> Result<String, String> {
    let query = args["query"].as_str().ok_or("缺少 'query' 参数")?;
    let category = args["category"].as_str();
    let limit = args["limit"].as_u64().unwrap_or(10) as u32;

    let db_path = get_db_path_for_tools()?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;

    let like_pattern = format!("%{}%", escape_like(query));
    let limit = limit.min(30);

    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(cat) = category {
        (
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 OR value LIKE ?1) AND category = ?2
             ORDER BY id DESC LIMIT ?3",
            vec![
                Box::new(like_pattern),
                Box::new(cat.to_string()),
                Box::new(limit),
            ],
        )
    } else {
        (
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 OR value LIKE ?1)
             ORDER BY id DESC LIMIT ?3",
            vec![Box::new(like_pattern), Box::new(limit)],
        )
    };

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("查询记忆失败: {e}"))?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| format!("查询记忆失败: {e}"))?;

    let mut lines = Vec::new();
    for row in rows {
        let (id, category, key, value, created_at) =
            row.map_err(|e| format!("读取记忆失败: {e}"))?;
        let val_display = if value.len() > 120 {
            format!("{}...", &value[..120])
        } else {
            value
        };
        lines.push(format!(
            "#{} [{}] {} | {} | 创建: {}",
            id, category, key, val_display, created_at
        ));
    }

    if lines.is_empty() {
        Ok(format!("未找到与 \"{}\" 相关的记忆。", query))
    } else {
        Ok(lines.join("\n"))
    }
}

async fn exec_save_memory(args: &serde_json::Value) -> Result<String, String> {
    let category = args["category"].as_str().ok_or("缺少 'category' 参数")?;
    let key = args["key"].as_str().ok_or("缺少 'key' 参数")?;
    let value = args["value"].as_str().ok_or("缺少 'value' 参数")?;

    let db_path = get_db_path_for_tools()?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;

    conn.execute(
        "INSERT INTO pet_memory (category, key, value) VALUES (?1, ?2, ?3)",
        rusqlite::params![category, key, value],
    )
    .map_err(|e| format!("保存记忆失败: {e}"))?;

    let id = conn.last_insert_rowid();
    Ok(format!("记忆已保存: #{} [{}] {}", id, category, key))
}

async fn exec_delete_memory(args: &serde_json::Value) -> Result<String, String> {
    let id = args["id"].as_i64().ok_or("缺少 'id' 参数")?;

    let db_path = get_db_path_for_tools()?;
    let conn = rusqlite::Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {e}"))?;

    let affected = conn
        .execute("DELETE FROM pet_memory WHERE id = ?1", [id])
        .map_err(|e| format!("删除记忆失败: {e}"))?;

    if affected == 0 {
        Ok(format!("未找到 ID 为 {} 的记忆，可能已被删除。", id))
    } else {
        Ok(format!("记忆 #{} 已删除。", id))
    }
}

async fn exec_open_app(args: &serde_json::Value) -> Result<String, String> {
    let app = args["app"].as_str().ok_or("缺少 'app' 参数")?;
    let app_args = args["args"].as_str().unwrap_or("");
    let mut resolved = resolve_app_name(app);

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

    // 拒绝 shell 元字符：避免通过 AI 控制的 resolved 路径触发命令注入
    if resolved
        .chars()
        .any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | ';' | '`' | '$' | '\n' | '\r' | '"'))
    {
        return Err(format!("解析出的应用路径包含不允许的字符: {resolved}"));
    }
    if app_args
        .chars()
        .any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | ';' | '`' | '$' | '\n' | '\r' | '"'))
    {
        return Err("args 包含不允许的 shell 元字符".to_string());
    }
    let resolved_path = std::path::Path::new(&resolved);
    if !resolved_path.exists() {
        return Err(format!("应用路径不存在: {resolved}"));
    }

    // 解析额外参数，按空格切分但保留双引号包裹（仅最基础切分）
    let arg_list: Vec<String> = if app_args.trim().is_empty() {
        Vec::new()
    } else {
        app_args
            .split_whitespace()
            .map(|s| s.trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    // 直接以 resolved 路径作为可执行文件启动，避免 cmd.exe 解析器介入
    let mut cmd = tokio::process::Command::new(&resolved);
    for arg in &arg_list {
        cmd.arg(arg);
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

/// 递归搜索匹配 glob 模式的文件
async fn exec_file_search(args: &serde_json::Value) -> Result<String, String> {
    let pattern = args["pattern"].as_str().ok_or("缺少 'pattern' 参数")?;
    let directory = args["directory"].as_str();
    let max_results = args["max_results"]
        .as_u64()
        .map(|v| v.min(200) as usize)
        .unwrap_or(50);

    if pattern.trim().is_empty() {
        return Err("搜索模式不能为空".to_string());
    }

    let allowed_roots = canonical_allowed_roots();
    let glob_pattern = pattern.to_lowercase();

    // 确定搜索起始目录
    let search_dirs: Vec<PathBuf> = if let Some(dir) = directory {
        let canonical = is_path_allowed(dir)?;
        if !canonical.is_dir() {
            return Err(format!("路径不是目录: {dir}"));
        }
        vec![canonical]
    } else {
        allowed_roots.clone()
    };

    let mut results: Vec<(PathBuf, u64)> = Vec::new();

    for root in &search_dirs {
        if results.len() >= max_results {
            break;
        }
        // walkdir 递归搜索，最大深度 10 层
        let walker = walkdir::WalkDir::new(root)
            .max_depth(10)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // 跳过隐藏目录和系统目录
                let name = e.file_name().to_string_lossy();
                !name.starts_with('.') && name != "node_modules" && name != "__pycache__"
            });

        for entry in walker.flatten() {
            if results.len() >= max_results {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_lowercase();
            if glob_match(&glob_pattern, &file_name) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                results.push((entry.path().to_path_buf(), size));
            }
        }
    }

    if results.is_empty() {
        let scope = directory.unwrap_or("所有允许目录");
        return Ok(format!("在 {} 中未找到匹配 '{}' 的文件", scope, pattern));
    }

    let mut output = format!("找到 {} 个匹配 '{}' 的文件:\n", results.len(), pattern);
    for (i, (path, size)) in results.iter().enumerate() {
        let size_str = format_file_size(*size);
        output.push_str(&format!("[{}] {} ({})", i + 1, path.display(), size_str));
        output.push('\n');
    }
    if results.len() >= max_results {
        output.push_str(&format!("\n... [已达到最大结果数 {}]", max_results));
    }
    Ok(output)
}

/// 简单的 glob 匹配（支持 * 和 ? 通配符）
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt, 0, 0)
}

fn glob_match_inner(pattern: &[char], text: &[char], mut pi: usize, mut ti: usize) -> bool {
    let mut star_pi: Option<usize> = None;
    let mut star_ti: Option<usize> = None;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = Some(pi);
            star_ti = Some(ti);
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            let st = star_ti.unwrap() + 1;
            star_ti = Some(st);
            ti = st;
        } else {
            return false;
        }
    }
    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }
    pi == pattern.len()
}

/// 格式化文件大小
fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' | '%' | '_' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_name(prefix: &str, ext: &str) -> String {
        format!(
            "{}-{}.{}",
            prefix,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            ext
        )
    }

    #[test]
    fn allows_current_workspace_paths() {
        let path = std::env::current_dir()
            .unwrap()
            .join(unique_test_name("path-policy", "txt"));
        std::fs::write(&path, "ok").unwrap();

        let allowed = is_path_allowed(path.to_str().unwrap()).unwrap();
        let expected = path.canonicalize().unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(allowed, expected);
    }

    #[test]
    fn rejects_sensitive_credential_paths() {
        let dir = std::env::temp_dir().join(unique_test_name("path-policy", "dir"));
        let ssh_dir = dir.join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let path = ssh_dir.join("id_rsa");
        std::fs::write(&path, "secret").unwrap();

        let result = is_path_allowed(path.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);

        assert!(result.is_err());
    }

    #[test]
    fn extracts_open_xml_text_with_entities_and_paragraphs() {
        let xml = r#"
            <p:txBody>
              <a:p><a:r><a:t>第一 &amp; 第二</a:t></a:r></a:p>
              <a:p><a:r><a:t>3 &lt; 4</a:t></a:r><a:br/><a:r><a:t>&#x4E2D;&#25991;</a:t></a:r></a:p>
            </p:txBody>
        "#;

        let text = extract_open_xml_text(xml);

        assert_eq!(text, "第一 & 第二\n3 < 4\n中文");
    }

    #[test]
    fn reads_pptx_slide_and_note_text() {
        use std::io::Write;

        let path = std::env::temp_dir().join(unique_test_name("ai-desktop-pet-test", "pptx"));

        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            zip.start_file("[Content_Types].xml", options).unwrap();
            zip.write_all(br#"<Types></Types>"#).unwrap();
            zip.start_file("ppt/slides/slide2.xml", options).unwrap();
            zip.write_all(
                r#"<p:sld><p:cSld><p:spTree><a:p><a:r><a:t>第二页</a:t></a:r></a:p></p:spTree></p:cSld></p:sld>"#
                    .as_bytes(),
            )
            .unwrap();
            zip.start_file("ppt/slides/slide1.xml", options).unwrap();
            zip.write_all(
                r#"<p:sld><p:cSld><p:spTree><a:p><a:r><a:t>标题 &amp; 要点</a:t></a:r></a:p></p:spTree></p:cSld></p:sld>"#
                    .as_bytes(),
            )
            .unwrap();
            zip.start_file("ppt/notesSlides/notesSlide1.xml", options)
                .unwrap();
            zip.write_all(
                r#"<p:notes><a:p><a:r><a:t>备注内容</a:t></a:r></a:p></p:notes>"#.as_bytes(),
            )
            .unwrap();
            zip.finish().unwrap();
        }

        let text = read_pptx_text(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert!(text.contains("幻灯片 1\n标题 & 要点"));
        assert!(text.contains("幻灯片 2\n第二页"));
        assert!(text.contains("备注 1\n备注内容"));
    }

    #[test]
    fn parses_duckduckgo_result_and_redirect_url() {
        let html = r#"
            <div class="result">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Freports%3Fa%3D1&amp;rut=abc">Example <b>Report</b></a>
              <a class="result__snippet">A short &amp; useful snippet.</a>
            </div>
        "#;

        let results = parse_duckduckgo_html(html);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example Report");
        assert_eq!(results[0].url, "https://example.com/reports?a=1");
        assert_eq!(results[0].snippet, "A short & useful snippet.");
    }

    #[test]
    fn parses_bing_rss_items() {
        let xml = r#"
            <rss><channel>
              <item>
                <title>Result &amp; One</title>
                <link>https://example.com/one</link>
                <description>Snippet with <b>markup</b>.</description>
              </item>
              <item>
                <title>Result Two</title>
                <link>https://example.com/two</link>
                <description>Second snippet.</description>
              </item>
            </channel></rss>
        "#;

        let results = parse_bing_rss(xml);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Result & One");
        assert_eq!(results[0].url, "https://example.com/one");
        assert_eq!(results[0].snippet, "Snippet with markup.");
    }

    #[test]
    fn parses_bing_html_results() {
        let html = r#"
            <ol>
              <li class="b_algo">
                <h2><a target="_blank" href="https://example.com/a">Title <strong>A</strong></a></h2>
                <div class="b_caption"><p>Snippet &amp; details.</p></div>
              </li>
            </ol>
        "#;

        let results = parse_bing_html(html);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Title A");
        assert_eq!(results[0].url, "https://example.com/a");
        assert_eq!(results[0].snippet, "Snippet & details.");
    }

    #[test]
    fn prefers_bing_for_domestic_default_search() {
        assert_eq!(
            provider_order("bing"),
            [
                SearchProvider::BingRss,
                SearchProvider::BingHtml,
                SearchProvider::DuckDuckGoHtml,
            ]
        );
        assert_eq!(
            provider_order("duckduckgo"),
            [
                SearchProvider::DuckDuckGoHtml,
                SearchProvider::BingRss,
                SearchProvider::BingHtml,
            ]
        );
    }

    #[test]
    fn reranks_search_results_by_query_terms_and_filters_search_pages() {
        let results = vec![
            SearchResult {
                title: "Bing search results".to_string(),
                url: "https://cn.bing.com/search?q=deepseek+api".to_string(),
                snippet: "Search page".to_string(),
            },
            SearchResult {
                title: "DeepSeek stock price today".to_string(),
                url: "https://finance.example.com/deepseek-stock".to_string(),
                snippet: "Market quote and unrelated trading news.".to_string(),
            },
            SearchResult {
                title: "DeepSeek API chat completions guide".to_string(),
                url: "https://api-docs.deepseek.com/guides/chat-completions".to_string(),
                snippet: "Use the chat completions endpoint with model deepseek-chat and OpenAI compatible messages.".to_string(),
            },
        ];

        let ranked = rank_search_results("DeepSeek chat completions API", results, &[], None, None);

        assert_eq!(
            ranked.first().map(|item| item.url.as_str()),
            Some("https://api-docs.deepseek.com/guides/chat-completions")
        );
        assert!(!ranked.iter().any(|item| item.url.contains("bing.com")));
    }

    #[test]
    fn rank_search_results_respects_site_and_excluded_domains() {
        let results = vec![
            SearchResult {
                title: "reqwest proxy examples".to_string(),
                url: "https://blog.example.com/reqwest-proxy".to_string(),
                snippet: "A casual blog post about proxy setup.".to_string(),
            },
            SearchResult {
                title: "reqwest Proxy official docs".to_string(),
                url: "https://docs.rs/reqwest/latest/reqwest/struct.Proxy.html".to_string(),
                snippet: "Official Rust documentation for reqwest Proxy configuration.".to_string(),
            },
        ];

        let ranked = rank_search_results(
            "rust reqwest proxy official docs",
            results,
            &["blog.example.com".to_string()],
            Some("docs.rs"),
            None,
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(
            ranked[0].url,
            "https://docs.rs/reqwest/latest/reqwest/struct.Proxy.html"
        );
    }

    #[test]
    fn recent_news_queries_infer_recency_window() {
        assert_eq!(infer_recency_days("帮我整理最近的国际新闻"), Some(7));
        assert_eq!(infer_recency_days("今天国际新闻速览"), Some(2));
        assert_eq!(infer_recency_days("本周 AI 行业新闻"), Some(7));
        assert_eq!(infer_recency_days("DeepSeek API 文档"), None);
    }

    #[test]
    fn effective_query_adds_after_operator_for_recency() {
        let mut options = SearchOptions::new("最近的国际新闻");
        options.recency_days = Some(7);
        let query =
            options.effective_query_for_date(chrono::NaiveDate::from_ymd_opt(2026, 6, 10).unwrap());

        assert!(query.contains("after:2026-06-03"));
    }

    #[test]
    fn freshness_score_penalizes_old_news_for_recent_queries() {
        let old = SearchResult {
            title: "International news roundup 2023".to_string(),
            url: "https://news.example.com/2023/roundup".to_string(),
            snippet: "A 2023 roundup of world events.".to_string(),
        };
        let fresh = SearchResult {
            title: "International news roundup 2026".to_string(),
            url: "https://news.example.com/2026/roundup".to_string(),
            snippet: "Updated 2026 international headlines.".to_string(),
        };

        let today = chrono::NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        assert!(
            score_result_freshness(&fresh, Some(7), today)
                > score_result_freshness(&old, Some(7), today)
        );
    }

    #[test]
    fn focused_webpage_excerpts_keep_relevant_text() {
        let text = compact_webpage_text(
            "Home\nAccept cookies\nA long unrelated paragraph about cooking rice and weekend plans.\nDeepSeek chat completions API supports OpenAI-compatible messages and the deepseek-chat model.\nPrivacy Policy\nAnother unrelated section about account settings.",
        );

        let excerpts =
            relevant_webpage_excerpts(&text, "DeepSeek chat completions API", 800).unwrap();

        assert!(excerpts.contains("DeepSeek chat completions API"));
        assert!(!excerpts.contains("Accept cookies"));
    }

    #[test]
    fn tool_definitions_include_weather_query() {
        let tools = tool_definitions();
        let has_weather = tools.iter().any(|tool| {
            tool["function"]["name"]
                .as_str()
                .is_some_and(|name| name == "get_weather")
        });

        assert!(has_weather);
    }

    #[test]
    fn web_search_tool_accepts_recency_days() {
        let tools = tool_definitions();
        let web_search = tools
            .iter()
            .find(|tool| {
                tool["function"]["name"]
                    .as_str()
                    .is_some_and(|name| name == "web_search")
            })
            .unwrap();

        assert!(web_search["function"]["parameters"]["properties"]["recency_days"].is_object());
    }

    #[test]
    fn read_webpage_tool_accepts_focus_query() {
        let tools = tool_definitions();
        let read_webpage = tools
            .iter()
            .find(|tool| {
                tool["function"]["name"]
                    .as_str()
                    .is_some_and(|name| name == "read_webpage")
            })
            .unwrap();

        assert!(read_webpage["function"]["parameters"]["properties"]["query"].is_object());
    }
}
