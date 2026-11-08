use rusqlite::{Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::Skill;

struct BuiltinSkillSeed {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    allowed_tools: &'static [&'static str],
}

const BUILTIN_SKILLS: &[BuiltinSkillSeed] = &[
    BuiltinSkillSeed {
        id: "builtin-daily-briefing",
        name: "每日简报",
        description: "整理今天需要知道的天气、日程、待办和新闻要点。",
        system_prompt: "你是用户的每日简报助手。先确认用户想看的时间范围和主题；输出要短、分层、可执行。涉及今天、最新、新闻、价格、规则或其他可能变化的信息时，优先使用搜索或读取网页工具核实。可以结合记忆和定时任务提醒用户今天要做的事。不要编造实时信息。",
        allowed_tools: &[
            "get_weather",
            "list_scheduled_tasks",
            "search_memory",
            "web_search",
            "read_webpage",
        ],
    },
    BuiltinSkillSeed {
        id: "builtin-document-reader",
        name: "文档阅读",
        description: "阅读本地文件，提炼摘要、行动项、风险点和下一步。",
        system_prompt: "你是文档阅读助手。用户给出文件或目录后，先读取必要内容，再给出结构化摘要、关键事实、待办、疑点和建议。遇到长文档时先分块理解，再汇总；不要只根据文件名猜测内容。引用文件内容时保持简短，并说明来自哪个文件。",
        allowed_tools: &["read_file", "file_search", "list_directory", "search_memory"],
    },
    BuiltinSkillSeed {
        id: "builtin-translation-polish",
        name: "翻译润色",
        description: "中英互译、改写、压缩、扩写，并保持语气一致。",
        system_prompt: "你是翻译和润色助手。优先保留原意、语气、格式和专有名词。用户没有特别说明时，给出直接可用的译文或改写结果；必要时再附简短说明。对商务、技术、学术、口语场景要主动调整语域。不要过度解释。",
        allowed_tools: &["read_file"],
    },
    BuiltinSkillSeed {
        id: "builtin-code-partner",
        name: "代码搭档",
        description: "分析报错、阅读代码、设计修改方案和生成补丁建议。",
        system_prompt: "你是代码搭档。先理解现有项目结构和约束，再给出最小可行修改。排查问题时优先读取相关文件和错误日志；解释代码时给出文件位置和原因。除非用户明确要求，不要进行大范围重构。对不确定的库行为或最新 API，使用搜索核实。",
        allowed_tools: &[
            "read_file",
            "list_directory",
            "file_search",
            "web_search",
            "read_webpage",
        ],
    },
    BuiltinSkillSeed {
        id: "builtin-schedule-secretary",
        name: "日程秘书",
        description: "拆解任务、创建提醒、查看日程，并帮用户安排优先级。",
        system_prompt: "你是日程秘书。把用户的模糊安排转成清晰的时间、标题、备注和重复规则；时间不明确时先追问。创建提醒前复述关键信息，尽量使用简洁标题。查看已有日程时按紧急程度和时间顺序整理。",
        allowed_tools: &[
            "create_scheduled_task",
            "list_scheduled_tasks",
            "search_memory",
            "save_memory",
        ],
    },
    BuiltinSkillSeed {
        id: "builtin-browser-research",
        name: "浏览器研究",
        description: "打开网页、查看浏览器内容，做资料搜集和页面摘要。",
        system_prompt: "你是浏览器研究助手。需要网页信息时先搜索或打开目标页面，再读取网页或浏览器快照。给结论时区分事实、来源和推断；涉及最新信息必须核实。不要在没有用户授权的情况下提交表单、购买、登录或执行不可逆操作。",
        allowed_tools: &[
            "web_search",
            "read_webpage",
            "browser_open",
            "browser_navigate",
            "browser_snapshot",
            "browser_extract",
            "browser_click",
            "browser_type",
            "browser_press",
            "browser_scroll",
            "browser_back",
            "browser_forward",
        ],
    },
    BuiltinSkillSeed {
        id: "builtin-file-organizer",
        name: "文件整理",
        description: "扫描目录、查找文件、提出命名和归档方案。",
        system_prompt: "你是文件整理助手。先查看目录和文件名，必要时读取少量内容判断类别；然后给出清晰的整理方案、命名规则和可执行步骤。默认不要删除、覆盖或移动文件；如果用户要求实际执行，先列出将要改变的路径并等待确认。",
        allowed_tools: &["list_directory", "file_search", "read_file"],
    },
];

/// 暴露给 lib.rs：判断指定 id 是否为内置技能 id，用于在 save_skill 中保留 `builtin-` 命名空间。
pub fn is_builtin_skill_id(id: &str) -> bool {
    BUILTIN_SKILLS.iter().any(|s| s.id == id)
}

#[derive(Debug, Serialize, Clone)]
pub struct ChatMessage {
    /// 全局自增主键：前端用它做稳定 key，也用于按 id ASC 还原顺序。
    pub id: i64,
    /// 所属会话 id；旧消息（迁移前）落到默认会话。
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quoted_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_avatar: Option<String>,
}

/// 迁移前保存的历史消息统一归到这个会话。
pub const DEFAULT_CHAT_SESSION_ID: &str = "session-default";
/// 当前活跃主会话 id 在 pet_settings 里的 key（前端启动时由 get_active_chat_session 读取）。
pub const ACTIVE_MAIN_SESSION_KEY: &str = "active_main_session_id";
/// 主对话槽位名；chat_sessions.slot 的取值之一。
pub const MAIN_CHAT_SLOT: &str = "main";

/// 一个对话会话：主对话窗口（kind=main/slot=main）与三个 agent 窗口
/// （kind=agent/slot=claude|codex|dsh）共用同一种结构。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub slot: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_avatar: String,
    pub model: String,
    /// 各 CLI 后端的续接会话 id：{ "claude_code": "...", "codex": "...", "dsh": "..." }。
    /// 只给后端用，不返回给前端。
    #[serde(default, skip_serializing)]
    pub cli_sessions: HashMap<String, String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub message_count: i64,
    #[serde(default)]
    pub preview: String,
}

impl ChatSession {
    pub fn cli_session(&self, backend: &str) -> Option<&str> {
        self.cli_sessions
            .get(backend)
            .map(String::as_str)
            .filter(|id| !id.trim().is_empty())
    }
}

/// 新建会话的入参；命令层填好默认值（标题 / 绑定的内置 agent）后落库。
#[derive(Debug, Clone, Default)]
pub struct NewChatSession {
    pub title: String,
    pub kind: String,
    pub slot: String,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_avatar: String,
    pub model: String,
}

impl NewChatSession {
    pub fn for_slot(slot: &str) -> Self {
        let kind = if slot == MAIN_CHAT_SLOT { "main" } else { "agent" };
        Self {
            slot: slot.to_string(),
            kind: kind.to_string(),
            ..Self::default()
        }
    }

    pub fn with_agent(mut self, agent: &Agent) -> Self {
        self.agent_id = agent.id.clone();
        self.agent_name = agent.name.clone();
        self.agent_avatar = agent.avatar.clone();
        self
    }
}

/// 用户自定义的智能体（可在对话中通过 @名称 唤起）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub description: String,
    pub system_prompt: String,
    pub model: String,
    #[serde(default = "default_agent_backend")]
    pub backend: String,
    #[serde(default)]
    pub api_profile_id: String,
    #[serde(default)]
    pub is_builtin: bool,
    pub allowed_tools: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_agent_backend() -> String {
    "direct_api".into()
}

#[derive(Debug, Serialize)]
pub struct ClipboardItem {
    pub id: i64,
    pub content: String,
    pub pinned: bool,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct MemoryItem {
    pub id: i64,
    pub category: String,
    pub key: String,
    pub value: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRepeat {
    Once,
    Daily,
    Weekly,
    Monthly,
}

impl TaskRepeat {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskRepeat::Once => "once",
            TaskRepeat::Daily => "daily",
            TaskRepeat::Weekly => "weekly",
            TaskRepeat::Monthly => "monthly",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "daily" | "day" | "每天" => TaskRepeat::Daily,
            "weekly" | "week" | "每周" => TaskRepeat::Weekly,
            "monthly" | "month" | "每月" => TaskRepeat::Monthly,
            _ => TaskRepeat::Once,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct ScheduledTask {
    pub id: i64,
    pub title: String,
    pub note: String,
    pub due_at: i64,
    pub repeat: String,
    pub enabled: bool,
    pub last_triggered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewScheduledTask<'a> {
    pub title: &'a str,
    pub note: &'a str,
    pub due_at: i64,
    pub repeat: TaskRepeat,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct CustomPetAsset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub manifest: String,
    pub sprite_path: String,
    pub preview_path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let db = Self { conn };
        db.configure_pragmas()?;
        db.init_tables()?;
        db.ensure_indexes()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.configure_pragmas()?;
        db.init_tables()?;
        db.ensure_indexes()?;
        Ok(db)
    }

    fn configure_pragmas(&self) -> Result<()> {
        self.conn.pragma_update(None, "journal_mode", "WAL")?;
        self.conn.pragma_update(None, "synchronous", "NORMAL")?;
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(())
    }

    fn ensure_indexes(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_pet_memory_category_key ON pet_memory(category, key);
             CREATE INDEX IF NOT EXISTS idx_chat_history_created_at ON chat_history(created_at);
             CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_enabled_due ON scheduled_tasks(enabled, due_at);
             CREATE INDEX IF NOT EXISTS idx_clipboard_items_pinned_id ON clipboard_items(pinned DESC, id DESC);
             CREATE INDEX IF NOT EXISTS idx_skills_is_active ON skills(is_active) WHERE is_active = 1;
             CREATE INDEX IF NOT EXISTS idx_chat_sessions_slot_updated ON chat_sessions(slot, updated_at DESC);
             CREATE INDEX IF NOT EXISTS idx_chat_history_session_id ON chat_history(session_id, id);",
        )?;
        Ok(())
    }

    fn init_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chat_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                session_id TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS pet_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS clipboard_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS pet_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                note TEXT NOT NULL DEFAULT '',
                due_at INTEGER NOT NULL,
                repeat TEXT NOT NULL DEFAULT 'once',
                enabled INTEGER NOT NULL DEFAULT 1,
                last_triggered_at INTEGER,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS custom_pet_assets (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                manifest TEXT NOT NULL,
                sprite_path TEXT NOT NULL,
                preview_path TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                system_prompt TEXT NOT NULL DEFAULT '',
                allowed_tools_json TEXT NOT NULL DEFAULT '[]',
                keywords_json TEXT NOT NULL DEFAULT '[]',
                is_active INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TRIGGER IF NOT EXISTS skills_single_active_update
            BEFORE UPDATE OF is_active ON skills
            WHEN NEW.is_active = 1
            BEGIN
                UPDATE skills SET is_active = 0 WHERE id != NEW.id;
            END;
            CREATE TRIGGER IF NOT EXISTS skills_single_active_insert
            BEFORE INSERT ON skills
            WHEN NEW.is_active = 1
            BEGIN
                UPDATE skills SET is_active = 0;
            END;
            CREATE TABLE IF NOT EXISTS deleted_builtin_skills (
                id TEXT PRIMARY KEY,
                deleted_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                avatar TEXT NOT NULL DEFAULT '🤖',
                description TEXT NOT NULL DEFAULT '',
                system_prompt TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                allowed_tools_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE TABLE IF NOT EXISTS chat_sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL DEFAULT 'main',
                slot TEXT NOT NULL DEFAULT 'main',
                agent_id TEXT NOT NULL DEFAULT '',
                agent_name TEXT NOT NULL DEFAULT '',
                agent_avatar TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                cli_sessions_json TEXT NOT NULL DEFAULT '{}',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );",
        )?;
        self.ensure_chat_thinking_column()?;
        self.ensure_chat_quote_columns()?;
        self.ensure_chat_agent_columns()?;
        self.ensure_chat_session_column()?;
        self.ensure_clipboard_pinned_column()?;
        self.ensure_scheduled_tasks_columns()?;
        self.init_chat_sessions()?;
        self.ensure_builtin_skills()?;
        Ok(())
    }

    fn ensure_chat_thinking_column(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(chat_history)")?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

        for column in columns {
            if column? == "thinking" {
                return Ok(());
            }
        }

        self.conn
            .execute("ALTER TABLE chat_history ADD COLUMN thinking TEXT", [])?;
        Ok(())
    }

    fn ensure_chat_quote_columns(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(chat_history)")?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;

        for (name, definition) in [
            ("quoted_role", "TEXT"),
            ("quoted_content", "TEXT"),
        ] {
            if !columns.iter().any(|column| column == name) {
                self.conn.execute(
                    &format!("ALTER TABLE chat_history ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    fn ensure_chat_agent_columns(&self) -> Result<()> {
        let agent_columns = self.conn.prepare("PRAGMA table_info(agents)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;
        for (name, default) in [("backend", "direct_api"), ("api_profile_id", "")] {
            if !agent_columns.iter().any(|column| column == name) {
                self.conn.execute(&format!("ALTER TABLE agents ADD COLUMN {name} TEXT NOT NULL DEFAULT '{default}'"), [])?;
            }
        }
        let mut stmt = self.conn.prepare("PRAGMA table_info(chat_history)")?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;

        for name in ["agent_id", "agent_name", "agent_avatar"] {
            if !columns.iter().any(|column| column == name) {
                self.conn.execute(
                    &format!("ALTER TABLE chat_history ADD COLUMN {name} TEXT"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    fn ensure_chat_session_column(&self) -> Result<()> {
        let columns = self
            .conn
            .prepare("PRAGMA table_info(chat_history)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;
        if !columns.iter().any(|column| column == "session_id") {
            self.conn.execute(
                "ALTER TABLE chat_history ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        Ok(())
    }

    /// chat_sessions 迁移：
    /// 1. 表为空时插入默认会话（session-default）；
    /// 2. 旧 chat_history 行（session_id='')归入默认会话；
    /// 3. 修正 pet_settings.active_main_session_id，指向一个真实存在的主会话。
    fn init_chat_sessions(&self) -> Result<()> {
        let empty: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chat_sessions", [], |row| row.get(0))?;
        if empty == 0 {
            self.insert_default_session()?;
        }
        self.conn.execute(
            "UPDATE chat_history SET session_id = ?1 WHERE session_id = ''",
            [DEFAULT_CHAT_SESSION_ID],
        )?;
        self.ensure_active_main_session()?;
        Ok(())
    }

    fn insert_default_session(&self) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO chat_sessions (id, title, kind, slot, created_at, updated_at)
             VALUES (?1, '默认对话', 'main', 'main', strftime('%s', 'now'), strftime('%s', 'now'))",
            [DEFAULT_CHAT_SESSION_ID],
        )?;
        Ok(())
    }

    /// 保证至少存在一个主会话；返回最新主会话 id（优先默认会话）。
    /// 表里只有 agent 会话时会补建默认主会话。
    pub fn ensure_default_session(&self) -> Result<String> {
        if let Some(id) = self.latest_main_session_id()? {
            return Ok(id);
        }
        self.insert_default_session()?;
        Ok(DEFAULT_CHAT_SESSION_ID.to_string())
    }

    fn latest_main_session_id(&self) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT id FROM chat_sessions
                 WHERE slot = ?1
                 ORDER BY updated_at DESC, rowid DESC
                 LIMIT 1",
                [MAIN_CHAT_SLOT],
                |row| row.get::<_, String>(0),
            )
            .optional()
    }

    /// 读取 setting 里的活跃主会话；失效（为空 / 不存在 / 不是主会话）时回退到
    /// 最新主会话（必要时重建默认会话）并写回 setting。
    pub fn ensure_active_main_session(&self) -> Result<String> {
        if let Some(id) = self.get_setting(ACTIVE_MAIN_SESSION_KEY)? {
            let id = id.trim().to_string();
            if !id.is_empty() {
                if let Some(session) = self.get_chat_session(&id)? {
                    if session.slot == MAIN_CHAT_SLOT {
                        return Ok(id);
                    }
                }
            }
        }
        let id = self.ensure_default_session()?;
        self.save_setting(ACTIVE_MAIN_SESSION_KEY, &id)?;
        Ok(id)
    }

    /// 当前活跃主会话 id（不修正、不建会话；启动迁移后一定存在）。
    pub fn active_main_session_id(&self) -> Result<Option<String>> {
        self.get_setting(ACTIVE_MAIN_SESSION_KEY)
            .map(|value| value.map(|id| id.trim().to_string()).filter(|id| !id.is_empty()))
    }

    fn ensure_clipboard_pinned_column(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(clipboard_items)")?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

        for column in columns {
            if column? == "pinned" {
                return Ok(());
            }
        }

        self.conn.execute(
            "ALTER TABLE clipboard_items ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        Ok(())
    }

    fn ensure_scheduled_tasks_columns(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(scheduled_tasks)")?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>>>()?;

        let required = [
            ("title", "TEXT NOT NULL DEFAULT ''"),
            ("note", "TEXT NOT NULL DEFAULT ''"),
            ("due_at", "INTEGER NOT NULL DEFAULT 0"),
            ("repeat", "TEXT NOT NULL DEFAULT 'once'"),
            ("enabled", "INTEGER NOT NULL DEFAULT 1"),
            ("last_triggered_at", "INTEGER"),
            ("created_at", "INTEGER NOT NULL DEFAULT 0"),
            ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
        ];

        for (name, definition) in required {
            if !columns.iter().any(|column| column == name) {
                self.conn.execute(
                    &format!("ALTER TABLE scheduled_tasks ADD COLUMN {name} {definition}"),
                    [],
                )?;
            }
        }

        Ok(())
    }

    fn ensure_builtin_skills(&self) -> Result<()> {
        for skill in BUILTIN_SKILLS {
            // 跳过用户主动删除过的内置技能，避免每次启动又把它插回来。
            let deleted: Option<String> = self
                .conn
                .query_row(
                    "SELECT id FROM deleted_builtin_skills WHERE id = ?1",
                    [skill.id],
                    |row| row.get(0),
                )
                .optional()?;
            if deleted.is_some() {
                continue;
            }

            let allowed_tools_json = serde_json::to_string(&skill.allowed_tools)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            self.conn.execute(
                "INSERT OR IGNORE INTO skills (
                    id, name, description, system_prompt,
                    allowed_tools_json, keywords_json,
                    created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, '[]',
                    strftime('%s', 'now'), strftime('%s', 'now')
                 )",
                (
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.system_prompt,
                    &allowed_tools_json,
                ),
            )?;
        }
        Ok(())
    }

    // ===== 对话历史 =====

    pub fn save_message(&self, role: &str, content: &str) -> Result<()> {
        self.save_message_with_thinking(role, content, None)
    }

    pub fn save_message_with_thinking(
        &self,
        role: &str,
        content: &str,
        thinking: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (role, content, thinking) VALUES (?1, ?2, ?3)",
            (role, content, thinking),
        )?;
        Ok(())
    }

    /// 保存带引用信息的用户消息（quoted_role/quoted_content 记录被引用的前文对话）
    pub fn save_message_with_quote(
        &self,
        role: &str,
        content: &str,
        thinking: Option<&str>,
        quoted_role: Option<&str>,
        quoted_content: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (role, content, thinking, quoted_role, quoted_content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (role, content, thinking, quoted_role, quoted_content),
        )?;
        Ok(())
    }

    /// 保存带智能体归属的消息（agent_id/agent_name/agent_avatar 标识回复来自哪个智能体）
    pub fn save_message_with_agent(
        &self,
        role: &str,
        content: &str,
        thinking: Option<&str>,
        agent_id: Option<&str>,
        agent_name: Option<&str>,
        agent_avatar: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (role, content, thinking, agent_id, agent_name, agent_avatar)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (role, content, thinking, agent_id, agent_name, agent_avatar),
        )?;
        Ok(())
    }

    // ===== 对话历史（按会话）=====
    // 生产路径都走 *_in 版本；旧方法写 session_id=''，只为保留既有单测/兼容代码。

    pub fn save_message_in(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        self.save_message_with_thinking_in(session_id, role, content, None)
    }

    pub fn save_message_with_thinking_in(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        thinking: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (session_id, role, content, thinking) VALUES (?1, ?2, ?3, ?4)",
            (session_id, role, content, thinking),
        )?;
        Ok(())
    }

    /// 保存带引用信息的用户消息（quoted_role/quoted_content 记录被引用的前文对话）。
    pub fn save_message_with_quote_in(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        thinking: Option<&str>,
        quoted_role: Option<&str>,
        quoted_content: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (session_id, role, content, thinking, quoted_role, quoted_content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (session_id, role, content, thinking, quoted_role, quoted_content),
        )?;
        Ok(())
    }

    /// 保存带智能体归属的消息（agent_id/agent_name/agent_avatar 标识回复来自哪个智能体）。
    pub fn save_message_with_agent_in(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        thinking: Option<&str>,
        agent_id: Option<&str>,
        agent_name: Option<&str>,
        agent_avatar: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_history (session_id, role, content, thinking, agent_id, agent_name, agent_avatar)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (session_id, role, content, thinking, agent_id, agent_name, agent_avatar),
        )?;
        Ok(())
    }

    pub fn get_recent_messages(&self, limit: u32) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at, quoted_role, quoted_content,
                    agent_id, agent_name, agent_avatar, id, session_id
             FROM chat_history
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(ChatMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                thinking: row.get(2)?,
                created_at: row.get(3)?,
                quoted_role: row.get(4)?,
                quoted_content: row.get(5)?,
                agent_id: row.get(6)?,
                agent_name: row.get(7)?,
                agent_avatar: row.get(8)?,
                id: row.get(9)?,
                session_id: row.get(10)?,
            })
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse();
        Ok(messages)
    }

    pub fn clear_messages(&self) -> Result<()> {
        self.conn.execute("DELETE FROM chat_history", ())?;
        Ok(())
    }

    /// 清空某个会话的历史消息（保留会话本身）。
    pub fn clear_session_messages(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM chat_history WHERE session_id = ?1", [session_id])?;
        Ok(())
    }

    /// 只返回 id > after_id 的最近 limit 条消息（用于 AI 上下文隔离）
    pub fn get_recent_messages_after(&self, after_id: i64, limit: u32) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at, quoted_role, quoted_content,
                    agent_id, agent_name, agent_avatar, id, session_id
             FROM chat_history
             WHERE id > ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map([after_id, limit as i64], |row| {
            Ok(ChatMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                thinking: row.get(2)?,
                created_at: row.get(3)?,
                quoted_role: row.get(4)?,
                quoted_content: row.get(5)?,
                agent_id: row.get(6)?,
                agent_name: row.get(7)?,
                agent_avatar: row.get(8)?,
                id: row.get(9)?,
                session_id: row.get(10)?,
            })
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse();
        Ok(messages)
    }

    /// Every visible message in the current conversation, without a last-N cutoff.
    pub fn get_conversation_messages(&self, after_id: i64) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at, quoted_role, quoted_content,
                    agent_id, agent_name, agent_avatar, id, session_id
             FROM chat_history WHERE id > ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([after_id], |row| Ok(ChatMessage {
            role: row.get(0)?, content: row.get(1)?, thinking: row.get(2)?, created_at: row.get(3)?,
            quoted_role: row.get(4)?, quoted_content: row.get(5)?, agent_id: row.get(6)?,
            agent_name: row.get(7)?, agent_avatar: row.get(8)?,
            id: row.get(9)?, session_id: row.get(10)?,
        }))?;
        rows.collect()
    }

    /// 返回 chat_history 表中最大的 id，无消息时返回 0
    pub fn get_max_message_id(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM chat_history", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.into())
    }

    /// 某个会话的全部可见消息（默认最多 500 条），按 id ASC 返回。
    pub fn get_session_messages(
        &self,
        session_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<ChatMessage>> {
        let limit = limit.unwrap_or(500).clamp(1, 5_000);
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at, quoted_role, quoted_content,
                    agent_id, agent_name, agent_avatar, id, session_id
             FROM (
                SELECT * FROM chat_history
                WHERE session_id = ?1
                ORDER BY id DESC
                LIMIT ?2
             )
             ORDER BY id ASC",
        )?;
        let rows = stmt.query_map((session_id, limit as i64), |row| {
            Ok(ChatMessage {
                role: row.get(0)?,
                content: row.get(1)?,
                thinking: row.get(2)?,
                created_at: row.get(3)?,
                quoted_role: row.get(4)?,
                quoted_content: row.get(5)?,
                agent_id: row.get(6)?,
                agent_name: row.get(7)?,
                agent_avatar: row.get(8)?,
                id: row.get(9)?,
                session_id: row.get(10)?,
            })
        })?;
        rows.collect()
    }

    // ===== 会话（chat_sessions）=====

    pub fn create_chat_session(&self, new: NewChatSession) -> Result<ChatSession> {
        let kind = match new.kind.trim() {
            "" if new.slot.trim() == MAIN_CHAT_SLOT => "main",
            "" => "agent",
            kind => kind,
        };
        let slot = match new.slot.trim() {
            "" => MAIN_CHAT_SLOT,
            slot => slot,
        };
        let title = match new.title.trim() {
            "" => "新对话",
            title => title,
        };
        let id = format!("session-{}", uuid::Uuid::new_v4());
        self.conn.execute(
            "INSERT INTO chat_sessions (
                id, title, kind, slot, agent_id, agent_name, agent_avatar, model,
                cli_sessions_json, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}',
                strftime('%s', 'now'), strftime('%s', 'now')
             )",
            (
                id.as_str(),
                title,
                kind,
                slot,
                new.agent_id.trim(),
                new.agent_name.trim(),
                new.agent_avatar.trim(),
                new.model.trim(),
            ),
        )?;
        self.get_chat_session(&id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// 会话列表（updated_at DESC）。slot 为空时返回全部槽位。
    /// message_count 与 preview 由 chat_history 实时算出。
    pub fn list_chat_sessions(&self, slot: Option<&str>) -> Result<Vec<ChatSession>> {
        let slot = slot.map(str::trim).filter(|slot| !slot.is_empty());
        let sql = match slot {
            Some(_) => format!(
                "{CHAT_SESSION_SELECT} WHERE s.slot = ?1 ORDER BY s.updated_at DESC, s.rowid DESC"
            ),
            None => {
                format!("{CHAT_SESSION_SELECT} ORDER BY s.updated_at DESC, s.rowid DESC")
            }
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if let Some(slot) = slot {
            stmt.query_map([slot], chat_session_from_row)?
                .collect::<Result<Vec<_>>>()?
        } else {
            stmt.query_map([], chat_session_from_row)?
                .collect::<Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    pub fn get_chat_session(&self, id: &str) -> Result<Option<ChatSession>> {
        let sql = format!("{CHAT_SESSION_SELECT} WHERE s.id = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map([id], chat_session_from_row)?;
        match rows.next() {
            Some(Ok(session)) => Ok(Some(session)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }

    /// 更新会话的标题 / 绑定 / 模型 / CLI 续接 id；同时刷新 updated_at。
    /// 每个 Option 为 None 表示不动该字段。
    pub fn update_chat_session(
        &self,
        id: &str,
        title: Option<&str>,
        agent: Option<(&str, &str, &str)>,
        model: Option<&str>,
        cli_sessions: Option<&HashMap<String, String>>,
    ) -> Result<ChatSession> {
        if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
            self.conn.execute(
                "UPDATE chat_sessions SET title = ?1 WHERE id = ?2",
                (title, id),
            )?;
        }
        if let Some((agent_id, agent_name, agent_avatar)) = agent {
            self.conn.execute(
                "UPDATE chat_sessions
                 SET agent_id = ?1, agent_name = ?2, agent_avatar = ?3
                 WHERE id = ?4",
                (agent_id, agent_name, agent_avatar, id),
            )?;
        }
        if let Some(model) = model {
            self.conn.execute(
                "UPDATE chat_sessions SET model = ?1 WHERE id = ?2",
                (model.trim(), id),
            )?;
        }
        if let Some(cli_sessions) = cli_sessions {
            let json = serde_json::to_string(cli_sessions)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            self.conn.execute(
                "UPDATE chat_sessions SET cli_sessions_json = ?1 WHERE id = ?2",
                (json, id),
            )?;
        }
        self.conn.execute(
            "UPDATE chat_sessions SET updated_at = strftime('%s', 'now') WHERE id = ?1",
            [id],
        )?;
        self.get_chat_session(id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// 删除会话及其全部历史消息。
    pub fn delete_chat_session(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM chat_history WHERE session_id = ?1", [id])?;
        self.conn
            .execute("DELETE FROM chat_sessions WHERE id = ?1", [id])?;
        Ok(())
    }

    /// 消息落库后的会话更新：刷新 updated_at；标题还是占位标题时用首条消息生成。
    /// preview 不落库（列表由 chat_history 实时计算），只在标题为空时兜底。
    pub fn touch_chat_session(
        &self,
        id: &str,
        title_if_empty: Option<&str>,
        preview: Option<&str>,
    ) -> Result<()> {
        let fallback = title_if_empty
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .or_else(|| preview.map(str::trim).filter(|preview| !preview.is_empty()));
        let current_title: Option<String> = self
            .conn
            .query_row("SELECT title FROM chat_sessions WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(current_title) = current_title else {
            return Ok(());
        };
        // 用户手动改过的标题永远保留；只覆盖空标题与「新对话 N」这类占位标题。
        let replace_title = fallback
            .filter(|title| is_placeholder_session_title(&current_title) && !title.is_empty());
        match replace_title {
            Some(title) => {
                self.conn.execute(
                    "UPDATE chat_sessions
                     SET title = ?1, updated_at = strftime('%s', 'now')
                     WHERE id = ?2",
                    (title, id),
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE chat_sessions SET updated_at = strftime('%s', 'now') WHERE id = ?1",
                    [id],
                )?;
            }
        }
        Ok(())
    }

    /// 清空所有会话与历史，重建默认主会话；返回新的活跃主会话 id。
    pub fn clear_all_chat_sessions(&self) -> Result<String> {
        self.conn.execute("DELETE FROM chat_history", [])?;
        self.conn.execute("DELETE FROM chat_sessions", [])?;
        self.conn
            .execute("DELETE FROM pet_settings WHERE key = ?1", [ACTIVE_MAIN_SESSION_KEY])?;
        self.insert_default_session()?;
        self.save_setting(ACTIVE_MAIN_SESSION_KEY, DEFAULT_CHAT_SESSION_ID)?;
        Ok(DEFAULT_CHAT_SESSION_ID.to_string())
    }

    /// 主会话数量，用于「新对话 N」的递增标题。
    pub fn count_main_sessions(&self) -> Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM chat_sessions WHERE slot = ?1",
            [MAIN_CHAT_SLOT],
            |row| row.get(0),
        )
    }

    // ===== 宠物记忆 =====

    pub fn save_memory(&self, category: &str, key: &str, value: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO pet_memory (category, key, value) VALUES (?1, ?2, ?3)",
            (category, key, value),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_memory(&self, category: &str, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM pet_memory WHERE category = ?1 AND key = ?2")?;
        let mut rows = stmt.query_map((category, key), |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(val)) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    pub fn get_memory_by_id(&self, id: i64) -> Result<Option<MemoryItem>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, category, key, value, created_at FROM pet_memory WHERE id = ?1")?;
        let mut rows = stmt.query_map([id], |row| {
            Ok(MemoryItem {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    pub fn get_all_memories(&self) -> Result<Vec<MemoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, category, key, value, created_at FROM pet_memory ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(MemoryItem {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_memories_by_category(&self, category: &str) -> Result<Vec<MemoryItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE category = ?1
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([category], |row| {
            Ok(MemoryItem {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn delete_memory(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM pet_memory WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn search_memories(
        &self,
        query: &str,
        category: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MemoryItem>> {
        let limit = limit.min(30);
        let like_pattern = format!("%{}%", escape_like_pattern(query));
        // `escape_like_pattern()` 会给 % _ \ 加反斜杠，所以 LIKE 必须配套 `ESCAPE '\'`：
        // 少了 ESCAPE 子句，反斜杠会被当作普通字符参与匹配，含这三个字符的关键词静默 0 命中。
        let sql = if category.is_some() {
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 ESCAPE '\\' OR value LIKE ?1 ESCAPE '\\')
               AND category = ?2
             ORDER BY id DESC
             LIMIT ?3"
        } else {
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 ESCAPE '\\' OR value LIKE ?1 ESCAPE '\\')
             ORDER BY id DESC
             LIMIT ?3"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![like_pattern, category, limit], |row| {
            Ok(MemoryItem {
                id: row.get(0)?,
                category: row.get(1)?,
                key: row.get(2)?,
                value: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    // ===== 定时任务 =====

    pub fn save_scheduled_task(&self, task: NewScheduledTask<'_>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO scheduled_tasks (title, note, due_at, repeat, enabled, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s', 'now'))",
            (
                task.title,
                task.note,
                task.due_at,
                task.repeat.as_str(),
                if task.enabled { 1 } else { 0 },
            ),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_scheduled_task_by_id(&self, id: i64) -> Result<Option<ScheduledTask>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, note, due_at, repeat, enabled, last_triggered_at, created_at, updated_at
             FROM scheduled_tasks
             WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map([id], scheduled_task_from_row)?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    pub fn get_all_scheduled_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, note, due_at, repeat, enabled, last_triggered_at, created_at, updated_at
             FROM scheduled_tasks
             ORDER BY enabled DESC, due_at ASC, id DESC",
        )?;
        let rows = stmt.query_map([], scheduled_task_from_row)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_due_scheduled_tasks(&self, now: i64) -> Result<Vec<ScheduledTask>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, note, due_at, repeat, enabled, last_triggered_at, created_at, updated_at
             FROM scheduled_tasks
             WHERE enabled = 1 AND due_at <= ?1
             ORDER BY due_at ASC, id ASC
             LIMIT 20",
        )?;
        let rows = stmt.query_map([now], scheduled_task_from_row)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn set_scheduled_task_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE scheduled_tasks
             SET enabled = ?1, updated_at = strftime('%s', 'now')
             WHERE id = ?2",
            (if enabled { 1 } else { 0 }, id),
        )?;
        Ok(())
    }

    pub fn reschedule_triggered_task(
        &self,
        id: i64,
        next_due_at: Option<i64>,
        triggered_at: i64,
    ) -> Result<()> {
        match next_due_at {
            Some(next_due_at) => {
                self.conn.execute(
                    "UPDATE scheduled_tasks
                     SET due_at = ?1,
                         last_triggered_at = ?2,
                         updated_at = strftime('%s', 'now')
                     WHERE id = ?3",
                    (next_due_at, triggered_at, id),
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE scheduled_tasks
                     SET enabled = 0,
                         last_triggered_at = ?1,
                         updated_at = strftime('%s', 'now')
                     WHERE id = ?2",
                    (triggered_at, id),
                )?;
            }
        }
        Ok(())
    }

    pub fn delete_scheduled_task(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM scheduled_tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    // ===== 剪切板 =====

    pub fn save_clipboard_item(&self, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO clipboard_items (content) VALUES (?1)",
            [content],
        )?;
        Ok(())
    }

    pub fn get_clipboard_items(&self, limit: u32) -> Result<Vec<ClipboardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, pinned, created_at
             FROM clipboard_items
             ORDER BY pinned DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(ClipboardItem {
                id: row.get(0)?,
                content: row.get(1)?,
                pinned: row.get::<_, i64>(2)? != 0,
                created_at: row.get(3)?,
            })
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    pub fn delete_clipboard_item(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM clipboard_items WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_clipboard_item_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items SET pinned = ?1 WHERE id = ?2",
            (if pinned { 1 } else { 0 }, id),
        )?;
        Ok(())
    }

    pub fn clear_clipboard_items(&self) -> Result<()> {
        self.conn.execute("DELETE FROM clipboard_items", ())?;
        Ok(())
    }

    // ===== 设置 =====

    // ===== Custom pet assets =====

    pub fn save_custom_pet_asset(&self, asset: &CustomPetAsset) -> Result<()> {
        self.conn.execute(
            "INSERT INTO custom_pet_assets (
                id, name, kind, manifest, sprite_path, preview_path, created_at, updated_at
             )
             VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, strftime('%s', 'now'), strftime('%s', 'now')
             )
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                manifest = excluded.manifest,
                sprite_path = excluded.sprite_path,
                preview_path = excluded.preview_path,
                updated_at = excluded.updated_at",
            (
                &asset.id,
                &asset.name,
                &asset.kind,
                &asset.manifest,
                &asset.sprite_path,
                &asset.preview_path,
            ),
        )?;
        Ok(())
    }

    pub fn get_custom_pet_asset(&self, id: &str) -> Result<Option<CustomPetAsset>> {
        self.conn
            .query_row(
                "SELECT id, name, kind, manifest, sprite_path, preview_path, created_at, updated_at
                 FROM custom_pet_assets
                 WHERE id = ?1",
                [id],
                custom_pet_asset_from_row,
            )
            .optional()
    }

    pub fn list_custom_pet_assets(&self) -> Result<Vec<CustomPetAsset>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, kind, manifest, sprite_path, preview_path, created_at, updated_at
             FROM custom_pet_assets
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([], custom_pet_asset_from_row)?;
        let mut assets = Vec::new();
        for row in rows {
            assets.push(row?);
        }
        Ok(assets)
    }

    pub fn delete_custom_pet_asset(&self, id: &str) -> Result<Option<CustomPetAsset>> {
        let existing = self.get_custom_pet_asset(id)?;
        self.conn
            .execute("DELETE FROM custom_pet_assets WHERE id = ?1", [id])?;
        Ok(existing)
    }

    pub fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO pet_settings (key, value, updated_at)
             VALUES (?1, ?2, strftime('%s', 'now'))
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at",
            (key, value),
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM pet_settings WHERE key = ?1")?;
        let mut rows = stmt.query_map([key], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(val)) => Ok(Some(val)),
            _ => Ok(None),
        }
    }

    // ===== 技能 =====

    pub fn list_skills(&self) -> Result<Vec<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, system_prompt,
                    allowed_tools_json, keywords_json, is_active,
                    created_at, updated_at
             FROM skills
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], skill_from_row)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_active_skill(&self) -> Result<Option<Skill>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, system_prompt,
                    allowed_tools_json, keywords_json, is_active,
                    created_at, updated_at
             FROM skills
             WHERE is_active = 1
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], skill_from_row)?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    /// 保存 / 更新技能（**不会**改动 `is_active`，激活走 `set_active_skill`）
    pub fn save_skill(&self, skill: &Skill) -> Result<()> {
        let allowed_tools_json = serde_json::to_string(&skill.allowed_tools)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let keywords_json = serde_json::to_string(&skill.keywords)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT INTO skills (
                id, name, description, system_prompt,
                allowed_tools_json, keywords_json,
                created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s', 'now')
             )
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                system_prompt = excluded.system_prompt,
                allowed_tools_json = excluded.allowed_tools_json,
                keywords_json = excluded.keywords_json,
                updated_at = excluded.updated_at",
            (
                &skill.id,
                &skill.name,
                &skill.description,
                &skill.system_prompt,
                &allowed_tools_json,
                &keywords_json,
                skill.created_at,
            ),
        )?;
        Ok(())
    }

    pub fn delete_skill(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM skills WHERE id = ?1", [id])?;
        // 删除的是内置技能时写墓碑，防止下次启动 ensure_builtin_skills 复活它。
        if id.starts_with("builtin-") {
            self.conn.execute(
                "INSERT OR IGNORE INTO deleted_builtin_skills (id) VALUES (?1)",
                [id],
            )?;
        }
        Ok(())
    }

    /// 逃生口：清空删除墓碑并重新 seed 所有内置技能。
    /// 用于用户误删内置技能后一键恢复（不删除用户自定义技能，也不改变当前激活状态）。
    pub fn reset_builtin_skills(&self) -> Result<()> {
        self.conn.execute("DELETE FROM deleted_builtin_skills", [])?;
        self.ensure_builtin_skills()?;
        Ok(())
    }

    /// `id = Some(...)` 激活该技能（其它行会被 DB 触发器自动置 0），
    /// `id = None` 清除所有激活。
    pub fn set_active_skill(&self, id: Option<&str>) -> Result<()> {
        match id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE skills
                     SET is_active = CASE WHEN id = ?1 THEN 1 ELSE 0 END,
                         updated_at = strftime('%s', 'now')",
                    [id],
                )?;
            }
            None => {
                self.conn.execute(
                    "UPDATE skills
                     SET is_active = 0, updated_at = strftime('%s', 'now')",
                    [],
                )?;
            }
        }
        Ok(())
    }

    // ===== 智能体 =====

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, avatar, description, system_prompt,
                    model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             FROM agents
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], agent_from_row)?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    pub fn get_agent_by_id(&self, id: &str) -> Result<Option<Agent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, avatar, description, system_prompt,
                    model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             FROM agents
             WHERE id = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([id], agent_from_row)?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    pub fn get_agent_by_name(&self, name: &str) -> Result<Option<Agent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, avatar, description, system_prompt,
                    model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             FROM agents
             WHERE name = ?1
             LIMIT 1",
        )?;
        let mut rows = stmt.query_map([name], agent_from_row)?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }

    /// 按名称精确匹配返回智能体列表（保持 names 的顺序、去重）。
    pub fn get_agents_by_names(&self, names: &[String]) -> Result<Vec<Agent>> {
        let trimmed: Vec<String> = names
            .iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        // 去重但保持顺序
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<&String> = trimmed
            .iter()
            .filter(|name| seen.insert(name.to_string()))
            .collect();
        let placeholders = unique
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT id, name, avatar, description, system_prompt,
                    model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             FROM agents
             WHERE name IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(unique.iter().map(|name| name.as_str())),
            agent_from_row,
        )?;
        let mut found: Vec<Agent> = Vec::new();
        for row in rows {
            found.push(row?);
        }
        // 按 unique 的顺序重排
        found.sort_by_key(|agent| {
            unique
                .iter()
                .position(|name| name.as_str() == agent.name)
                .unwrap_or(usize::MAX)
        });
        Ok(found)
    }

    pub fn save_agent(&self, agent: &Agent) -> Result<()> {
        let allowed_tools_json = serde_json::to_string(&agent.allowed_tools)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        self.conn.execute(
            "INSERT INTO agents (
                id, name, avatar, description, system_prompt,
                model, allowed_tools_json, created_at, updated_at, backend, api_profile_id
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s', 'now'), ?9, ?10
             )
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                avatar = excluded.avatar,
                description = excluded.description,
                system_prompt = excluded.system_prompt,
                model = excluded.model,
                backend = excluded.backend,
                api_profile_id = excluded.api_profile_id,
                allowed_tools_json = excluded.allowed_tools_json,
                updated_at = excluded.updated_at",
            (
                &agent.id,
                &agent.name,
                &agent.avatar,
                &agent.description,
                &agent.system_prompt,
                &agent.model,
                &allowed_tools_json,
                agent.created_at,
                &agent.backend,
                &agent.api_profile_id,
            ),
        )?;
        Ok(())
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM agents WHERE id = ?1", [id])?;
        Ok(())
    }
}

/// 会话查询列：末尾两个子查询给出消息数与最后一条消息（预览）。
const CHAT_SESSION_SELECT: &str = "SELECT s.id, s.title, s.kind, s.slot, s.agent_id, s.agent_name, \
     s.agent_avatar, s.model, s.cli_sessions_json, s.created_at, s.updated_at, \
     (SELECT COUNT(*) FROM chat_history h WHERE h.session_id = s.id) AS message_count, \
     (SELECT h.content FROM chat_history h WHERE h.session_id = s.id ORDER BY h.id DESC LIMIT 1) AS preview \
     FROM chat_sessions s";

fn chat_session_from_row(row: &rusqlite::Row<'_>) -> Result<ChatSession> {
    let cli_sessions_json: String = row.get(8)?;
    Ok(ChatSession {
        id: row.get(0)?,
        title: row.get(1)?,
        kind: row.get(2)?,
        slot: row.get(3)?,
        agent_id: row.get(4)?,
        agent_name: row.get(5)?,
        agent_avatar: row.get(6)?,
        model: row.get(7)?,
        cli_sessions: serde_json::from_str(&cli_sessions_json).unwrap_or_default(),
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        message_count: row.get(11)?,
        preview: compact_preview(row.get::<_, Option<String>>(12)?),
    })
}

/// 会话标题是否还是占位标题（空 / 「默认对话」/「新对话」/「新对话 N」）。
/// 占位标题允许被首条消息覆盖；用户手动改过的标题永远保留。
fn is_placeholder_session_title(title: &str) -> bool {
    let title = title.trim();
    title.is_empty()
        || title == "默认对话"
        || title == "新对话"
        || title
            .strip_prefix("新对话 ")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
}

/// 会话列表预览：折叠空白后取前 60 个字符。
fn compact_preview(content: Option<String>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(60).collect()
}

fn agent_from_row(row: &rusqlite::Row<'_>) -> Result<Agent> {
    let allowed_tools_json: String = row.get(6)?;
    Ok(Agent {
        id: row.get(0)?,
        name: row.get(1)?,
        avatar: row.get(2)?,
        description: row.get(3)?,
        system_prompt: row.get(4)?,
        model: row.get(5)?,
        backend: row.get(9)?,
        api_profile_id: row.get(10)?,
        is_builtin: false,
        allowed_tools: serde_json::from_str(&allowed_tools_json).unwrap_or_default(),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn scheduled_task_from_row(row: &rusqlite::Row<'_>) -> Result<ScheduledTask> {
    Ok(ScheduledTask {
        id: row.get(0)?,
        title: row.get(1)?,
        note: row.get(2)?,
        due_at: row.get(3)?,
        repeat: TaskRepeat::from_str(&row.get::<_, String>(4)?)
            .as_str()
            .to_string(),
        enabled: row.get::<_, i64>(5)? != 0,
        last_triggered_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn escape_like_pattern(input: &str) -> String {
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

fn custom_pet_asset_from_row(row: &rusqlite::Row<'_>) -> Result<CustomPetAsset> {
    Ok(CustomPetAsset {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        manifest: row.get(3)?,
        sprite_path: row.get(4)?,
        preview_path: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn skill_from_row(row: &rusqlite::Row<'_>) -> Result<Skill> {
    let allowed_tools_json: String = row.get(4)?;
    let keywords_json: String = row.get(5)?;
    let allowed_tools = serde_json::from_str(&allowed_tools_json).unwrap_or_default();
    let keywords = serde_json::from_str(&keywords_json).unwrap_or_default();
    Ok(Skill {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        system_prompt: row.get(3)?,
        allowed_tools,
        keywords,
        is_active: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_agent_rows_gain_backend_fields_without_losing_existing_roles() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE agents (id TEXT PRIMARY KEY, name TEXT NOT NULL, avatar TEXT NOT NULL, description TEXT NOT NULL, system_prompt TEXT NOT NULL, model TEXT NOT NULL, allowed_tools_json TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            INSERT INTO agents VALUES ('legacy', '原有角色', 'x', '保留描述', '保留设定', 'old-model', '[]', 1, 1);").unwrap();
        let db = Database { conn };
        db.init_tables().unwrap();
        let mut agent = db.get_agent_by_id("legacy").unwrap().unwrap();
        assert_eq!(agent.backend, "direct_api");
        assert_eq!(agent.model, "old-model");
        assert_eq!(agent.system_prompt, "保留设定");
        agent.backend = "codex".into();
        agent.api_profile_id = "profile-a".into();
        db.save_agent(&agent).unwrap();
        let reloaded = db.list_agents().unwrap().pop().unwrap();
        assert_eq!(reloaded.backend, "codex");
        assert_eq!(reloaded.api_profile_id, "profile-a");
    }

    #[test]
    fn search_memories_escapes_literal_wildcards() {
        // `escape_like_pattern()` 会给 % _ \ 加反斜杠，SQL 必须配套 `ESCAPE '\'`；
        // 少了它，含这些字符的关键词会静默 0 命中（不报错，看起来只是"没有这条记忆"）。
        let db = Database::open_in_memory().expect("open in-memory db");
        db.save_memory("fact", "p1", "100% done").unwrap();
        db.save_memory("fact", "p2", "report_final").unwrap();

        let hits =
            |query: &str, category: Option<&str>| db.search_memories(query, category, 10).unwrap().len();
        assert_eq!(hits("100%", None), 1, "字面量 % 应能命中");
        assert_eq!(hits("report_final", None), 1, "字面量 _ 应能命中");
        assert_eq!(hits("100%", Some("fact")), 1, "带 category 路径同样要能命中");
        assert_eq!(hits("missing", None), 0);
    }

    #[test]
    fn deleted_builtin_skill_does_not_respawn_on_reseed() {
        let db = Database::open_in_memory().expect("open in-memory db");
        let seeded: Vec<String> = db
            .list_skills()
            .expect("list skills")
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(
            seeded.iter().any(|id| id == "builtin-translation-polish"),
            "builtin-translation-polish 应在首次 seed 时出现"
        );

        db.delete_skill("builtin-translation-polish")
            .expect("delete builtin");
        assert!(db
            .list_skills()
            .unwrap()
            .iter()
            .all(|s| s.id != "builtin-translation-polish"));

        db.ensure_builtin_skills().expect("re-seed");
        let after = db.list_skills().expect("list after reseed");
        assert!(
            after
                .iter()
                .all(|s| s.id != "builtin-translation-polish"),
            "墓碑应阻止内置技能在删除后被 ensure_builtin_skills 复活"
        );

        for id in seeded
            .iter()
            .filter(|id| *id != "builtin-translation-polish")
        {
            assert!(after.iter().any(|s| s.id == *id), "其他内置技能应仍在: {id}");
        }
    }

    #[test]
    fn legacy_chat_history_migrates_into_the_default_session() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE chat_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                thinking TEXT,
                created_at INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO chat_history (role, content) VALUES ('user', '迁移前的旧消息');",
        )
        .unwrap();
        let db = Database { conn };
        db.init_tables().unwrap();
        db.ensure_indexes().unwrap();

        let sessions = db.list_chat_sessions(None).unwrap();
        assert_eq!(sessions.len(), 1, "迁移后应只有一个默认会话");
        assert_eq!(sessions[0].id, DEFAULT_CHAT_SESSION_ID);
        assert_eq!(sessions[0].kind, "main");
        assert_eq!(sessions[0].slot, "main");
        assert_eq!(sessions[0].title, "默认对话");

        let messages = db.get_session_messages(DEFAULT_CHAT_SESSION_ID, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "迁移前的旧消息");
        assert_eq!(messages[0].session_id, DEFAULT_CHAT_SESSION_ID);
        assert_eq!(
            db.active_main_session_id().unwrap().as_deref(),
            Some(DEFAULT_CHAT_SESSION_ID),
            "活跃主会话必须指向迁移出的默认会话"
        );
    }

    #[test]
    fn chat_sessions_keep_messages_isolated_with_title_and_preview() {
        let db = Database::open_in_memory().unwrap();
        let main = db.ensure_default_session().unwrap();
        assert_eq!(main, DEFAULT_CHAT_SESSION_ID);

        let mut new_session = NewChatSession::for_slot("main");
        new_session.title = "第二段对话".into();
        let second = db.create_chat_session(new_session).unwrap();
        assert_eq!(second.kind, "main");
        assert_eq!(second.title, "第二段对话");

        db.save_message_with_quote_in(
            &second.id,
            "user",
            "第一条用户消息",
            None,
            Some("assistant"),
            Some("被引用的内容"),
        )
        .unwrap();
        db.touch_chat_session(&second.id, Some("首条消息标题"), None)
            .unwrap();
        db.save_message_with_agent_in(
            &second.id,
            "assistant",
            "助手回复内容",
            None,
            Some("builtin-claude"),
            Some("claude"),
            Some("⌘"),
        )
        .unwrap();
        db.save_message_in(&main, "user", "主会话消息").unwrap();

        let messages = db.get_session_messages(&second.id, None).unwrap();
        assert_eq!(messages.len(), 2, "不同会话的消息必须隔离");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].quoted_content.as_deref(), Some("被引用的内容"));
        assert!(messages[0].id < messages[1].id, "必须按 id ASC 返回");
        assert_eq!(messages[1].agent_id.as_deref(), Some("builtin-claude"));
        assert_eq!(messages[1].session_id, second.id);

        let main_messages = db.get_session_messages(&main, None).unwrap();
        assert_eq!(main_messages.len(), 1);
        assert_eq!(main_messages[0].content, "主会话消息");

        let listed = db.list_chat_sessions(None).unwrap();
        let listed_second = listed.iter().find(|item| item.id == second.id).unwrap();
        assert_eq!(listed_second.message_count, 2);
        assert_eq!(listed_second.preview, "助手回复内容");

        // 用户可读的标题不会被后续消息覆盖。
        db.touch_chat_session(&second.id, Some("不应覆盖"), None)
            .unwrap();
        assert_eq!(
            db.get_chat_session(&second.id).unwrap().unwrap().title,
            "第二段对话"
        );
    }

    #[test]
    fn first_message_replaces_only_placeholder_session_titles() {
        let db = Database::open_in_memory().unwrap();
        // create_chat_session 对空标题填「新对话」，首条消息应当覆盖它。
        let auto = db
            .create_chat_session(NewChatSession::for_slot("main"))
            .unwrap();
        assert_eq!(auto.title, "新对话");
        db.touch_chat_session(&auto.id, Some("今天要做什么"), None)
            .unwrap();
        assert_eq!(
            db.get_chat_session(&auto.id).unwrap().unwrap().title,
            "今天要做什么"
        );
        db.touch_chat_session(&auto.id, Some("第二条消息"), None)
            .unwrap();
        assert_eq!(
            db.get_chat_session(&auto.id).unwrap().unwrap().title,
            "今天要做什么",
            "首条消息生成后不再变化"
        );

        // 手动重命名后的标题必须保留。
        db.update_chat_session(&auto.id, Some("手动命名"), None, None, None)
            .unwrap();
        db.touch_chat_session(&auto.id, Some("不应覆盖"), None)
            .unwrap();
        assert_eq!(
            db.get_chat_session(&auto.id).unwrap().unwrap().title,
            "手动命名"
        );

        // 「新对话 N」同样是占位标题，首条消息会覆盖它。
        let numbered = db
            .create_chat_session(NewChatSession::for_slot("main"))
            .unwrap();
        db.update_chat_session(&numbered.id, Some("新对话 3"), None, None, None)
            .unwrap();
        db.touch_chat_session(&numbered.id, Some("第三条消息"), None)
            .unwrap();
        assert_eq!(
            db.get_chat_session(&numbered.id).unwrap().unwrap().title,
            "第三条消息"
        );
    }

    #[test]
    fn session_messages_return_the_latest_in_ascending_order() {
        let db = Database::open_in_memory().unwrap();
        let session = db.ensure_default_session().unwrap();
        for index in 0..10 {
            db.save_message_in(&session, "user", &format!("消息{index}"))
                .unwrap();
        }
        let all = db.get_session_messages(&session, None).unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(all.first().unwrap().content, "消息0");
        assert_eq!(all.last().unwrap().content, "消息9");

        let limited = db.get_session_messages(&session, Some(3)).unwrap();
        assert_eq!(limited.len(), 3, "limit 取最新 N 条");
        assert_eq!(limited.first().unwrap().content, "消息7");
        assert_eq!(limited.last().unwrap().content, "消息9");
    }

    #[test]
    fn deleting_a_session_removes_only_its_own_messages() {
        let db = Database::open_in_memory().unwrap();
        let main = db.ensure_default_session().unwrap();
        let mut new_session = NewChatSession::for_slot("claude");
        new_session.agent_id = "builtin-claude".into();
        let agent_session = db.create_chat_session(new_session).unwrap();
        db.save_message_in(&main, "user", "主会话消息").unwrap();
        db.save_message_in(&agent_session.id, "user", "agent 会话消息")
            .unwrap();

        db.delete_chat_session(&agent_session.id).unwrap();
        assert!(db.get_chat_session(&agent_session.id).unwrap().is_none());
        assert!(db
            .get_session_messages(&agent_session.id, None)
            .unwrap()
            .is_empty());
        assert_eq!(db.get_session_messages(&main, None).unwrap().len(), 1);
    }

    #[test]
    fn updating_a_session_writes_binding_model_and_cli_ids() {
        let db = Database::open_in_memory().unwrap();
        let mut new_session = NewChatSession::for_slot("codex");
        let session = db.create_chat_session(new_session.clone()).unwrap();
        assert_eq!(session.kind, "agent");

        let mut cli_sessions = HashMap::new();
        cli_sessions.insert("codex".to_string(), "0199a213-thread".to_string());
        let updated = db
            .update_chat_session(
                &session.id,
                Some("重命名后的标题"),
                Some(("builtin-codex", "codex", "⌘")),
                Some("gpt-5-codex"),
                Some(&cli_sessions),
            )
            .unwrap();
        assert_eq!(updated.title, "重命名后的标题");
        assert_eq!(updated.agent_id, "builtin-codex");
        assert_eq!(updated.agent_name, "codex");
        assert_eq!(updated.model, "gpt-5-codex");
        assert_eq!(updated.cli_session("codex"), Some("0199a213-thread"));

        // cli_sessions 只给后端用，不能通过 IPC 泄漏到前端。
        let wire = serde_json::to_value(&updated).unwrap();
        assert!(wire.get("cli_sessions").is_none());

        // 空标题 / None 字段不覆盖既有值。
        let unchanged = db
            .update_chat_session(&session.id, Some("   "), None, None, None)
            .unwrap();
        assert_eq!(unchanged.title, "重命名后的标题");
        assert_eq!(unchanged.model, "gpt-5-codex");

        new_session.title.clear();
        assert_eq!(NewChatSession::for_slot("main").kind, "main");
    }

    #[test]
    fn clearing_all_sessions_rebuilds_the_default_active_session() {
        let db = Database::open_in_memory().unwrap();
        let main = db.ensure_default_session().unwrap();
        let extra = db
            .create_chat_session(NewChatSession::for_slot("dsh"))
            .unwrap();
        db.save_message_in(&main, "user", "旧消息").unwrap();
        db.save_message_in(&extra.id, "user", "旧 agent 消息").unwrap();

        let rebuilt = db.clear_all_chat_sessions().unwrap();
        assert_eq!(rebuilt, DEFAULT_CHAT_SESSION_ID);
        let sessions = db.list_chat_sessions(None).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, DEFAULT_CHAT_SESSION_ID);
        assert!(db.get_session_messages(DEFAULT_CHAT_SESSION_ID, None).unwrap().is_empty());
        assert_eq!(
            db.active_main_session_id().unwrap().as_deref(),
            Some(DEFAULT_CHAT_SESSION_ID)
        );
    }

    #[test]
    fn falling_back_when_the_active_main_session_disappears() {
        let db = Database::open_in_memory().unwrap();
        let main = db.ensure_default_session().unwrap();
        db.save_setting(ACTIVE_MAIN_SESSION_KEY, &main).unwrap();
        db.delete_chat_session(&main).unwrap();
        let recovered = db.ensure_active_main_session().unwrap();
        assert_eq!(recovered, DEFAULT_CHAT_SESSION_ID, "默认会话应被重建");
        assert_eq!(
            db.active_main_session_id().unwrap().as_deref(),
            Some(DEFAULT_CHAT_SESSION_ID)
        );
    }
}
