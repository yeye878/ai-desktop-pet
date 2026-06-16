use rusqlite::{Connection, OptionalExtension, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::Skill;

#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub created_at: i64,
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
             CREATE INDEX IF NOT EXISTS idx_skills_is_active ON skills(is_active) WHERE is_active = 1;",
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
            END;",
        )?;
        self.ensure_chat_thinking_column()?;
        self.ensure_clipboard_pinned_column()?;
        self.ensure_scheduled_tasks_columns()?;
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

    pub fn get_recent_messages(&self, limit: u32) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at
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

    /// 只返回 id > after_id 的最近 limit 条消息（用于 AI 上下文隔离）
    pub fn get_recent_messages_after(&self, after_id: i64, limit: u32) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, thinking, created_at
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
            })
        })?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        messages.reverse();
        Ok(messages)
    }

    /// 返回 chat_history 表中最大的 id，无消息时返回 0
    pub fn get_max_message_id(&self) -> Result<i64> {
        self.conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM chat_history", [], |row| {
                row.get(0)
            })
            .map_err(|e| e.into())
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
        let sql = if category.is_some() {
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 OR value LIKE ?1)
               AND category = ?2
             ORDER BY id DESC
             LIMIT ?3"
        } else {
            "SELECT id, category, key, value, created_at
             FROM pet_memory
             WHERE (key LIKE ?1 OR value LIKE ?1)
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
