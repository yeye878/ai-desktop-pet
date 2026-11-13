use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct DirectApiConfig {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub confirm_enabled: bool,
    pub thinking_depth: String,
    pub execution_mode: String,
    pub search_provider: String,
    pub auto_approved_tools: Vec<String>,
    pub stream_mode: String,
    pub api_mode: String,
}

#[derive(Clone, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UserAttachment {
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub extension: String,
    #[serde(default)]
    pub is_image: bool,
}

impl Default for DirectApiConfig {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "Default API".to_string(),
            api_key: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            confirm_enabled: true,
            thinking_depth: "auto".to_string(),
            execution_mode: "normal".to_string(),
            search_provider: "bing".to_string(),
            auto_approved_tools: Vec::new(),
            stream_mode: "auto".to_string(),
            api_mode: "chat_completions".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub reasoning: Option<Value>,
    pub thinking: Option<Value>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolCallDelta {
    /// 部分 OpenAI 兼容服务（旧版 Ollama、部分 Gemini 兼容层）不下发 index：
    /// 必填会让整块 chunk 反序列化失败而被静默丢弃（连正文一起丢）。
    #[serde(default)]
    pub index: Option<usize>,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub delta: Delta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionChunk {
    pub choices: Vec<Choice>,
}

/// 非流式响应的工具调用
#[derive(Debug, Deserialize, Clone)]
pub struct NonStreamToolCall {
    pub id: Option<String>,
    #[serde(default = "default_tool_type")]
    pub r#type: String,
    pub function: NonStreamFunctionCall,
}

fn default_tool_type() -> String {
    "function".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct NonStreamFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// 非流式响应的解析结果
#[derive(Debug, Default)]
pub struct NonStreamResult {
    pub content: Option<String>,
    pub tool_calls: Vec<NonStreamToolCall>,
    pub finish_reason: Option<String>,
}

/// 跨 chunk 的增量 UTF-8 解码。
///
/// HTTP 分块边界和字符边界无关：一个汉字的 3 个字节可能被拆进相邻两块。对每块单独
/// `from_utf8_lossy` 会把被拆开的字符变成 `�`（实测"你好世界"→"���好世界"）。
/// 这里把块尾不完整的多字节序列留到下一块再解；真正非法的字节才替换成 U+FFFD。
#[derive(Default)]
pub struct Utf8ChunkDecoder {
    pending: Vec<u8>,
}

impl Utf8ChunkDecoder {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    out.push_str(text);
                    self.pending.clear();
                    return out;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    out.push_str(std::str::from_utf8(&self.pending[..valid]).unwrap_or(""));
                    match error.error_len() {
                        // 块尾是被切断的多字节字符：残余字节留给下一块。
                        None => {
                            self.pending.drain(..valid);
                            return out;
                        }
                        Some(len) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid + len);
                        }
                    }
                }
            }
        }
    }

    /// 流结束时冲出残余字节（不完整的字符按替换字符输出）。
    pub fn finish(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

/// 发起流式请求失败：区分能否重试（401 / 400 这类配置错误重试多少次都一样）。
#[derive(Debug)]
pub struct StreamRequestError {
    pub message: String,
    pub retryable: bool,
}

impl std::fmt::Display for StreamRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 只有限流、超时和服务端错误值得重试；4xx 配置类错误（密钥错、参数错、上下文超长）不重试。
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::CONFLICT
        || status.as_u16() == 425
        || status.is_server_error()
}

pub struct SseParser {
    buffer: String,
    current_event: String,
}

const SSE_BUFFER_MAX_BYTES: usize = 4 * 1024 * 1024;

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            current_event: String::new(),
        }
    }

    /// 返回 (event_type, data) 对的列表。
    /// 对于 Chat Completions 格式，event_type 为空字符串。
    /// 对于 Responses API 格式，event_type 为事件类型（如 "response.output_text.delta"）。
    pub fn push(&mut self, text: &str) -> Vec<(String, String)> {
        self.buffer.push_str(text);
        if self.buffer.len() > SSE_BUFFER_MAX_BYTES {
            let keep_from = self.buffer.len() - SSE_BUFFER_MAX_BYTES;
            let safe_start = self.buffer.ceil_char_boundary(keep_from);
            self.buffer.drain(..safe_start);
        }
        let mut results = Vec::new();

        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos].trim().to_string();
            self.buffer.drain(..=newline_pos);

            if line.is_empty() {
                // SSE 事件边界：空行表示事件结束，此时清除 event_type
                self.current_event.clear();
            } else if let Some(event) = line.strip_prefix("event:") {
                self.current_event = event.trim().to_string();
            } else if let Some(data) = line.strip_prefix("data:") {
                let data_str = data.trim().to_string();
                if data_str == "[DONE]" {
                    results.push((String::new(), "[DONE]".to_string()));
                } else if !data_str.is_empty() {
                    results.push((self.current_event.clone(), data_str));
                }
            }
        }

        results
    }
}

pub fn api_error_message_from_value(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    let (raw_message, error_type, code) = match error {
        Value::String(message) => (message.as_str(), None, None),
        Value::Object(map) => {
            let message = map
                .get("message")
                .and_then(|item| item.as_str())
                .unwrap_or("上游 API 返回错误");
            let error_type = map.get("type").and_then(|item| item.as_str());
            let code = map.get("code").and_then(|item| item.as_str());
            (message, error_type, code)
        }
        _ => return Some("上游 API 返回了无法识别的错误格式。".to_string()),
    };

    let cleaned = clean_api_error_text(raw_message);
    let mut message = if looks_like_html(raw_message) {
        format!(
            "上游返回了 HTML 错误页而不是 API JSON/SSE：{}。这通常表示上游网关、Cloudflare、Base URL、API Key 或网络代理异常。",
            cleaned
        )
    } else {
        format!("上游 API 错误：{}", cleaned)
    };

    let details = [("type", error_type), ("code", code)]
        .into_iter()
        .filter_map(|(label, value)| value.map(|value| format!("{label}: {value}")))
        .collect::<Vec<_>>();
    if !details.is_empty() {
        message.push_str(&format!(" ({})", details.join(", ")));
    }

    Some(message)
}

pub fn api_error_message_from_sse_text(text: &str) -> Option<String> {
    let mut sse = SseParser::new();
    let mut input = text.to_string();
    if !input.ends_with('\n') {
        input.push('\n');
    }
    for (_event, data) in sse.push(&input) {
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        if let Some(message) = api_error_message_from_value(&value) {
            return Some(message);
        }
    }
    None
}

fn clean_api_error_text(text: &str) -> String {
    let cleaned = if looks_like_html(text) {
        html_to_text(text)
    } else {
        collapse_whitespace(text)
    };

    truncate_chars(
        cleaned
            .trim()
            .trim_matches('"')
            .trim()
            .trim_end_matches('.'),
        360,
    )
}

fn looks_like_html(text: &str) -> bool {
    let trimmed = text.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<head")
        || trimmed.starts_with("<body")
        || (trimmed.starts_with('<') && trimmed.contains("</html>"))
}

fn html_to_text(html: &str) -> String {
    let without_scripts = remove_html_block(html, "script");
    let without_styles = remove_html_block(&without_scripts, "style");
    let mut text = String::new();
    let mut in_tag = false;
    for ch in without_styles.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }

    collapse_whitespace(&decode_basic_html_entities(&text))
}

fn remove_html_block(input: &str, tag: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    let lower = input.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");

    while let Some(relative_start) = lower[cursor..].find(&open) {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let Some(relative_end) = lower[start..].find(&close) else {
            cursor = input.len();
            break;
        };
        cursor = start + relative_end + close.len();
    }

    output.push_str(&input[cursor..]);
    output
}

fn decode_basic_html_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        truncated.push_str("...");
    }
    truncated
}

/// 检查 URL 是否包含 API 版本路径 (/v1, /v2, /v3)
/// 精确匹配 /v1/, /v2/, /v3/ 或以 /v1, /v2, /v3 结尾
fn has_api_version_path(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("/v1/")
        || lower.contains("/v2/")
        || lower.contains("/v3/")
        || lower.ends_with("/v1")
        || lower.ends_with("/v2")
        || lower.ends_with("/v3")
}

/// 智能构建 API 端点 URL
/// - 已包含完整路径（/chat/completions 或 /responses）→ 直接使用
/// - 已包含版本路径（/v1, /v2 等）→ 只追加端点
/// - 仅域名 → 追加 /v1/端点
pub fn build_chat_completions_url(base_url: &str, api_mode: &str) -> String {
    let url = base_url.trim().trim_end_matches('/');

    // 已包含完整端点路径，直接使用
    if url.ends_with("/chat/completions") || url.ends_with("/responses") {
        return url.to_string();
    }

    // 检测是否已包含版本路径
    let has_version = has_api_version_path(url);

    let endpoint = match api_mode {
        "responses" => "responses",
        _ => "chat/completions",
    };

    if has_version {
        format!("{}/{}", url, endpoint)
    } else {
        format!("{}/v1/{}", url, endpoint)
    }
}

/// 智能构建 models URL
pub fn build_models_url(base_url: &str) -> String {
    let url = base_url.trim().trim_end_matches('/');

    if url.ends_with("/models") {
        return url.to_string();
    }

    // 检测是否已包含版本路径
    if has_api_version_path(url) {
        return format!("{}/models", url);
    }

    format!("{}/v1/models", url)
}

// ===== Responses API 支持 =====

/// 将 chat/completions 格式的 messages 转换为 Responses API 的 input 格式
pub fn convert_messages_to_responses_input(messages: &[Value]) -> Vec<Value> {
    let mut input = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

        match role {
            "system" | "developer" => {
                let content = msg
                    .get("content")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                input.push(json!({
                    "type": "message",
                    "role": "developer",
                    "content": normalize_content_to_responses(content)
                }));
            }
            "assistant" => {
                // 文本消息部分
                if let Some(content) = msg.get("content") {
                    if !content.is_null() {
                        let is_empty = match content {
                            Value::String(s) => s.is_empty(),
                            Value::Array(a) => a.is_empty(),
                            _ => false,
                        };
                        if !is_empty {
                            // Responses API 里 assistant 的内容块类型必须是 output_text：
                            // 用 input_text 会被官方端点以 "Invalid value: 'input_text'" 拒绝。
                            input.push(json!({
                                "type": "message",
                                "role": "assistant",
                                "content": normalize_assistant_content_to_responses(content.clone())
                            }));
                        }
                    }
                }
                // 工具调用部分：每个 tool_call 变成独立的 function_call item
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
                        let arguments = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        // 只传 call_id：function_call 项的 id 要求是 "fc_" 开头的条目 id，
                        // 把 "call_..." 填进 id 会被官方端点拒绝；省略 id 是允许的。
                        input.push(json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": arguments
                        }));
                    }
                }
            }
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("");
                let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": content
                }));
            }
            _ => {
                // user 等角色
                let content = msg
                    .get("content")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                input.push(json!({
                    "type": "message",
                    "role": role,
                    "content": normalize_content_to_responses(content)
                }));
            }
        }
    }

    input
}

/// 将 chat/completions 的 content 转换为 Responses API 的 content 数组格式
fn normalize_content_to_responses(content: Value) -> Vec<Value> {
    match content {
        Value::String(s) => vec![json!({"type": "input_text", "text": s})],
        Value::Array(arr) => arr
            .into_iter()
            .map(|part| {
                let ptype = part.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                match ptype {
                    "image_url" => {
                        let url = part
                            .get("image_url")
                            .and_then(|iu| iu.get("url"))
                            .and_then(|u| u.as_str())
                            .unwrap_or("");
                        json!({"type": "input_image", "image_url": url})
                    }
                    "text" | _ => {
                        let text = part.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        json!({"type": "input_text", "text": text})
                    }
                }
            })
            .collect(),
        other => vec![json!({"type": "input_text", "text": other.to_string()})],
    }
}

/// assistant 历史消息转换为 Responses 格式：文本块用 output_text（图片不会出现在 assistant 里）。
fn normalize_assistant_content_to_responses(content: Value) -> Vec<Value> {
    let text = match content {
        Value::String(text) => text,
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|text| text.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        other => other.to_string(),
    };
    vec![json!({ "type": "output_text", "text": text })]
}

/// 将 Chat Completions 风格的嵌套工具定义转换为 Responses API 需要的扁平结构。
/// 兼容已扁平的定义；缺少 name 的项会被跳过；description/parameters 缺省补齐，
/// 且不额外添加 "strict" 字段。
fn responses_tool_definitions(tools: &[Value]) -> Vec<Value> {
    let mut definitions = Vec::with_capacity(tools.len());

    for tool in tools {
        let nested = tool.get("function").filter(|f| f.is_object());
        let name = nested
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .or_else(|| tool.get("name").and_then(Value::as_str));
        let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
            continue;
        };

        let description = nested
            .and_then(|f| f.get("description"))
            .and_then(Value::as_str)
            .or_else(|| tool.get("description").and_then(Value::as_str))
            .unwrap_or("");

        let parameters = nested
            .and_then(|f| f.get("parameters"))
            .or_else(|| tool.get("parameters"))
            .filter(|p| p.is_object())
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));

        definitions.push(json!({
            "type": "function",
            "name": name,
            "description": description,
            "parameters": parameters,
        }));
    }

    definitions
}

/// 构建 Responses API 请求体（流式和非流式通用）
pub fn build_responses_request_body(
    config: &DirectApiConfig,
    messages: &[Value],
    tools: Option<Vec<Value>>,
    stream: bool,
) -> Value {
    let input = convert_messages_to_responses_input(messages);

    let mut body = json!({
        "model": config.model,
        "input": input,
        "stream": stream,
    });

    // Responses API 的推理强度参数是 reasoning.effort；顶层 reasoning_effort 是 Chat Completions 的写法。
    if matches!(config.thinking_depth.as_str(), "low" | "medium" | "high") {
        body.as_object_mut()
            .unwrap()
            .insert("reasoning".to_string(), json!({ "effort": config.thinking_depth }));
    }

    if let Some(t) = tools {
        let definitions = responses_tool_definitions(&t);
        if !definitions.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(definitions));
        }
    }

    body
}

/// 解析 Responses API 非流式响应，返回 NonStreamResult
pub fn parse_responses_non_stream(value: &Value) -> NonStreamResult {
    let mut content_parts = Vec::new();
    let mut tool_calls = Vec::new();

    if let Some(output) = value.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        for c in content_arr {
                            let ctype = c.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            match ctype {
                                "output_text" => {
                                    if let Some(text) = c.get("text").and_then(|t| t.as_str()) {
                                        content_parts.push(text.to_string());
                                    }
                                }
                                "reasoning" => {
                                    // reasoning 内容暂时忽略，非流式中不常见
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "function_call" => {
                    let call_id = item
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .or_else(|| item.get("id").and_then(|i| i.as_str()))
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}")
                        .to_string();
                    tool_calls.push(NonStreamToolCall {
                        id: Some(call_id),
                        r#type: "function".to_string(),
                        function: NonStreamFunctionCall { name, arguments },
                    });
                }
                _ => {}
            }
        }
    }

    let combined = content_parts.join("");
    NonStreamResult {
        content: if combined.is_empty() {
            None
        } else {
            Some(combined)
        },
        tool_calls,
        finish_reason: value
            .get("status")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
    }
}

pub async fn call_chat_completions_stream(
    client: &reqwest::Client,
    config: &DirectApiConfig,
    messages: &Vec<serde_json::Value>,
    tools: Option<Vec<serde_json::Value>>,
) -> Result<reqwest::Response, StreamRequestError> {
    let url = build_chat_completions_url(&config.base_url, &config.api_mode);

    let body = if config.api_mode == "responses" {
        build_responses_request_body(config, messages, tools, true)
    } else {
        let mut body = json!({
            "model": config.model,
            "messages": messages,
            "stream": true,
        });

        if matches!(config.thinking_depth.as_str(), "low" | "medium" | "high") {
            body.as_object_mut()
                .unwrap()
                .insert("reasoning_effort".to_string(), json!(config.thinking_depth));
        }

        if let Some(t) = tools {
            if !t.is_empty() {
                body.as_object_mut()
                    .unwrap()
                    .insert("tools".to_string(), json!(t));
            }
        }

        body
    };

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body);
    if !config.api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", config.api_key.trim()));
    }

    let res = tokio::time::timeout(Duration::from_secs(60), req.send())
        .await
        .map_err(|_| StreamRequestError {
            message: "API 请求超过 60 秒没有响应，请检查网络、Base URL 或代理配置。".to_string(),
            retryable: true,
        })?
        .map_err(|e| StreamRequestError {
            message: format!("{e}"),
            retryable: true,
        })?;

    let status = res.status();
    if !status.is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(StreamRequestError {
            message: format!(
                "API 返回错误 {} ({}): {}",
                status.as_u16(),
                url,
                clean_api_error_text(&err_text)
            ),
            retryable: is_retryable_status(status),
        });
    }

    Ok(res)
}

pub async fn call_chat_completions_non_stream(
    client: &reqwest::Client,
    config: &DirectApiConfig,
    messages: &Vec<serde_json::Value>,
    tools: Option<Vec<serde_json::Value>>,
) -> Result<NonStreamResult, String> {
    let url = build_chat_completions_url(&config.base_url, &config.api_mode);

    let body = if config.api_mode == "responses" {
        build_responses_request_body(config, messages, tools, false)
    } else {
        let mut body = json!({
            "model": config.model,
            "messages": messages,
            "stream": false,
        });

        if matches!(config.thinking_depth.as_str(), "low" | "medium" | "high") {
            body.as_object_mut()
                .unwrap()
                .insert("reasoning_effort".to_string(), json!(config.thinking_depth));
        }

        if let Some(t) = tools {
            if !t.is_empty() {
                body.as_object_mut()
                    .unwrap()
                    .insert("tools".to_string(), json!(t));
            }
        }

        body
    };

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body);
    if !config.api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", config.api_key.trim()));
    }

    // 非流式请求要等模型整段生成完才返回响应头：长回复、写大文件的工具调用经常超过 60 秒，
    // 以前固定 60 秒一律判超时。这里给整个请求（含读取响应体）5 分钟。
    const NON_STREAM_TIMEOUT: Duration = Duration::from_secs(300);
    let started = std::time::Instant::now();
    let res = tokio::time::timeout(NON_STREAM_TIMEOUT, req.send())
        .await
        .map_err(|_| "API 请求超过 5 分钟没有返回，请检查网络、Base URL 或代理配置；回复很长时可以改用流式模式。".to_string())?
        .map_err(|e| format!("{e}"))?;

    if !res.status().is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(format!(
            "API 返回错误 ({}): {}",
            url,
            clean_api_error_text(&err_text)
        ));
    }

    // 先读取响应体为文本，以便调试和兼容多种格式
    let remaining = NON_STREAM_TIMEOUT.saturating_sub(started.elapsed()).max(Duration::from_secs(10));
    let response_text = tokio::time::timeout(remaining, res.text())
        .await
        .map_err(|_| "读取响应内容超时，可以改用流式模式。".to_string())?
        .map_err(|e| format!("读取响应内容失败: {e}"))?;

    // 空响应体检测
    if response_text.trim().is_empty() {
        return Err("API 返回了 HTTP 200，但响应体为空。可能原因：API 服务端错误、Base URL 配置错误、或请求被中间代理拦截。".to_string());
    }

    // HTML 响应检测（API 返回了登录页面或错误页面）
    let trimmed = response_text.trim();
    if looks_like_html(trimmed) {
        return Err(format!(
            "API 返回了 HTML 页面而非 JSON 响应：{}。请检查 Base URL、API Key、上游网关或网络代理。",
            clean_api_error_text(trimmed)
        ));
    }

    // Responses API 模式：优先使用专用解析器
    if config.api_mode == "responses" {
        if let Ok(value) = serde_json::from_str::<Value>(&response_text) {
            let result = parse_responses_non_stream(&value);
            if result.content.is_some() || !result.tool_calls.is_empty() {
                return Ok(result);
            }
        }
        // 如果专用解析器未提取到内容，继续尝试通用解析器作为回退
    }

    // 策略1: 直接 JSON 解析
    if let Ok(value) = serde_json::from_str::<Value>(&response_text) {
        if let Some(message) = api_error_message_from_value(&value) {
            return Err(message);
        }

        let content = ContentExtractor::extract_content(&value);
        let tool_calls = ContentExtractor::extract_tool_calls(&value);
        let finish_reason = ContentExtractor::extract_finish_reason(&value);

        if content.is_some() || !tool_calls.is_empty() {
            return Ok(NonStreamResult {
                content,
                tool_calls,
                finish_reason,
            });
        }

        // 检查是否只有 reasoning
        if ContentExtractor::extract_reasoning(&value).is_some() {
            return Err("接口只返回了 reasoning（思考过程），没有返回正文内容。请检查模型是否支持内容输出。".to_string());
        }
    }

    // 策略2: 如果看起来像SSE格式，尝试SSE解析（包含 tool_calls 支持）
    if response_text.contains("data:") {
        let mut sse = SseParser::new();
        let data_lines = sse.push(&response_text);
        let mut combined_content = String::new();
        let mut accumulated_tool_calls: HashMap<usize, NonStreamToolCall> = HashMap::new();

        for (_event, data) in data_lines {
            if data == "[DONE]" {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                if let Some(message) = api_error_message_from_value(&value) {
                    return Err(message);
                }
            }
            if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(&data) {
                if let Some(choice) = chunk.choices.first() {
                    if let Some(ref text) = choice.delta.content {
                        combined_content.push_str(text);
                    }

                    // 处理 SSE 格式的 tool_calls
                    if let Some(ref tc_deltas) = choice.delta.tool_calls {
                        for tc in tc_deltas {
                            let entry =
                                accumulated_tool_calls.entry(tc.index.unwrap_or(0)).or_insert_with(|| {
                                    NonStreamToolCall {
                                        id: None,
                                        r#type: "function".to_string(),
                                        function: NonStreamFunctionCall {
                                            name: String::new(),
                                            arguments: String::new(),
                                        },
                                    }
                                });

                            if let Some(ref id) = tc.id {
                                entry.id = Some(id.clone());
                            }
                            if let Some(ref func) = tc.function {
                                if let Some(ref name) = func.name {
                                    entry.function.name.push_str(name);
                                }
                                if let Some(ref args) = func.arguments {
                                    entry.function.arguments.push_str(args);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !combined_content.is_empty() || !accumulated_tool_calls.is_empty() {
            // 按模型给出的 index 排序：以前按 id（随机串）排序，"先建文件再修改"会被打乱顺序执行。
            let mut indexed: Vec<(usize, NonStreamToolCall)> = accumulated_tool_calls.into_iter().collect();
            indexed.sort_by_key(|(index, _)| *index);
            let tool_calls: Vec<NonStreamToolCall> = indexed.into_iter().map(|(_, tc)| tc).collect();

            return Ok(NonStreamResult {
                content: if combined_content.is_empty() {
                    None
                } else {
                    Some(combined_content)
                },
                tool_calls,
                finish_reason: None,
            });
        }
    }

    // 策略3: 检查是否是纯文本响应（某些API可能返回纯文本）
    if !response_text.trim().is_empty()
        && !response_text.trim().starts_with('{')
        && !response_text.trim().starts_with('[')
    {
        return Ok(NonStreamResult {
            content: Some(response_text.trim().to_string()),
            tool_calls: Vec::new(),
            finish_reason: None,
        });
    }

    // 所有策略失败，返回详细错误
    Err(format!(
        "接口返回了HTTP 200，但响应不是有效的JSON格式。响应内容预览: {}",
        truncate_chars(&response_text, 200)
    ))
}

pub async fn list_models(
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
) -> Result<Vec<String>, String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("请先填写请求地址".to_string());
    }

    let url = build_models_url(base_url);

    let mut req = client.get(&url).header("Accept", "application/json");
    if !api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
    }

    let res = tokio::time::timeout(Duration::from_secs(60), req.send())
        .await
        .map_err(|_| {
            "获取模型列表超过 60 秒没有响应，请检查网络、Base URL 或代理配置。".to_string()
        })?
        .map_err(|e| format!("获取模型列表失败: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(format!(
            "获取模型列表失败 ({}): {}",
            status,
            clean_api_error_text(&err_text)
        ));
    }

    let value = res
        .json::<Value>()
        .await
        .map_err(|e| format!("模型列表解析失败: {e}"))?;
    let mut models = Vec::new();

    collect_model_ids(&value, &mut models);
    models.sort();
    models.dedup();

    if models.is_empty() {
        Err("接口返回成功，但没有找到可用模型".to_string())
    } else {
        Ok(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_cleans_html_error_from_sse() {
        let sse = r#"data: {"error":{"message":"<html><head><style>body{color:red}</style></head><body><p>Unable to load site</p><span>Please try again later.</span><script>window.bad=true</script></body></html>","type":"server_error","code":"upstream_error"}}"#;

        let message = api_error_message_from_sse_text(sse).expect("sse error");

        assert!(message.contains("Unable to load site"));
        assert!(message.contains("Please try again later"));
        assert!(message.contains("upstream_error"));
        assert!(!message.contains("<html"));
        assert!(!message.contains("body{color:red}"));
        assert!(!message.contains("window.bad"));
    }

    #[test]
    fn ignores_normal_sse_chunks_when_extracting_errors() {
        let sse = r#"data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}

data: [DONE]
"#;

        assert!(api_error_message_from_sse_text(sse).is_none());
    }

    #[test]
    fn responses_tool_definitions_flatten_nested_chat_completions_shape() {
        let nested = vec![json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "读取本地文件",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        })];

        let definitions = responses_tool_definitions(&nested);

        assert_eq!(definitions.len(), 1);
        let def = &definitions[0];
        assert_eq!(def["type"], "function");
        assert_eq!(def["name"], "read_file");
        assert!(
            def.get("function").is_none(),
            "Responses tools 不能保留 Chat Completions 的 function 嵌套键"
        );
        assert_eq!(def["description"], "读取本地文件");
        assert_eq!(def["parameters"]["type"], "object");
        assert!(def["parameters"]["properties"]["path"].is_object());
        assert!(def.get("strict").is_none(), "不要擅自添加 strict 字段");
    }

    #[test]
    fn responses_tool_definitions_accept_flat_tools_and_skip_unnamed() {
        let input = vec![
            json!({
                "type": "function",
                "name": "flat_tool",
                "parameters": { "type": "object", "properties": {} }
            }),
            json!({ "type": "function", "function": { "description": "缺少 name" } }),
            json!({ "type": "web_search_preview" }),
        ];

        let definitions = responses_tool_definitions(&input);

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0]["name"], "flat_tool");
        assert_eq!(definitions[0]["description"], "");
        assert_eq!(definitions[0]["parameters"]["type"], "object");
        assert!(definitions[0]["parameters"]["properties"].is_object());

        // 缺省 description/parameters 时补默认值
        let defaults = responses_tool_definitions(&[json!({ "name": "only_name" })]);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0]["type"], "function");
        assert_eq!(defaults[0]["description"], "");
        assert_eq!(defaults[0]["parameters"]["type"], "object");
        assert!(defaults[0]["parameters"]["properties"].is_object());
    }

    #[test]
    fn responses_request_body_uses_flat_tool_definitions() {
        let tools = crate::direct_api::tools::tool_definitions();
        let source_names: Vec<String> = tools
            .iter()
            .filter_map(|tool| {
                tool.get("function")?
                    .get("name")?
                    .as_str()
                    .map(str::to_string)
            })
            .collect();

        let body = build_responses_request_body(
            &DirectApiConfig::default(),
            &[],
            Some(tools.clone()),
            true,
        );
        let body_tools = body["tools"]
            .as_array()
            .expect("responses 请求体应包含 tools 数组");

        assert_eq!(body_tools.len(), tools.len());
        for def in body_tools {
            assert!(
                def.get("function").is_none(),
                "Responses tools 项必须是扁平结构: {def}"
            );
            assert!(
                def.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| !n.is_empty()),
                "工具名称缺失: {def}"
            );
        }

        // 工具名称齐全，一个都不能少
        let body_names: Vec<&str> = body_tools
            .iter()
            .filter_map(|def| def.get("name").and_then(|n| n.as_str()))
            .collect();
        for name in &source_names {
            assert!(
                body_names.contains(&name.as_str()),
                "responses 请求体缺少工具: {name}"
            );
        }
    }

    #[test]
    fn utf8_decoder_keeps_multibyte_characters_split_across_chunks() {
        let bytes = "data: 你好世界\n".as_bytes();
        let split = bytes.iter().position(|&b| b >= 0x80).unwrap() + 1;
        let mut decoder = Utf8ChunkDecoder::default();
        let mut text = decoder.push(&bytes[..split]);
        text.push_str(&decoder.push(&bytes[split..]));
        text.push_str(&decoder.finish());
        assert_eq!(text, "data: 你好世界\n", "逐块 lossy 解码会得到 \"���好世界\"");

        // 极端情况：每个字节单独一块。
        let mut decoder = Utf8ChunkDecoder::default();
        let one_by_one: String = bytes
            .iter()
            .map(|byte| decoder.push(std::slice::from_ref(byte)))
            .collect();
        assert_eq!(one_by_one, "data: 你好世界\n");
    }

    #[test]
    fn utf8_decoder_replaces_only_truly_invalid_bytes() {
        let mut decoder = Utf8ChunkDecoder::default();
        let mut input = b"ok \xFF ".to_vec();
        input.extend_from_slice(&"中".as_bytes()[..1]);
        assert_eq!(decoder.push(&input), "ok \u{FFFD} ");
        assert_eq!(decoder.push(&"中".as_bytes()[1..]), "中");
    }

    #[test]
    fn only_rate_limits_and_server_errors_are_retryable() {
        assert!(is_retryable_status(reqwest::StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(reqwest::StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(reqwest::StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable_status(reqwest::StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(reqwest::StatusCode::PAYLOAD_TOO_LARGE));
    }

    #[test]
    fn responses_input_uses_output_text_for_assistant_and_omits_call_item_id() {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "我先读一下文件",
                "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": "read_file", "arguments": "{}" } }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "ok" }),
        ];
        let input = convert_messages_to_responses_input(&messages);
        assert_eq!(input[0]["content"][0]["type"], "output_text");
        assert_eq!(input[1]["type"], "function_call");
        assert!(input[1].get("id").is_none(), "call_ 开头的 id 不能当作条目 id 发送");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[2]["type"], "function_call_output");
    }

    #[test]
    fn responses_body_puts_effort_under_reasoning() {
        let config = DirectApiConfig {
            thinking_depth: "high".into(),
            ..DirectApiConfig::default()
        };
        let body = build_responses_request_body(&config, &[], None, true);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn tool_call_delta_without_index_still_parses() {
        let chunk: ChatCompletionChunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"hi","tool_calls":[{"id":"call_1","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
        )
        .expect("缺少 index 的 chunk 不能整块丢弃");
        let delta = &chunk.choices[0].delta;
        assert_eq!(delta.content.as_deref(), Some("hi"));
        assert_eq!(delta.tool_calls.as_ref().unwrap()[0].index, None);
    }
}

fn collect_model_ids(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_model_id_from_item(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(data) = map.get("data") {
                collect_model_ids(data, out);
            }
            if let Some(models) = map.get("models") {
                collect_model_ids(models, out);
            }
        }
        _ => {}
    }
}

fn collect_model_id_from_item(item: &Value, out: &mut Vec<String>) {
    match item {
        Value::String(id) => push_model_id(id, out),
        Value::Object(map) => {
            for key in ["id", "name", "model"] {
                if let Some(Value::String(id)) = map.get(key) {
                    push_model_id(id, out);
                    break;
                }
            }
        }
        _ => {}
    }
}

fn push_model_id(id: &str, out: &mut Vec<String>) {
    let id = id.trim();
    if !id.is_empty() {
        out.push(id.to_string());
    }
}

/// 规范化stream_mode值
pub fn normalize_stream_mode(value: &str) -> String {
    match value.to_lowercase().as_str() {
        "auto" | "stream" | "non_stream" => value.to_lowercase(),
        "non-stream" => "non_stream".to_string(),
        _ => "auto".to_string(),
    }
}

/// 规范化api_mode值
pub fn normalize_api_mode(value: &str) -> String {
    match value {
        "chat_completions" | "responses" => value.to_string(),
        _ => "chat_completions".to_string(),
    }
}

/// 内容提取器 - 兼容多种响应格式
pub struct ContentExtractor;

impl ContentExtractor {
    /// 从API响应中提取内容文本
    /// 优先级：
    /// 1. choices[0].delta.content（标准流式）
    /// 2. choices[0].message.content（非流式）
    /// 3. choices[0].text（其他格式）
    /// 4. content数组中的text块
    pub fn extract_content(value: &Value) -> Option<String> {
        // 尝试choices数组
        if let Some(choices) = value.get("choices") {
            if let Some(first_choice) = choices.as_array().and_then(|arr| arr.first()) {
                // 1. delta.content
                if let Some(content) = first_choice
                    .get("delta")
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                {
                    if !content.is_empty() {
                        return Some(content.to_string());
                    }
                }

                // 2. message.content
                if let Some(content) = first_choice
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    if !content.is_empty() {
                        return Some(content.to_string());
                    }
                }

                // 3. text
                if let Some(text) = first_choice.get("text").and_then(|t| t.as_str()) {
                    if !text.is_empty() {
                        return Some(text.to_string());
                    }
                }

                // 4. content as array with text blocks
                if let Some(content_arr) = first_choice
                    .get("message")
                    .or_else(|| first_choice.get("delta"))
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    for item in content_arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    return Some(text.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // 直接content字段
        if let Some(content) = value.get("content").and_then(|c| c.as_str()) {
            if !content.is_empty() {
                return Some(content.to_string());
            }
        }

        None
    }

    /// 提取reasoning内容
    pub fn extract_reasoning(value: &Value) -> Option<String> {
        if let Some(choices) = value.get("choices") {
            if let Some(first_choice) = choices.as_array().and_then(|arr| arr.first()) {
                // reasoning_content
                if let Some(reasoning) = first_choice
                    .get("delta")
                    .and_then(|d| d.get("reasoning_content"))
                    .and_then(|r| r.as_str())
                {
                    if !reasoning.is_empty() {
                        return Some(reasoning.to_string());
                    }
                }

                // reasoning object
                if let Some(reasoning_obj) = first_choice
                    .get("delta")
                    .and_then(|d| d.get("reasoning"))
                    .or_else(|| first_choice.get("delta").and_then(|d| d.get("thinking")))
                {
                    if let Some(text) = reasoning_obj
                        .get("content")
                        .or_else(|| reasoning_obj.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        if !text.is_empty() {
                            return Some(text.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// 提取工具调用（支持流式和非流式格式）
    pub fn extract_tool_calls(value: &Value) -> Vec<NonStreamToolCall> {
        let mut tool_calls = Vec::new();

        if let Some(choices) = value.get("choices") {
            if let Some(first_choice) = choices.as_array().and_then(|arr| arr.first()) {
                // 尝试 message.tool_calls（非流式格式）
                if let Some(tc_array) = first_choice
                    .get("message")
                    .and_then(|m| m.get("tool_calls"))
                    .and_then(|tc| tc.as_array())
                {
                    for tc in tc_array {
                        if let Ok(tool_call) =
                            serde_json::from_value::<NonStreamToolCall>(tc.clone())
                        {
                            tool_calls.push(tool_call);
                        }
                    }
                }

                // 尝试 delta.tool_calls（流式格式，某些API可能在非流式中使用）
                if tool_calls.is_empty() {
                    if let Some(tc_array) = first_choice
                        .get("delta")
                        .and_then(|d| d.get("tool_calls"))
                        .and_then(|tc| tc.as_array())
                    {
                        for tc in tc_array {
                            if let Ok(tool_call) =
                                serde_json::from_value::<NonStreamToolCall>(tc.clone())
                            {
                                tool_calls.push(tool_call);
                            }
                        }
                    }
                }
            }
        }

        tool_calls
    }

    /// 提取 finish_reason
    pub fn extract_finish_reason(value: &Value) -> Option<String> {
        value
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(|fr| fr.as_str())
            .map(|s| s.to_string())
    }
}
