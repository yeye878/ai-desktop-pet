// 临时端到端验证 binary —— 5 步纯 DB 层探查
use rusqlite::{params, Connection};

fn conn() -> Connection {
    let c = Connection::open_in_memory().expect("open mem db");
    c.execute_batch(
        "CREATE TABLE pet_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE TABLE skills (
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
        CREATE TRIGGER skills_single_active_update
        BEFORE UPDATE OF is_active ON skills
        WHEN NEW.is_active = 1
        BEGIN
            UPDATE skills SET is_active = 0 WHERE id != NEW.id;
        END;
        CREATE TRIGGER skills_single_active_insert
        BEFORE INSERT ON skills
        WHEN NEW.is_active = 1
        BEGIN
            UPDATE skills SET is_active = 0;
        END;
        CREATE INDEX idx_skills_is_active ON skills(is_active) WHERE is_active = 1;",
    )
    .expect("init");
    c
}

fn section(t: &str) {
    println!("\n=== {t} ===");
}
fn ok(b: bool, label: &str) {
    println!("  {} {label}", if b { "PASS" } else { "FAIL" });
    assert!(b, "{label}");
}

fn main() {
    // === 步 1: schema / 触发器 / 部分索引 ===
    section("Step 1: schema / triggers / partial index");
    let c = conn();
    let trigger_update: String = c
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name='skills_single_active_update'",
            [],
            |r| r.get(0),
        )
        .expect("trigger update missing");
    ok(
        trigger_update == "skills_single_active_update",
        "trigger update exists",
    );
    let trigger_insert: String = c
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name='skills_single_active_insert'",
            [],
            |r| r.get(0),
        )
        .expect("trigger insert missing");
    ok(
        trigger_insert == "skills_single_active_insert",
        "trigger insert exists",
    );
    let idx_count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_skills_is_active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    ok(idx_count == 1, "partial index exists");

    // === 步 2: 自定义提示往返 ===
    section("Step 2: custom_system_prompt roundtrip");
    c.execute(
        "INSERT INTO pet_settings(key, value) VALUES ('custom_system_prompt', ?1)",
        params!["汪！"],
    )
    .unwrap();
    let v: String = c
        .query_row(
            "SELECT value FROM pet_settings WHERE key='custom_system_prompt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    ok(v == "汪！", &format!("stored={v:?}"));

    // === 步 4: 技能 CRUD ===
    section("Step 4: skills CRUD");
    let insert = |conn: &Connection, id: &str, name: &str, tools: &str, active: i64| {
        conn.execute(
            "INSERT INTO skills (id, name, allowed_tools_json, is_active) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, tools, active],
        )
        .unwrap();
    };
    insert(&c, "a", "翻译官", r#"["web_search"]"#, 0);
    insert(&c, "b", "程序员", r#"["run_command"]"#, 0);
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .unwrap();
    ok(n == 2, "two skills inserted");
    c.execute(
        "UPDATE skills SET name=?1 WHERE id='a'",
        params!["翻译官v2"],
    )
    .unwrap();
    let new_name: String = c
        .query_row("SELECT name FROM skills WHERE id='a'", [], |r| r.get(0))
        .unwrap();
    ok(new_name == "翻译官v2", "rename works");
    c.execute("DELETE FROM skills WHERE id='a'", []).unwrap();
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .unwrap();
    ok(n == 1, "delete works");

    // === 步 5: 手动激活 + 单激活不变量 ===
    section("Step 5: single-active invariant");
    insert(&c, "c", "C技能", "[]", 0);
    insert(&c, "d", "D技能", "[]", 0);
    c.execute(
        "UPDATE skills SET is_active = CASE WHEN id = 'b' THEN 1 ELSE 0 END",
        [],
    )
    .unwrap();
    let active: Vec<String> = c
        .prepare("SELECT id FROM skills WHERE is_active = 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    ok(active == vec!["b".to_string()], "activate b");
    c.execute(
        "UPDATE skills SET is_active = CASE WHEN id = 'd' THEN 1 ELSE 0 END",
        [],
    )
    .unwrap();
    let active: Vec<String> = c
        .prepare("SELECT id FROM skills WHERE is_active = 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    ok(active == vec!["d".to_string()], "switch to d clears b");
    c.execute("UPDATE skills SET is_active = 0", []).unwrap();
    let n_active: i64 = c
        .query_row("SELECT COUNT(*) FROM skills WHERE is_active = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    ok(n_active == 0, "clear all");
    insert(&c, "e", "E技能", "[]", 1);
    let active: Vec<String> = c
        .prepare("SELECT id FROM skills WHERE is_active = 1")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    ok(
        active == vec!["e".to_string()],
        "insert with is_active=1 also clears others",
    );

    // === 步 10: DB 并发（同连接交错更新，模拟 set_active_skill 并发） ===
    section("Step 10: concurrent set_active_skill");
    insert(&c, "f", "F技能", "[]", 0);
    insert(&c, "g", "G技能", "[]", 0);
    c.execute(
        "UPDATE skills SET is_active = CASE WHEN id = 'f' THEN 1 ELSE 0 END",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE skills SET is_active = CASE WHEN id = 'g' THEN 1 ELSE 0 END",
        [],
    )
    .unwrap();
    let n_active: i64 = c
        .query_row("SELECT COUNT(*) FROM skills WHERE is_active = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    ok(
        n_active == 1,
        "exactly one active after back-to-back updates",
    );
    let winner: String = c
        .query_row("SELECT id FROM skills WHERE is_active = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    ok(winner == "g", "last writer wins");

    println!("\nAll 5 DB-layer steps PASS.");
}
