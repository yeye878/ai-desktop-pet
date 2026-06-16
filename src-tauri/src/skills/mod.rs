use serde::{Deserialize, Serialize};

/// `keywords` 字段保留在数据结构 / 数据库里以便历史数据兼容，但前端不再使用，激活完全由用户手动控制。
/// AI 通过系统提示里"技能清单"知晓每个技能存在，按需调用对应工具；只有用户手动激活的技能会注入追加系统提示并合并工具免确认白名单。
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub keywords: Vec<String>,
    pub is_active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}
