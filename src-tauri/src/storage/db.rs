use rusqlite::{Connection, Result};
use serde::Serialize;
use std::path::PathBuf;

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

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let db = Self { conn };
        db.init_tables()?;
        Ok(db)
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
            );",
        )?;
        self.ensure_chat_thinking_column()?;
        self.ensure_clipboard_pinned_column()?;
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

    // ===== 宠物记忆 =====

    pub fn save_memory(&self, category: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pet_memory (category, key, value) VALUES (?1, ?2, ?3)",
            (category, key, value),
        )?;
        Ok(())
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

    pub fn get_memories_by_category(&self, category: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT key, value FROM pet_memory WHERE category = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([category], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
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
}
