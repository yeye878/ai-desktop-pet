use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    pub auto_approved_tools: Vec<String>,
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
            auto_approved_tools: Vec::new(),
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

            if let Some(data) = line.strip_prefix("data:") {
                let data_str = data.trim().to_string();
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
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 API 客户端失败: {e}"))?;
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

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body);
    if !config.api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", config.api_key.trim()));
    }

    let res = tokio::time::timeout(Duration::from_secs(45), req.send())
        .await
        .map_err(|_| "API 请求超过 45 秒没有响应，请检查网络、Base URL 或代理配置。".to_string())?
        .map_err(|e| format!("API 请求失败: {e}"))?;

    if !res.status().is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(format!("API 返回错误 ({}): {}", url, err_text));
    }

    Ok(res)
}

pub async fn list_models(api_key: &str, base_url: &str) -> Result<Vec<String>, String> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err("请先填写请求地址".to_string());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 API 客户端失败: {e}"))?;
    let url = if base_url.ends_with("/models") {
        base_url.to_string()
    } else {
        format!("{}/models", base_url.trim_end_matches('/'))
    };

    let mut req = client.get(&url).header("Accept", "application/json");
    if !api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
    }

    let res = tokio::time::timeout(Duration::from_secs(45), req.send())
        .await
        .map_err(|_| "获取模型列表超过 45 秒没有响应，请检查网络、Base URL 或代理配置。".to_string())?
        .map_err(|e| format!("获取模型列表失败: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let err_text = res
            .text()
            .await
            .unwrap_or_else(|_| "无法读取错误详情".to_string());
        return Err(format!("获取模型列表失败 ({}): {}", status, err_text));
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
