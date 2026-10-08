//! 智能体工坊工具：让 AI 能在对话里直接创建 / 修改本应用的智能体。
//!
//! 数据来源与 `storage::Database` 相同的 `pet.db`，但这里走独立短连接
//! （与 save_memory / create_scheduled_task 一致），避免把 DB 句柄传进工具层。
//! 保存后由调用方（direct_api::agent 的工具循环）广播 `agents-changed`，
//! 让所有已开的窗口刷新「智能体工坊」列表。

use rusqlite::Connection;
use serde_json::{json, Value};

const NAME_MAX: usize = 40;
const AVATAR_MAX: usize = 8;
const DESC_MAX: usize = 300;
const PROMPT_MAX: usize = 4000;
const MODEL_MAX: usize = 200;
const TOOL_MAX: usize = 30;
const TOOL_NAME_MAX: usize = 64;
/// AI 自动生成的系统提示词长度约束：太短会写出空壳角色，太长会挤占上下文。
const PROMPT_MIN_CHARS: usize = 40;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// 与工具层其他模块共用同一个库文件。
fn db_path() -> Result<std::path::PathBuf, String> {
    let mut path = dirs::data_dir().ok_or("无法获取数据目录")?;
    path.push("ai-desktop-pet");
    path.push("pet.db");
    Ok(path)
}

fn open_db() -> Result<Connection, String> {
    let path = db_path()?;
    let conn = Connection::open(&path).map_err(|e| format!("打开数据库失败: {e}"))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Ok(conn)
}

fn is_reserved_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("claude") || name.eq_ignore_ascii_case("codex")
}

fn normalize_name(raw: &str) -> Result<String, String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err("智能体名字不能为空".to_string());
    }
    if name.chars().any(|c| c.is_whitespace()) {
        return Err("智能体名字不能包含空格（名字会用于对话中的 @ 提及）".to_string());
    }
    if name.contains('@') {
        return Err("智能体名字不能包含 @".to_string());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("智能体名字不能包含控制字符".to_string());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("智能体名字不能超过 {NAME_MAX} 个字"));
    }
    if is_reserved_name(&name) {
        return Err("claude 和 codex 是内置角色，请换一个名字".to_string());
    }
    Ok(name)
}

/// 与前端 `agent- + Date.now().toString(36)` 同风格，避免 id 里出现非法字符。
fn new_agent_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("agent-{}", base36(millis as u64))
}

fn base36(mut value: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ascii")
}

fn clean_allowed_tools(raw: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(items) = raw.and_then(|value| value.as_array()) else {
        return Ok(Vec::new());
    };
    let mut tools: Vec<String> = Vec::new();
    for item in items {
        let Some(name) = item.as_str() else { continue };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if name.chars().count() > TOOL_NAME_MAX {
            return Err(format!("工具名过长: {name}"));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("工具名只能包含字母、数字和 '_'：{name}"));
        }
        if !tools.iter().any(|existing| existing == name) {
            tools.push(name.to_string());
        }
    }
    if tools.len() > TOOL_MAX {
        return Err(format!("单个智能体最多 {TOOL_MAX} 个免确认工具"));
    }
    Ok(tools)
}

fn name_taken(conn: &Connection, name: &str, except_id: Option<&str>) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM agents")
        .map_err(|e| format!("读取智能体列表失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("读取智能体列表失败: {e}"))?;
    for row in rows {
        let (id, existing_name) = row.map_err(|e| format!("读取智能体列表失败: {e}"))?;
        if existing_name == name && except_id != Some(id.as_str()) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn describe_agent(row: &rusqlite::Row<'_>) -> Result<Value, rusqlite::Error> {
    let allowed_json: String = row.get(5)?;
    let allowed: Vec<String> = serde_json::from_str(&allowed_json).unwrap_or_default();
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "name": row.get::<_, String>(1)?,
        "avatar": row.get::<_, String>(2)?,
        "description": row.get::<_, String>(3)?,
        "backend": row.get::<_, String>(4)?,
        "allowed_tools": allowed,
        "system_prompt_chars": row.get::<_, String>(6)?.chars().count(),
    }))
}

/// 列出已存在的自定义智能体，让模型先看到现状再决定新建还是修改。
pub fn exec_list_agents() -> String {
    let result = (|| -> Result<String, String> {
        let conn = open_db()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, avatar, description, backend, allowed_tools_json, system_prompt
                 FROM agents ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("读取智能体列表失败: {e}"))?;
        let rows = stmt
            .query_map([], describe_agent)
            .map_err(|e| format!("读取智能体列表失败: {e}"))?;
        let mut agents = Vec::new();
        for row in rows {
            agents.push(row.map_err(|e| format!("读取智能体列表失败: {e}"))?);
        }
        Ok(serde_json::to_string_pretty(&json!({
            "内置角色（不可修改）": [
                { "name": "claude", "description": "本机 Claude Code 独立智能体" },
                { "name": "codex", "description": "本机 Codex 独立智能体" }
            ],
            "自定义智能体": agents,
        }))
        .unwrap_or_default())
    })();
    result.unwrap_or_else(|e| e)
}

/// 新建智能体。名字唯一、不能占用内置角色名，系统提示词必须足够具体。
pub fn exec_create_agent(args: &Value) -> String {
    let result = (|| -> Result<String, String> {
        let name = normalize_name(args["name"].as_str().ok_or("缺少 'name' 参数")?)?;
        let system_prompt = args["system_prompt"]
            .as_str()
            .ok_or("缺少 'system_prompt' 参数（必须先写出这个角色的完整角色设定）")?
            .trim()
            .to_string();
        if system_prompt.is_empty() {
            return Err("system_prompt 不能为空：没有角色设定的智能体在对话里只是个空壳。请按身份、职责、输出风格、边界四段写清楚。".to_string());
        }
        if system_prompt.chars().count() < PROMPT_MIN_CHARS {
            return Err(format!(
                "system_prompt 太短（当前 {} 字）：至少 {PROMPT_MIN_CHARS} 字，写清身份、职责、工作方式、输出风格和边界。",
                system_prompt.chars().count()
            ));
        }
        if system_prompt.chars().count() > PROMPT_MAX {
            return Err(format!("system_prompt 不能超过 {PROMPT_MAX} 个字"));
        }
        let description = args["description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .chars()
            .take(DESC_MAX)
            .collect::<String>();
        let avatar = args["avatar"]
            .as_str()
            .unwrap_or("")
            .trim()
            .chars()
            .take(AVATAR_MAX)
            .collect::<String>();
        let avatar = if avatar.is_empty() {
            "🤖".to_string()
        } else {
            avatar
        };
        let model = args["model"]
            .as_str()
            .unwrap_or("")
            .trim()
            .chars()
            .take(MODEL_MAX)
            .collect::<String>();
        let allowed_tools = clean_allowed_tools(args.get("allowed_tools"))?;

        let conn = open_db()?;
        if let Some(existing_id) = name_taken(&conn, &name, None)? {
            return Err(format!(
                "已存在名为「{name}」的智能体（id={existing_id}）。想改它请调用 update_agent，换名字请换一个不同的名字。"
            ));
        }

        let id = new_agent_id();
        let allowed_json = serde_json::to_string(&allowed_tools).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO agents (
                id, name, avatar, description, system_prompt,
                model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'direct_api', '')",
            rusqlite::params![
                &id,
                &name,
                &avatar,
                &description,
                &system_prompt,
                &model,
                &allowed_json,
                now_secs(),
            ],
        )
        .map_err(|e| format!("保存智能体失败: {e}"))?;

        Ok(format!(
            "智能体「{name}」已创建（id={id}，头像 {avatar}，角色设定 {} 字，免确认工具 {} 个）。用户现在可以在对话里用 @{name} 召唤它。",
            system_prompt.chars().count(),
            allowed_tools.len()
        ))
    })();
    result.unwrap_or_else(|e| e)
}

/// 修改已有智能体：只更新显式传入的字段，其余保持原样。
pub fn exec_update_agent(args: &Value) -> String {
    let result = (|| -> Result<String, String> {
        let lookup = args["agent"]
            .as_str()
            .ok_or("缺少 'agent' 参数（要修改的智能体名字或 id）")?
            .trim()
            .to_string();
        let conn = open_db()?;
        let existing = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, avatar, description, system_prompt, model, allowed_tools_json, backend
                     FROM agents WHERE id = ?1 OR name = ?1",
                )
                .map_err(|e| format!("读取智能体失败: {e}"))?;
            let mut rows = stmt
                .query_map([&lookup], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })
                .map_err(|e| format!("读取智能体失败: {e}"))?;
            rows.next()
                .transpose()
                .map_err(|e| format!("读取智能体失败: {e}"))?
                .ok_or_else(|| format!("没有找到名为或 id 为「{lookup}」的自定义智能体。先用 list_agents 看看现有角色。"))?
        };
        let (id, old_name, old_avatar, old_desc, old_prompt, old_model, old_tools_json, backend) = existing;

        let name = match args.get("name").and_then(|value| value.as_str()) {
            Some(raw) if !raw.trim().is_empty() => normalize_name(raw)?,
            _ => old_name.clone(),
        };
        if name != old_name {
            if let Some(clash) = name_taken(&conn, &name, Some(&id))? {
                return Err(format!("已存在名为「{name}」的智能体（id={clash}），换一个名字。"));
            }
        }
        let system_prompt = match args.get("system_prompt").and_then(|value| value.as_str()) {
            Some(raw) if !raw.trim().is_empty() => {
                let prompt = raw.trim().to_string();
                if prompt.chars().count() < PROMPT_MIN_CHARS {
                    return Err(format!(
                        "system_prompt 太短（当前 {} 字）：至少 {PROMPT_MIN_CHARS} 字。",
                        prompt.chars().count()
                    ));
                }
                if prompt.chars().count() > PROMPT_MAX {
                    return Err(format!("system_prompt 不能超过 {PROMPT_MAX} 个字"));
                }
                prompt
            }
            _ => old_prompt.clone(),
        };
        let description = args
            .get("description")
            .and_then(|value| value.as_str())
            .map(|raw| raw.trim().chars().take(DESC_MAX).collect::<String>())
            .unwrap_or(old_desc.clone());
        let avatar = args
            .get("avatar")
            .and_then(|value| value.as_str())
            .map(|raw| raw.trim().chars().take(AVATAR_MAX).collect::<String>())
            .filter(|value| !value.is_empty())
            .unwrap_or(old_avatar.clone());
        let model = args
            .get("model")
            .and_then(|value| value.as_str())
            .map(|raw| raw.trim().chars().take(MODEL_MAX).collect::<String>())
            .unwrap_or(old_model.clone());
        let allowed_tools = match args.get("allowed_tools") {
            Some(value) if !value.is_null() => clean_allowed_tools(Some(value))?,
            _ => serde_json::from_str(&old_tools_json).unwrap_or_default(),
        };

        let allowed_json = serde_json::to_string(&allowed_tools).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE agents SET name = ?1, avatar = ?2, description = ?3, system_prompt = ?4,
                model = ?5, allowed_tools_json = ?6, updated_at = ?7 WHERE id = ?8",
            rusqlite::params![
                &name,
                &avatar,
                &description,
                &system_prompt,
                &model,
                &allowed_json,
                now_secs(),
                &id,
            ],
        )
        .map_err(|e| format!("更新智能体失败: {e}"))?;

        Ok(format!(
            "智能体「{name}」已更新（id={id}，后端 {backend}，角色设定 {} 字，免确认工具 {} 个）。",
            system_prompt.chars().count(),
            allowed_tools.len()
        ))
    })();
    result.unwrap_or_else(|e| e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_ids_are_url_safe() {
        let id = new_agent_id();
        assert!(id.starts_with("agent-"));
        assert!(id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn reserved_and_malformed_names_are_rejected() {
        assert!(normalize_name("claude").is_err());
        assert!(normalize_name("Codex").is_err());
        assert!(normalize_name("周报 助手").is_err());
        assert!(normalize_name("@管家").is_err());
        assert!(normalize_name("  ").is_err());
        assert_eq!(normalize_name(" 周报助手 ").unwrap(), "周报助手");
    }

    #[test]
    fn tool_grants_are_deduplicated_and_validated() {
        let tools = clean_allowed_tools(Some(&json!(["read_file", "read_file", "web_search"])))
            .unwrap();
        assert_eq!(tools, vec!["read_file", "web_search"]);
        assert!(clean_allowed_tools(Some(&json!(["run-command"]))).is_err());
    }
}
