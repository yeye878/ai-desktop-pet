use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, Serialize)]
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
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub changes: Option<Vec<FileChange>>,
}

/// Codex file_change 的单个条目：路径 + 变更类型（add/update/delete）。
#[derive(Clone, Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub kind: Option<String>,
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

/// Codex todo_list 的清单数组（不同版本字段名不同，按顺序回退）。
fn plan_items(item: &Value) -> Value {
    for key in ["items", "plan", "todos"] {
        if item[key].is_array() {
            return item[key].clone();
        }
    }
    Value::Array(Vec::new())
}

/// Codex file_change 的变更列表，只保留可展示的 path / kind。
fn file_changes(item: &Value) -> Option<Vec<FileChange>> {
    let changes = item["changes"].as_array()?;
    Some(
        changes
            .iter()
            .filter_map(|change| {
                change["path"].as_str().map(|path| FileChange {
                    path: path.to_string(),
                    kind: change["kind"].as_str().map(str::to_string),
                })
            })
            .collect(),
    )
}

#[derive(Default)]
pub struct CliActivityParser {
    tools: HashMap<String, ToolEvent>,
    block_ids: HashMap<u64, String>,
    completed: HashSet<String>,
    /// 已上报过的 Codex 配置告警：Codex 每轮都会重复同一条 message。
    notices: HashSet<String>,
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
        if kind == "reasoning" {
            // 思考内容由 codex.rs 的 thinking 通道统一输出，不重复计入工具轨迹。
            return Vec::new();
        }
        if kind == "error" {
            if event_type != "item.completed" {
                return Vec::new();
            }
            // Codex 每一轮都会重复同一条配置告警，同一轮内只上报一次。
            let message = item["message"].as_str().unwrap_or_default().to_string();
            if message.is_empty() || !self.notices.insert(message.clone()) {
                return Vec::new();
            }
            let summary = message
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("Codex 提示")
                .to_string();
            self.completed.insert(id.to_string());
            return vec![Activity::Tool(ToolEvent {
                id: id.to_string(),
                tool_name: "codex_notice".into(),
                status: "notice".into(),
                summary,
                arguments: "{}".into(),
                command: None,
                path: None,
                output: Some(compact(&message)),
                approved: None,
                exit_code: None,
                duration_ms: None,
                changes: None,
            })];
        }
        let (tool_name, args, output, changes) = match kind {
            "command_execution" => (
                "run_command".to_string(),
                serde_json::json!({ "command": item["command"] }),
                item["aggregated_output"].as_str().map(compact),
                None,
            ),
            "file_change" => {
                let changes = file_changes(item);
                (
                    "edit_file".to_string(),
                    serde_json::json!({ "changes": item["changes"] }),
                    None,
                    changes,
                )
            }
            "web_search" => (
                "web_search".to_string(),
                serde_json::json!({ "query": item["query"], "action": item["action"] }),
                None,
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
                None,
            ),
            "todo_list" => (
                "update_plan".to_string(),
                serde_json::json!({ "plan": plan_items(item) }),
                None,
                None,
            ),
            _ => return Vec::new(),
        };
        let exit_code = item["exit_code"].as_i64();
        let failed = item["status"] == "failed"
            || exit_code.is_some_and(|code| code != 0)
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
        let path = changes
            .as_ref()
            .and_then(|changes| changes.first())
            .map(|change| change.path.clone());
        let summary = if kind == "todo_list" {
            "更新任务清单".to_string()
        } else {
            command
                .clone()
                .or_else(|| path.clone())
                .unwrap_or_else(|| tool_name.clone())
        };
        vec![Activity::Tool(ToolEvent {
            id: id.to_string(),
            summary,
            tool_name,
            status: status.into(),
            arguments: serde_json::to_string_pretty(&args).unwrap_or_default(),
            command,
            path,
            output,
            approved: None,
            exit_code,
            duration_ms: item["duration_ms"].as_u64(),
            changes,
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
                            exit_code: None,
                            duration_ms: None,
                            changes: None,
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
            exit_code: None,
            duration_ms: None,
            changes: None,
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
        assert_eq!(
            tool.changes
                .as_ref()
                .map(|changes| changes[0].kind.as_deref()),
            Some(Some("update"))
        );
        let mcp = r#"{"type":"item.completed","item":{"id":"m1","type":"mcp_tool_call","server":"files","tool":"read","arguments":{"path":"a"},"result":{"content":[{"type":"text","text":"结果"},{"type":"image","data":"private-image"}]}}}"#;
        let Activity::Tool(tool) = parser.codex(mcp).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "files / read");
        assert_eq!(tool.output.as_deref(), Some("结果"));
        let search =
            r#"{"type":"item.started","item":{"id":"s1","type":"web_search","query":"测试"}}"#;
        let Activity::Tool(tool) = parser.codex(search).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "web_search");
        assert_eq!(tool.arguments, "{\n  \"action\": null,\n  \"query\": \"测试\"\n}");
        assert!(compact(&"中".repeat(5000)).chars().count() < 3100);
    }

    #[test]
    fn codex_command_file_plan_and_notice_carry_structured_details() {
        let mut parser = CliActivityParser::default();
        let command = r#"{"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"cargo test","aggregated_output":"失败","exit_code":-1,"duration_ms":1234,"status":"failed"}}"#;
        let Activity::Tool(tool) = parser.codex(command).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "run_command");
        assert_eq!(tool.command.as_deref(), Some("cargo test"));
        assert_eq!(tool.exit_code, Some(-1));
        assert_eq!(tool.duration_ms, Some(1234));
        assert_eq!(tool.status, "failed");
        assert!(tool.changes.is_none());
        let change = r#"{"type":"item.completed","item":{"id":"f1","type":"file_change","changes":[{"path":"a.rs","kind":"add"},{"path":"b.rs","kind":"delete"},{"path":"c.rs"}]}}"#;
        let Activity::Tool(tool) = parser.codex(change).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "edit_file");
        assert_eq!(tool.path.as_deref(), Some("a.rs"));
        let changes = tool.changes.as_ref().expect("changes");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[1].path, "b.rs");
        assert_eq!(changes[1].kind.as_deref(), Some("delete"));
        assert!(changes[2].kind.is_none(), "kind 允许为 null");
        assert!(tool.arguments.contains("\"path\": \"a.rs\""));
        let plan = r#"{"type":"item.completed","item":{"id":"p1","type":"todo_list","items":[{"text":"第一步","completed":true},{"text":"第二步","completed":false}]}}"#;
        let Activity::Tool(tool) = parser.codex(plan).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "update_plan");
        assert_eq!(tool.summary, "更新任务清单");
        assert!(tool.arguments.contains("第一步"));
        assert!(parser.codex(plan).is_empty(), "completed item 只报一次");
    }

    #[test]
    fn codex_notice_is_reported_once_per_turn_and_reasoning_stays_out_of_activity() {
        let mut parser = CliActivityParser::default();
        let notice = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 3 unrecognized configuration settings. Check for typos or deprecated settings.\n  user (config.toml): `network_access` is ignored."}}"#;
        let Activity::Tool(tool) = parser.codex(notice).remove(0) else {
            panic!()
        };
        assert_eq!(tool.tool_name, "codex_notice");
        assert_eq!(tool.status, "notice");
        assert_eq!(
            tool.summary,
            "Codex is ignoring 3 unrecognized configuration settings. Check for typos or deprecated settings."
        );
        assert_eq!(tool.output.as_deref(), Some("Codex is ignoring 3 unrecognized configuration settings. Check for typos or deprecated settings.\n  user (config.toml): `network_access` is ignored."));
        // Codex 每轮都会重复同一条告警，第二轮重新计一次。
        assert!(parser.codex(notice).is_empty());
        let other = r#"{"type":"item.completed","item":{"id":"item_1","type":"error","message":"另一条告警"}}"#;
        assert_eq!(parser.codex(other).len(), 1);
        let mut next_turn = CliActivityParser::default();
        assert_eq!(next_turn.codex(notice).len(), 1);
        // 思考内容只走 thinking 通道，不重复进工具轨迹。
        assert!(parser
            .codex(r#"{"type":"item.completed","item":{"id":"r1","type":"reasoning","text":"先检查配置"}}"#)
            .is_empty());
        assert!(parser
            .codex(r#"{"type":"item.started","item":{"id":"r2","type":"reasoning","text":"思考中"}}"#)
            .is_empty());
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
            serde_json::json!({"type":"item.completed","item":{"id":"cmd1","type":"command_execution","command":"fixture","status":"completed","aggregated_output":"成功","exit_code":0}}),
            serde_json::json!({"type":"item.completed","item":{"id":"think1","type":"reasoning","text":"先确认命令结果"}}),
            serde_json::json!({"type":"item.completed","item":{"id":"msg1","type":"agent_message","text":"完成"}}),
            serde_json::json!({"type":"turn.completed"}),
        ]).await;
        let mut text = String::new();
        let mut thinking = String::new();
        let mut events = Vec::new();
        let state = crate::codex::read_stream_with_activity(
            &mut child,
            |delta| thinking.push_str(delta),
            |delta| text.push_str(delta),
            |event| events.push(event.clone()),
        )
        .await
        .unwrap();
        assert!(!state.is_error);
        assert_eq!(state.full_text, "完成");
        assert_eq!(text, "完成");
        assert_eq!(thinking, "先确认命令结果");
        assert_eq!(state.full_thinking, thinking);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].output.as_deref(), Some("成功"));
        assert_eq!(events[1].exit_code, Some(0));
    }

    #[tokio::test]
    async fn codex_process_stream_interleaves_thinking_tools_and_text_in_order() {
        // 跨通道顺序：三类回调共用一个有序日志，钉住"思考 → 工具 → 过程正文 → 最终正文"
        // 的真实交错。单看各通道内容无法发现 reasoning 被挪到回合末尾这类回归。
        let mut child = fixture_child(&[
            serde_json::json!({"type":"item.completed","item":{"id":"think1","type":"reasoning","text":"先看目录"}}),
            serde_json::json!({"type":"item.started","item":{"id":"cmd1","type":"command_execution","command":"Get-ChildItem","status":"in_progress"}}),
            serde_json::json!({"type":"item.completed","item":{"id":"cmd1","type":"command_execution","command":"Get-ChildItem","status":"completed","aggregated_output":"a.txt","exit_code":0}}),
            serde_json::json!({"type":"item.completed","item":{"id":"think2","type":"reasoning","text":"再确认文件"}}),
            serde_json::json!({"type":"item.completed","item":{"id":"msg1","type":"agent_message","text":"完成"}}),
            serde_json::json!({"type":"turn.completed"}),
        ]).await;
        // 三个回调没有共享的可变捕获，用 RefCell 记录同一个有序日志。
        let order = std::cell::RefCell::new(Vec::<String>::new());
        let state = crate::codex::read_stream_with_activity(
            &mut child,
            |delta| order.borrow_mut().push(format!("thinking:{delta}")),
            |delta| order.borrow_mut().push(format!("text:{delta}")),
            |event| order.borrow_mut().push(format!("tool:{}:{}", event.id, event.status)),
        )
        .await
        .unwrap();
        assert!(!state.is_error);
        assert_eq!(
            order.borrow().as_slice(),
            [
                "thinking:先看目录".to_string(),
                "tool:cmd1:running".to_string(),
                "tool:cmd1:completed".to_string(),
                "thinking:再确认文件".to_string(),
                "text:完成".to_string(),
            ]
        );
        assert_eq!(state.full_thinking, "先看目录\n\n再确认文件");
        assert_eq!(state.full_text, "完成");
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
