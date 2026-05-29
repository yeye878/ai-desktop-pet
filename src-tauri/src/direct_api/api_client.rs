use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DirectApiConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub confirm_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolCallDelta {
    pub index: usize,
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

pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buffer.push_str(text);
        let mut results = Vec::new();

        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos].trim().to_string();
            self.buffer.drain(..=newline_pos);

            if line.starts_with("data: ") {
                let data_str = line["data: ".len()..].trim().to_string();
                if data_str == "[DONE]" {
                    results.push("[DONE]".to_string());
                } else if !data_str.is_empty() {
                    results.push(data_str);
                }
            }
        }

        results
    }
}

pub async fn call_chat_completions_stream(
    config: &DirectApiConfig,
    messages: &Vec<serde_json::Value>,
    tools: Option<Vec<serde_json::Value>>,
) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::new();
    let url = if config.base_url.ends_with("/chat/completions") {
        config.base_url.clone()
    } else {
        format!("{}/chat/completions", config.base_url.trim_end_matches('/'))
    };

    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "stream": true,
    });

    if let Some(t) = tools {
        if !t.is_empty() {
            body.as_object_mut()
                .unwrap()
                .insert("tools".to_string(), json!(t));
        }
    }

    let req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(&body);

    let res = req.send().await.map_err(|e| format!("API 请求失败: {e}"))?;

    if !res.status().is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(format!("API 返回错误 ({}): {}", url, err_text));
    }

    Ok(res)
}
