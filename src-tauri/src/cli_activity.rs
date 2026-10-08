use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Serialize)]
pub struct ToolEvent {
    pub id: String,
    pub tool_name: String,
    pub status: String,
    pub summary: String,
    pub arguments: String,
    pub command: Option<String>,
    pub path: Option<String>,
    pub output: Option<String>,
    pub approved: Option<bool>,
}

pub enum Activity {
    Text(String),
    Tool(ToolEvent),
}

fn compact(text: &str) -> String {
    let mut value = text.chars().take(3000).collect::<String>();
    if text.chars().count() > 3000 {
        value.push_str("\n... [已截断]");
    }
    value
}

fn text_content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return compact(text);
    }
    if let Some(blocks) = value.as_array() {
        return compact(
            &blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    String::new()
}

#[derive(Default)]
pub struct CliActivityParser {
    tools: HashMap<String, ToolEvent>,
    block_ids: HashMap<u64, String>,
    completed: HashSet<String>,
    message_text: String,
}

impl CliActivityParser {
    pub fn codex(&mut self, line: &str) -> Vec<Activity> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let event_type = value["type"].as_str().unwrap_or_default();
        if !matches!(
            event_type,
            "item.started" | "item.updated" | "item.completed"
        ) {
            return Vec::new();
        }
        let item = &value["item"];
        let Some(id) = item["id"].as_str() else {
            return Vec::new();
        };
        if self.completed.contains(id) {
            return Vec::new();
        }
        let kind = item["type"].as_str().unwrap_or_default();
        let (tool_name, args, output) = match kind {
            "command_execution" => (
                "run_command".to_string(),
                serde_json::json!({ "command": item["command"] }),
                item["aggregated_output"].as_str().map(compact),
            ),
            "file_change" => (
                "edit_file".to_string(),
                serde_json::json!({ "changes": item["changes"] }),
                None,
            ),
            "web_search" => (
                "web_search".to_string(),
                serde_json::json!({ "query": item["query"] }),
                None,
            ),
            "mcp_tool_call" => (
                format!(
                    "{} / {}",
                    item["server"].as_str().unwrap_or("MCP"),
                    item["tool"].as_str().unwrap_or("tool")
                ),
                item["arguments"].clone(),
                item["error"]["message"]
                    .as_str()
                    .map(compact)
                    .or_else(|| Some(text_content(&item["result"]["content"]))),
            ),
            _ => return Vec::new(),
        };
        let failed = item["status"] == "failed"
            || item["exit_code"].as_i64().is_some_and(|code| code != 0)
            || item["error"].is_object();
        let status = if failed {
            "failed"
        } else if event_type == "item.completed" {
            "completed"
        } else {
            "running"
        };
        if event_type == "item.completed" {
            self.completed.insert(id.to_string());
        }
        let command = args["command"].as_str().map(str::to_string);
        let path = item["changes"]
            .as_array()
            .and_then(|changes| changes.first())
            .and_then(|change| change["path"].as_str())
            .map(str::to_string);
        vec![Activity::Tool(ToolEvent {
            id: id.to_string(),
            summary: command
                .clone()
                .or_else(|| path.clone())
                .unwrap_or_else(|| tool_name.clone()),
            tool_name,
            status: status.into(),
            arguments: serde_json::to_string_pretty(&args).unwrap_or_default(),
            command,
            path,
            output,
            approved: None,
        })]
    }

    pub fn claude(&mut self, line: &str) -> Vec<Activity> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        match value["type"].as_str().unwrap_or_default() {
            "stream_event" => {
                let event = &value["event"];
                match event["type"].as_str().unwrap_or_default() {
                    "message_start" => {
                        self.message_text.clear();
                        self.block_ids.clear();
                    }
                    "content_block_start" => {
                        let block = &event["content_block"];
                        if block["type"] == "tool_use" {
                            if let Some(tool) = self.tool_start(block) {
                                self.block_ids
                                    .insert(event["index"].as_u64().unwrap_or(0), tool.id.clone());
                                result.push(Activity::Tool(tool));
                            }
                        } else if let Some(text) =
                            block["text"].as_str().filter(|text| !text.is_empty())
                        {
                            self.message_text.push_str(text);
                            result.push(Activity::Text(text.to_string()));
                        }
                    }
                    "content_block_delta" => {
                        let delta = &event["delta"];
                        if let Some(text) = delta["text"].as_str() {
                            self.message_text.push_str(text);
                            result.push(Activity::Text(text.to_string()));
                        }
                        if let Some(fragment) = delta["partial_json"].as_str() {
                            if let Some(id) =
                                self.block_ids.get(&event["index"].as_u64().unwrap_or(0))
                            {
                                if let Some(tool) = self.tools.get_mut(id) {
                                    if tool.arguments == "{}" {
                                        tool.arguments.clear();
                                    }
                                    tool.arguments.push_str(fragment);
                                }
                            }
                        }
                    }
                    "content_block_stop" => {
                        if let Some(id) = self.block_ids.get(&event["index"].as_u64().unwrap_or(0))
                        {
                            if let Some(tool) = self.tools.get_mut(id) {
                                tool.status = "running".into();
                                if let Ok(args) = serde_json::from_str::<Value>(&tool.arguments) {
                                    tool.command = args["command"].as_str().map(str::to_string);
                                    tool.path = args["file_path"]
                                        .as_str()
                                        .or_else(|| args["path"].as_str())
                                        .map(str::to_string);
                                }
                                result.push(Activity::Tool(tool.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            "assistant" => {
                if let Some(blocks) = value["message"]["content"].as_array() {
                    let text = blocks
                        .iter()
                        .filter_map(|block| block["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("");
                    if text.starts_with(&self.message_text) {
                        let delta = &text[self.message_text.len()..];
                        if !delta.is_empty() {
                            result.push(Activity::Text(delta.to_string()));
                        }
                    } else if !text.is_empty() {
                        result.push(Activity::Text(text.clone()));
                    }
                    self.message_text = text;
                    for block in blocks.iter().filter(|block| block["type"] == "tool_use") {
                        if let Some(tool) = self.tool_start(block) {
                            result.push(Activity::Tool(tool));
                        }
                    }
                }
            }
            "user" => {
                if let Some(blocks) = value["message"]["content"].as_array() {
                    for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
                        let Some(id) = block["tool_use_id"].as_str() else {
                            continue;
                        };
                        if !self.completed.insert(id.to_string()) {
                            continue;
                        }
                        let mut tool = self.tools.get(id).cloned().unwrap_or(ToolEvent {
                            id: id.into(),
                            tool_name: "tool".into(),
                            status: String::new(),
                            summary: "tool".into(),
                            arguments: String::new(),
                            command: None,
                            path: None,
                            output: None,
                            approved: None,
                        });
                        tool.status = if block["is_error"].as_bool().unwrap_or(false) {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into();
                        tool.output = Some(text_content(&block["content"]));
                        result.push(Activity::Tool(tool));
                    }
                }
            }
            _ => {}
        }
        result
    }

    fn tool_start(&mut self, block: &Value) -> Option<ToolEvent> {
        let id = block["id"].as_str()?;
        if self.completed.contains(id) {
            return None;
        }
        let name = block["name"].as_str().unwrap_or("tool");
        let args = &block["input"];
        let event = ToolEvent {
            id: id.into(),
            tool_name: name.into(),
            status: "running".into(),
            summary: name.into(),
            arguments: serde_json::to_string_pretty(args).unwrap_or_default(),
            command: args["command"].as_str().map(str::to_string),
            path: args["file_path"]
                .as_str()
                .or_else(|| args["path"].as_str())
                .map(str::to_string),
            output: None,
            approved: None,
        };
        self.tools.insert(id.into(), event.clone());
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_tools_keep_lifecycle_failure_and_chinese_output() {
        let mut parser = CliActivityParser::default();
        let start = r#"{"type":"item.started","item":{"id":"c1","type":"command_execution","command":"测试命令","status":"in_progress"}}"#;
        let Activity::Tool(tool) = parser.codex(start).remove(0) else {
            panic!()
        };
        assert_eq!(tool.status, "running");
        let end = r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"测试命令","exit_code":1,"aggregated_output":"检查失败"}}"#;
        let Activity::Tool(tool) = parser.codex(end).remove(0) else {
            panic!()
        };
        assert_eq!(tool.status, "failed");
        assert_eq!(tool.output.as_deref(), Some("检查失败"));
        assert!(parser.codex(start).is_empty());
        assert!(parser.codex(end).is_empty());
    }
    #[test]
    fn claude_stream_and_snapshot_do_not_repeat_text_and_pair_results() {
        let mut parser = CliActivityParser::default();
        assert!(parser.claude("not JSON").is_empty());
        let chunk = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"先检查"}}}"#;
        assert!(matches!(parser.claude(chunk).remove(0), Activity::Text(_)));
        let snapshot = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"先检查"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/中文.ts"}}]}}"#;
        let events = parser.claude(snapshot);
        assert_eq!(events.len(), 1);
        let Activity::Tool(tool) = &events[0] else {
            panic!()
        };
        assert_eq!(tool.path.as_deref(), Some("src/中文.ts"));
        let output = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"找不到"},{"type":"image","data":"private-base64"}]}]}}"#;
        let Activity::Tool(tool) = parser.claude(output).remove(0) else {
            panic!()
        };
        assert_eq!(tool.status, "failed");
        assert_eq!(tool.output.as_deref(), Some("找不到"));
        assert!(parser.claude(output).is_empty());
    }

    #[test]
    fn codex_file_search_mcp_and_large_output_are_inspectable() {
        let mut parser = CliActivityParser::default();
        let change = r#"{"type":"item.completed","item":{"id":"f1","type":"file_change","status":"failed","changes":[{"path":"src/main.ts","kind":"update"}]}}"#;
        let Activity::Tool(tool) = parser.codex(change).remove(0) else {
            panic!()
        };
        assert_eq!(tool.status, "failed");
        assert_eq!(tool.path.as_deref(), Some("src/main.ts"));
        let mcp = r#"{"type":"item.completed","item":{"id":"m1","type":"mcp_tool_call","server":"files","tool":"read","arguments":{"path":"a"},"result":{"content":[{"type":"text","text":"结果"},{"type":"image","data":"private-image"}]}}}"#;
        let Activity::Tool(tool) = parser.codex(mcp).remove(0) else {
            panic!()
        };
        assert_eq!(tool.output.as_deref(), Some("结果"));
        let search =
            r#"{"type":"item.started","item":{"id":"s1","type":"web_search","query":"测试"}}"#;
        let Activity::Tool(tool) = parser.codex(search).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "web_search");
        assert!(compact(&"中".repeat(5000)).chars().count() < 3100);
    }

    #[test]
    fn claude_partial_arguments_preserve_command_and_independent_calls() {
        let mut parser = CliActivityParser::default();
        parser.claude(r#"{"type":"stream_event","event":{"type":"message_start"}}"#);
        parser.claude(r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"bash1","name":"Bash","input":{}}}}"#);
        parser.claude(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"echo 中文\"}"}}}"#);
        let Activity::Tool(tool) = parser
            .claude(r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#)
            .remove(0)
        else {
            panic!()
        };
        assert_eq!(tool.command.as_deref(), Some("echo 中文"));
        let result = parser.claude(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"bash1","content":"中文"}]}}"#);
        let Activity::Tool(tool) = &result[0] else {
            panic!()
        };
        assert_eq!(tool.status, "completed");
        parser.claude(r#"{"type":"stream_event","event":{"type":"message_start"}}"#);
        let text = parser.claude(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"另一轮正文"}]}}"#,
        );
        assert!(matches!(&text[0], Activity::Text(value) if value == "另一轮正文"));
    }

    async fn fixture_child(lines: &[Value]) -> tokio::process::Child {
        tokio::process::Command::new("node")
            .args([
                "-e",
                &format!(
                    "for (const item of {}) process.stdout.write(JSON.stringify(item)+'\\n');",
                    serde_json::to_string(lines).unwrap()
                ),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("Node.js is required by the desktop frontend toolchain")
    }

    #[tokio::test]
    async fn codex_process_stream_preserves_existing_answer_and_emits_tool_events() {
        let mut child = fixture_child(&[
            serde_json::json!({"type":"item.started","item":{"id":"cmd1","type":"command_execution","command":"fixture","status":"in_progress"}}),
            serde_json::json!({"type":"item.completed","item":{"id":"cmd1","type":"command_execution","command":"fixture","status":"completed","aggregated_output":"成功"}}),
            serde_json::json!({"type":"item.completed","item":{"id":"msg1","type":"agent_message","text":"完成"}}),
            serde_json::json!({"type":"turn.completed"}),
        ]).await;
        let mut text = String::new();
        let mut events = Vec::new();
        let state = crate::codex::read_stream_with_activity(
            &mut child,
            |_| {},
            |delta| text.push_str(delta),
            |event| events.push(event.clone()),
        )
        .await
        .unwrap();
        assert!(!state.is_error);
        assert_eq!(state.full_text, "完成");
        assert_eq!(text, "完成");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].output.as_deref(), Some("成功"));
    }

    #[tokio::test]
    async fn claude_process_stream_deduplicates_partial_and_final_messages() {
        let mut child = fixture_child(&[
            serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"中文"}}}),
            serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"中文"},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.ts"}}]}}),
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"fixture"}]}}),
            serde_json::json!({"type":"result","result":"中文","session_id":"fixture"}),
        ]).await;
        let mut text = String::new();
        let mut events = Vec::new();
        let state = crate::openclaw::read_stream_with_activity(
            &mut child,
            |_| {},
            |delta| text.push_str(delta),
            |event| events.push(event.clone()),
        )
        .await
        .unwrap();
        assert!(!state.is_error);
        assert_eq!(state.full_text, "中文");
        assert_eq!(text, "中文");
        assert_eq!(events.len(), 2);
        assert_eq!(state.session_id.as_deref(), Some("fixture"));
    }
}
