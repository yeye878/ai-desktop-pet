//! SQLite 持久化（第 11 节）：独立短连接、短事务、幂等迁移。
//! 模型运行期间不持有数据库锁；状态与对应事件同一事务写入，revision 比较更新。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::contracts::{NodeState, OrchestrationEvent, RunState};

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(conn)
}

/// 幂等建表。唯一约束：(node_id,attempt)、(run_id,seq)、idempotency_key、
/// 每 session 最多一个活跃 adaptive run（部分唯一索引）。
pub fn ensure_schema(path: &Path) -> rusqlite::Result<()> {
    let conn = open(path)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS orchestration_tasks (
            task_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            original_request TEXT NOT NULL,
            current_revision INTEGER NOT NULL DEFAULT 1,
            idempotency_key TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS orchestration_task_revisions (
            task_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            change_text TEXT NOT NULL,
            acceptance_json TEXT NOT NULL DEFAULT '[]',
            input_hash TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            PRIMARY KEY (task_id, revision)
        );
        CREATE TABLE IF NOT EXISTS orchestration_runs (
            run_id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            state TEXT NOT NULL,
            state_revision INTEGER NOT NULL DEFAULT 1,
            task_revision INTEGER NOT NULL DEFAULT 1,
            policy_json TEXT,
            usage_stats_json TEXT NOT NULL DEFAULT '{}',
            final_artifact_set_id TEXT,
            status_message TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS orchestration_one_active_run
            ON orchestration_runs(session_id)
            WHERE state NOT IN ('succeeded','failed','review_failed','cancelled');
        CREATE TABLE IF NOT EXISTS orchestration_nodes (
            node_id TEXT NOT NULL,
            run_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            plan_version INTEGER NOT NULL DEFAULT 1,
            spec_json TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending',
            assigned_agent_id TEXT NOT NULL DEFAULT '',
            attempts INTEGER NOT NULL DEFAULT 0,
            rework_round INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (run_id, node_id)
        );
        CREATE TABLE IF NOT EXISTS orchestration_invocations (
            invocation_id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            attempt INTEGER NOT NULL,
            role_kind TEXT NOT NULL,
            config_json TEXT NOT NULL DEFAULT '{}',
            backend TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            usage_json TEXT NOT NULL DEFAULT '{}',
            error TEXT,
            UNIQUE (run_id, node_id, attempt)
        );
        CREATE TABLE IF NOT EXISTS orchestration_artifacts (
            artifact_set_id TEXT NOT NULL,
            revision INTEGER NOT NULL,
            run_id TEXT NOT NULL,
            task_revision INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            manifest_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (artifact_set_id, revision)
        );
        CREATE TABLE IF NOT EXISTS orchestration_evidence (
            evidence_id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            source_invocation_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            cwd TEXT NOT NULL DEFAULT '',
            exit_code INTEGER,
            artifact_refs_json TEXT NOT NULL DEFAULT '[]',
            log_ref TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS orchestration_reviews (
            review_id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            reviewer_invocation_id TEXT NOT NULL,
            task_revision INTEGER NOT NULL,
            artifact_set_id TEXT NOT NULL,
            artifact_revision INTEGER NOT NULL,
            artifact_hash TEXT NOT NULL,
            verdict TEXT NOT NULL,
            report_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS orchestration_events (
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            event_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            task_id TEXT NOT NULL,
            node_id TEXT NOT NULL DEFAULT '',
            invocation_id TEXT NOT NULL DEFAULT '',
            attempt INTEGER NOT NULL DEFAULT 0,
            state_revision INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (run_id, seq)
        );
        CREATE TABLE IF NOT EXISTS orchestration_operations (
            operation_id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            idempotency_key TEXT NOT NULL DEFAULT '',
            action_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'initiated',
            result_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    )?;
    // 中间版本曾用 budget_used_json 列名；幂等改名，不丢统计。
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(orchestration_runs)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if columns.iter().any(|c| c == "budget_used_json") && !columns.iter().any(|c| c == "usage_stats_json") {
        conn.execute_batch("ALTER TABLE orchestration_runs RENAME COLUMN budget_used_json TO usage_stats_json;")?;
    }
    // 早期表把 UNIQUE 建在 (node_id, attempt) 而非 (run_id, node_id, attempt)，导致跨 run 冲突；幂等重建。
    let inv_sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'orchestration_invocations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if inv_sql.as_deref().is_some_and(|sql| {
        !sql.to_ascii_lowercase()
            .replace([' ', '\n', '\r', '\t'], "")
            .contains("unique(run_id,node_id,attempt)")
    }) {
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             ALTER TABLE orchestration_invocations RENAME TO orchestration_invocations_old;
             CREATE TABLE orchestration_invocations (
                 invocation_id TEXT PRIMARY KEY,
                 run_id TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 attempt INTEGER NOT NULL,
                 role_kind TEXT NOT NULL,
                 config_json TEXT NOT NULL DEFAULT '{}',
                 backend TEXT NOT NULL DEFAULT '',
                 started_at INTEGER NOT NULL,
                 finished_at INTEGER,
                 usage_json TEXT NOT NULL DEFAULT '{}',
                 error TEXT,
                 UNIQUE (run_id, node_id, attempt)
             );
             INSERT OR IGNORE INTO orchestration_invocations
                 (invocation_id, run_id, node_id, attempt, role_kind, config_json, backend, started_at, finished_at, usage_json, error)
             SELECT invocation_id, run_id, node_id, attempt, role_kind, config_json, backend, started_at, finished_at, usage_json, error
             FROM orchestration_invocations_old;
             DROP TABLE orchestration_invocations_old;
             COMMIT;",
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub task_id: String,
    pub session_id: String,
    pub original_request: String,
    pub task_revision: u32,
    pub run_id: String,
    pub state: RunState,
    pub state_revision: u64,
    pub policy: Option<serde_json::Value>,
    pub usage_stats: serde_json::Value,
    pub final_artifact_set_id: Option<String>,
    pub status_message: String,
    pub nodes: Vec<serde_json::Value>,
    pub artifacts: Vec<serde_json::Value>,
    pub reviews: Vec<serde_json::Value>,
    pub latest_seq: u64,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn conn(&self) -> rusqlite::Result<Connection> {
        open(&self.path)
    }

    /// 创建 task+run；idempotency_key 冲突时返回已有 run，不产生重复 run。
    /// 每 session 一个活跃 run 由部分唯一索引兜底，这里先给出友好错误。
    pub fn create_run(
        &self,
        session_id: &str,
        request: &str,
        idempotency_key: &str,
    ) -> rusqlite::Result<(String, String, bool)> {
        let mut conn = self.conn()?;
        if let Some(existing) = conn
            .query_row(
                "SELECT r.run_id, r.task_id FROM orchestration_runs r
                 JOIN orchestration_tasks t ON t.task_id = r.task_id
                 WHERE t.idempotency_key = ?1",
                params![idempotency_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            return Ok((existing.1, existing.0, false));
        }
        let active: Option<String> = conn
            .query_row(
                "SELECT run_id FROM orchestration_runs
                 WHERE session_id = ?1 AND state NOT IN ('succeeded','failed','review_failed','cancelled')",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if active.is_some() {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                Some("该会话已有进行中的智能调度任务，请先完成或停止".into()),
            ));
        }
        let tx = conn.transaction()?;
        let task_id = new_id("task");
        let run_id = new_id("run");
        let now = unix_now();
        tx.execute(
            "INSERT INTO orchestration_tasks (task_id, session_id, original_request, current_revision, idempotency_key, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)",
            params![task_id, session_id, request, idempotency_key, now],
        )?;
        tx.execute(
            "INSERT INTO orchestration_task_revisions (task_id, revision, change_text, created_at) VALUES (?1, 1, ?2, ?3)",
            params![task_id, request, now],
        )?;
        tx.execute(
            "INSERT INTO orchestration_runs (run_id, task_id, session_id, state, state_revision, task_revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'queued', 1, 1, ?4, ?4)",
            params![run_id, task_id, session_id, now],
        )?;
        append_event_tx(
            &tx,
            &run_id,
            session_id,
            &task_id,
            "",
            "",
            0,
            1,
            "run.created",
            &serde_json::json!({ "request": request }),
        )?;
        tx.commit()?;
        Ok((task_id, run_id, true))
    }

    /// revision 比较的状态更新；状态与事件同一事务。返回 false 表示 revision 过期。
    pub fn transition(
        &self,
        run_id: &str,
        expected_revision: u64,
        new_state: RunState,
        status_message: &str,
        event_kind: &str,
        event_payload: &serde_json::Value,
        node_id: &str,
        invocation_id: &str,
    ) -> rusqlite::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current: Option<(String, u64, String, String)> = tx
            .query_row(
                "SELECT state, state_revision, session_id, task_id FROM orchestration_runs WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((state, revision, session_id, task_id)) = current else {
            return Ok(false);
        };
        if revision != expected_revision {
            return Ok(false);
        }
        if RunState::parse(&state).is_some_and(|s| s.is_terminal()) {
            return Ok(false);
        }
        let next_revision = revision + 1;
        tx.execute(
            "UPDATE orchestration_runs SET state = ?1, state_revision = ?2, status_message = ?3, updated_at = ?4 WHERE run_id = ?5",
            params![new_state.as_str(), next_revision, status_message, unix_now(), run_id],
        )?;
        append_event_tx(
            &tx,
            run_id,
            &session_id,
            &task_id,
            node_id,
            invocation_id,
            0,
            next_revision,
            event_kind,
            event_payload,
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn set_policy(&self, run_id: &str, policy: &serde_json::Value, task_revision: u32) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_runs SET policy_json = ?1, task_revision = ?2, updated_at = ?3 WHERE run_id = ?4",
            params![policy.to_string(), task_revision, unix_now(), run_id],
        )?;
        Ok(())
    }

    pub fn set_final_artifact(&self, run_id: &str, artifact_set_id: &str) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_runs SET final_artifact_set_id = ?1, updated_at = ?2 WHERE run_id = ?3",
            params![artifact_set_id, unix_now(), run_id],
        )?;
        Ok(())
    }

    pub fn add_usage_stats(&self, run_id: &str, invocations: u32, tool_calls: u64) -> rusqlite::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let used: String = tx.query_row(
            "SELECT usage_stats_json FROM orchestration_runs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        let mut value: serde_json::Value = serde_json::from_str(&used).unwrap_or(serde_json::json!({}));
        let object = value.as_object_mut().unwrap();
        let inv = object.get("agent_invocations").and_then(|v| v.as_u64()).unwrap_or(0) + invocations as u64;
        let tools = object.get("tool_calls").and_then(|v| v.as_u64()).unwrap_or(0) + tool_calls;
        object.insert("agent_invocations".into(), serde_json::json!(inv));
        object.insert("tool_calls".into(), serde_json::json!(tools));
        tx.execute(
            "UPDATE orchestration_runs SET usage_stats_json = ?1, updated_at = ?2 WHERE run_id = ?3",
            params![value.to_string(), unix_now(), run_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_node(&self, run_id: &str, spec: &serde_json::Value, state: NodeState) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        let node_id = spec["node_id"].as_str().unwrap_or_default();
        let kind = spec["kind"].as_str().unwrap_or_default();
        let assigned_agent_id = spec["assigned_agent_id"].as_str().unwrap_or_default();
        conn.execute(
            "INSERT INTO orchestration_nodes (node_id, run_id, kind, spec_json, state, assigned_agent_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(run_id, node_id) DO UPDATE SET spec_json = excluded.spec_json, state = excluded.state, assigned_agent_id = excluded.assigned_agent_id, updated_at = excluded.updated_at",
            params![node_id, run_id, kind, spec.to_string(), state.as_str(), assigned_agent_id, unix_now()],
        )?;
        Ok(())
    }

    pub fn set_node_state(&self, run_id: &str, node_id: &str, state: NodeState) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_nodes SET state = ?1, updated_at = ?2 WHERE run_id = ?3 AND node_id = ?4",
            params![state.as_str(), unix_now(), run_id, node_id],
        )?;
        Ok(())
    }

    pub fn next_attempt(&self, run_id: &str, node_id: &str) -> rusqlite::Result<u32> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_nodes SET attempts = attempts + 1, updated_at = ?1 WHERE run_id = ?2 AND node_id = ?3",
            params![unix_now(), run_id, node_id],
        )?;
        let attempts: u32 = conn.query_row(
            "SELECT attempts FROM orchestration_nodes WHERE run_id = ?1 AND node_id = ?2",
            params![run_id, node_id],
            |row| row.get(0),
        )?;
        Ok(attempts)
    }

    pub fn set_rework_round(&self, run_id: &str, node_id: &str, round: u32) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_nodes SET rework_round = ?1, updated_at = ?2 WHERE run_id = ?3 AND node_id = ?4",
            params![round, unix_now(), run_id, node_id],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_invocation(
        &self,
        invocation_id: &str,
        run_id: &str,
        node_id: &str,
        attempt: u32,
        role_kind: &str,
        backend: &str,
        config: &serde_json::Value,
    ) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO orchestration_invocations (invocation_id, run_id, node_id, attempt, role_kind, config_json, backend, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![invocation_id, run_id, node_id, attempt, role_kind, config.to_string(), backend, unix_now()],
        )?;
        Ok(())
    }

    pub fn finish_invocation(
        &self,
        invocation_id: &str,
        usage: &serde_json::Value,
        error: Option<&str>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE orchestration_invocations SET finished_at = ?1, usage_json = ?2, error = ?3 WHERE invocation_id = ?4",
            params![unix_now(), usage.to_string(), error, invocation_id],
        )?;
        Ok(())
    }

    pub fn insert_artifact(&self, run_id: &str, manifest: &serde_json::Value) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO orchestration_artifacts (artifact_set_id, revision, run_id, task_revision, content_hash, manifest_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                manifest["artifact_set_id"].as_str().unwrap_or_default(),
                manifest["revision"].as_u64().unwrap_or(1),
                run_id,
                manifest["task_revision"].as_u64().unwrap_or(1),
                manifest["content_hash"].as_str().unwrap_or_default(),
                manifest.to_string(),
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn insert_evidence(&self, run_id: &str, record: &super::contracts::EvidenceRecord) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO orchestration_evidence (evidence_id, run_id, source_invocation_id, kind, summary, cwd, exit_code, artifact_refs_json, log_ref, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.evidence_id,
                run_id,
                record.source_invocation_id,
                record.kind,
                record.summary,
                record.cwd,
                record.exit_code,
                serde_json::to_string(&record.artifact_refs).unwrap_or_default(),
                record.log_ref,
                record.created_at
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_review(
        &self,
        review_id: &str,
        run_id: &str,
        reviewer_invocation_id: &str,
        report: &super::contracts::ReviewReport,
        artifact_revision: u32,
    ) -> rusqlite::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO orchestration_reviews (review_id, run_id, reviewer_invocation_id, task_revision, artifact_set_id, artifact_revision, artifact_hash, verdict, report_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                review_id,
                run_id,
                reviewer_invocation_id,
                report.task_revision,
                report.artifact_set_id,
                artifact_revision,
                report.artifact_hash,
                serde_json::to_string(&report.verdict).unwrap_or_default().trim_matches('"'),
                serde_json::to_string(report).unwrap_or_default(),
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn append_event(
        &self,
        run_id: &str,
        kind: &str,
        payload: &serde_json::Value,
        node_id: &str,
        invocation_id: &str,
    ) -> rusqlite::Result<u64> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (session_id, task_id, revision): (String, String, u64) = tx.query_row(
            "SELECT session_id, task_id, state_revision FROM orchestration_runs WHERE run_id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let seq = append_event_tx(&tx, run_id, &session_id, &task_id, node_id, invocation_id, 0, revision, kind, payload)?;
        tx.commit()?;
        Ok(seq)
    }

    pub fn events_after(&self, run_id: &str, after_seq: u64) -> rusqlite::Result<Vec<OrchestrationEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT event_id, session_id, task_id, run_id, node_id, invocation_id, attempt, seq, state_revision, kind, payload_json, created_at
             FROM orchestration_events WHERE run_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![run_id, after_seq], |row| {
            Ok(OrchestrationEvent {
                event_id: row.get(0)?,
                session_id: row.get(1)?,
                task_id: row.get(2)?,
                run_id: row.get(3)?,
                node_id: row.get(4)?,
                invocation_id: row.get(5)?,
                attempt: row.get(6)?,
                seq: row.get(7)?,
                state_revision: row.get(8)?,
                kind: row.get(9)?,
                payload: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(10)?)
                    .unwrap_or(serde_json::Value::Null),
                created_at: row.get(11)?,
            })
        })?;
        rows.collect()
    }

    pub fn snapshot(&self, run_id: &str) -> rusqlite::Result<Option<RunSnapshot>> {
        let conn = self.conn()?;
        let base = conn
            .query_row(
                "SELECT r.task_id, r.session_id, r.state, r.state_revision, r.task_revision, r.policy_json, r.usage_stats_json, r.final_artifact_set_id, r.status_message, t.original_request
                 FROM orchestration_runs r JOIN orchestration_tasks t ON t.task_id = r.task_id
                 WHERE r.run_id = ?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((task_id, session_id, state, state_revision, task_revision, policy_json, usage_stats, final_set, status_message, request)) = base else {
            return Ok(None);
        };
        let json_rows = |sql: &str| -> rusqlite::Result<Vec<serde_json::Value>> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![run_id], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(serde_json::from_str(&row?).unwrap_or(serde_json::Value::Null));
            }
            Ok(out)
        };
        let mut node_stmt = conn.prepare(
            "SELECT spec_json, state, attempts, rework_round FROM orchestration_nodes WHERE run_id = ?1 ORDER BY rowid",
        )?;
        let node_rows = node_stmt.query_map(params![run_id], |row| {
            let mut value: serde_json::Value =
                serde_json::from_str(&row.get::<_, String>(0)?).unwrap_or(serde_json::json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert("state".into(), serde_json::json!(row.get::<_, String>(1)?));
                object.insert("attempts".into(), serde_json::json!(row.get::<_, u32>(2)?));
                object.insert("rework_round".into(), serde_json::json!(row.get::<_, u32>(3)?));
            }
            Ok(value)
        })?;
        let nodes: Vec<serde_json::Value> = node_rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let artifacts = json_rows("SELECT manifest_json FROM orchestration_artifacts WHERE run_id = ?1 ORDER BY revision")?;
        let reviews = json_rows("SELECT report_json FROM orchestration_reviews WHERE run_id = ?1 ORDER BY created_at")?;
        let latest_seq: u64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM orchestration_events WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )?;
        Ok(Some(RunSnapshot {
            task_id,
            session_id,
            original_request: request,
            task_revision,
            run_id: run_id.to_string(),
            state: RunState::parse(&state).unwrap_or(RunState::Failed),
            state_revision,
            policy: policy_json.and_then(|p| serde_json::from_str(&p).ok()),
            usage_stats: serde_json::from_str(&usage_stats).unwrap_or(serde_json::json!({})),
            final_artifact_set_id: final_set,
            status_message,
            nodes,
            artifacts,
            reviews,
            latest_seq,
        }))
    }

    pub fn list_runs(&self, session_id: &str) -> rusqlite::Result<Vec<serde_json::Value>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.run_id, r.task_id, r.state, r.state_revision, r.status_message, r.created_at, r.updated_at, t.original_request
             FROM orchestration_runs r JOIN orchestration_tasks t ON t.task_id = r.task_id
             WHERE r.session_id = ?1 ORDER BY r.created_at DESC, r.rowid DESC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(serde_json::json!({
                "run_id": row.get::<_, String>(0)?,
                "task_id": row.get::<_, String>(1)?,
                "state": row.get::<_, String>(2)?,
                "state_revision": row.get::<_, u64>(3)?,
                "status_message": row.get::<_, String>(4)?,
                "created_at": row.get::<_, i64>(5)?,
                "updated_at": row.get::<_, i64>(6)?,
                "request": row.get::<_, String>(7)?,
            }))
        })?;
        rows.collect()
    }

    pub fn active_run(&self, session_id: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT run_id FROM orchestration_runs
             WHERE session_id = ?1 AND state NOT IN ('succeeded','failed','review_failed','cancelled')",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn run_state(&self, run_id: &str) -> rusqlite::Result<Option<(RunState, u64)>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT state, state_revision FROM orchestration_runs WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(state, rev)| RunState::parse(&state).map(|s| (s, rev))))
    }

    /// 需求版本事务递增（第 3 节）；旧审核不能批准新需求由版本比对保证。
    pub fn amend_task(&self, task_id: &str, expected_revision: u32, change: &str) -> rusqlite::Result<Option<u32>> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current: Option<u32> = tx
            .query_row(
                "SELECT current_revision FROM orchestration_tasks WHERE task_id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(current) = current else { return Ok(None) };
        if current != expected_revision {
            return Ok(None);
        }
        let next = current + 1;
        tx.execute(
            "UPDATE orchestration_tasks SET current_revision = ?1, updated_at = ?2 WHERE task_id = ?3",
            params![next, unix_now(), task_id],
        )?;
        tx.execute(
            "INSERT INTO orchestration_task_revisions (task_id, revision, change_text, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![task_id, next, change, unix_now()],
        )?;
        tx.commit()?;
        Ok(Some(next))
    }

    /// 重启核对：cancelling 直接收尾为 cancelled；其余不确定的进行中 run 标 interrupted。
    pub fn mark_interrupted_on_startup(&self) -> rusqlite::Result<u64> {
        let conn = self.conn()?;
        let now = unix_now();
        let cancelled = conn.execute(
            "UPDATE orchestration_runs SET state = 'cancelled', state_revision = state_revision + 1, status_message = '应用重启，取消已完成', updated_at = ?1
             WHERE state = 'cancelling'",
            params![now],
        )?;
        let changed = conn.execute(
            "UPDATE orchestration_runs SET state = 'interrupted', state_revision = state_revision + 1, status_message = '应用重启，状态待核对', updated_at = ?1
             WHERE state NOT IN ('succeeded','failed','review_failed','cancelled','interrupted','waiting_input','blocked')",
            params![now],
        )?;
        Ok((cancelled + changed) as u64)
    }

    pub fn task_session_id(&self, task_id: &str) -> rusqlite::Result<Option<String>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT session_id FROM orchestration_tasks WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()
    }

    pub fn latest_event(&self, run_id: &str) -> rusqlite::Result<Option<OrchestrationEvent>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT event_id, session_id, task_id, run_id, node_id, invocation_id, attempt, seq, state_revision, kind, payload_json, created_at
             FROM orchestration_events WHERE run_id = ?1 ORDER BY seq DESC LIMIT 1",
            params![run_id],
            |row| {
                Ok(OrchestrationEvent {
                    event_id: row.get(0)?,
                    session_id: row.get(1)?,
                    task_id: row.get(2)?,
                    run_id: row.get(3)?,
                    node_id: row.get(4)?,
                    invocation_id: row.get(5)?,
                    attempt: row.get(6)?,
                    seq: row.get(7)?,
                    state_revision: row.get(8)?,
                    kind: row.get(9)?,
                    payload: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(10)?)
                        .unwrap_or(serde_json::Value::Null),
                    created_at: row.get(11)?,
                })
            },
        )
        .optional()
    }
}

#[allow(clippy::too_many_arguments)]
fn append_event_tx(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    session_id: &str,
    task_id: &str,
    node_id: &str,
    invocation_id: &str,
    attempt: u32,
    state_revision: u64,
    kind: &str,
    payload: &serde_json::Value,
) -> rusqlite::Result<u64> {
    let seq: u64 = tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM orchestration_events WHERE run_id = ?1",
        params![run_id],
        |row| row.get(0),
    )?;
    let event_id = new_id("evt");
    tx.execute(
        "INSERT INTO orchestration_events (run_id, seq, event_id, session_id, task_id, node_id, invocation_id, attempt, state_revision, kind, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![run_id, seq, event_id, session_id, task_id, node_id, invocation_id, attempt, state_revision, kind, payload.to_string(), unix_now()],
    )?;
    Ok(seq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, tempfile_dir::TempDir) {
        let dir = tempfile_dir::TempDir::new();
        let store = Store::new(dir.path.join("pet.db"));
        ensure_schema(&store.path).unwrap();
        (store, dir)
    }

    // 轻量临时目录，避免引入 tempfile 依赖。
    mod tempfile_dir {
        use std::path::PathBuf;
        pub struct TempDir {
            pub path: PathBuf,
        }
        impl TempDir {
            pub fn new() -> Self {
                let path = std::env::temp_dir().join(format!("orch-test-{}", uuid::Uuid::new_v4().simple()));
                std::fs::create_dir_all(&path).unwrap();
                Self { path }
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }
    }

    #[test]
    fn idempotency_key_returns_existing_run() {
        let (store, _dir) = store();
        let (task1, run1, created1) = store.create_run("s1", "写个总结", "key-1").unwrap();
        let (task2, run2, created2) = store.create_run("s1", "写个总结", "key-1").unwrap();
        assert!(created1);
        assert!(!created2);
        assert_eq!((task1, run1), (task2, run2));
    }

    #[test]
    fn one_active_run_per_session() {
        let (store, _dir) = store();
        store.create_run("s1", "任务一", "k1").unwrap();
        let err = store.create_run("s1", "任务二", "k2").unwrap_err();
        assert!(err.to_string().contains("进行中"));
        // 其它会话不受影响。
        assert!(store.create_run("s2", "任务三", "k3").is_ok());
    }

    #[test]
    fn transition_uses_revision_compare_and_writes_event_atomically() {
        let (store, _dir) = store();
        let (_, run, _) = store.create_run("s1", "t", "k").unwrap();
        assert!(store.transition(&run, 1, RunState::Assessing, "", "run.assessing", &serde_json::json!({}), "", "").unwrap());
        // 过期 revision 被拒收。
        assert!(!store.transition(&run, 1, RunState::Executing, "", "run.executing", &serde_json::json!({}), "", "").unwrap());
        assert!(store.transition(&run, 2, RunState::Executing, "", "run.executing", &serde_json::json!({}), "", "").unwrap());
        let events = store.events_after(&run, 0).unwrap();
        assert_eq!(events.len(), 3); // created + assessing + executing
        assert_eq!(events[2].seq, 3);
        assert_eq!(events[2].state_revision, 3);
        // 终态后不再接受转移。
        assert!(store.transition(&run, 3, RunState::Succeeded, "", "run.succeeded", &serde_json::json!({}), "", "").unwrap());
        assert!(!store.transition(&run, 4, RunState::Executing, "", "x", &serde_json::json!({}), "", "").unwrap());
    }

    #[test]
    fn terminal_run_frees_session_slot() {
        let (store, _dir) = store();
        let (_, run, _) = store.create_run("s1", "t", "k").unwrap();
        store.transition(&run, 1, RunState::Cancelled, "", "run.cancelled", &serde_json::json!({}), "", "").unwrap();
        assert!(store.create_run("s1", "t2", "k2").is_ok());
        assert_eq!(store.active_run("s1").unwrap().is_some(), true);
    }

    #[test]
    fn amend_increments_revision_with_compare() {
        let (store, _dir) = store();
        let (task, _, _) = store.create_run("s1", "t", "k").unwrap();
        assert_eq!(store.amend_task(&task, 1, "补充要求").unwrap(), Some(2));
        assert_eq!(store.amend_task(&task, 1, "过期补充").unwrap(), None);
        assert_eq!(store.amend_task(&task, 2, "再补充").unwrap(), Some(3));
    }

    #[test]
    fn startup_marks_running_runs_interrupted() {
        let (store, _dir) = store();
        let (_, run, _) = store.create_run("s1", "t", "k").unwrap();
        store.transition(&run, 1, RunState::Executing, "", "run.executing", &serde_json::json!({}), "", "").unwrap();
        assert_eq!(store.mark_interrupted_on_startup().unwrap(), 1);
        let (state, _) = store.run_state(&run).unwrap().unwrap();
        assert_eq!(state, RunState::Interrupted);
        // 幂等：再跑一次不再改动。
        assert_eq!(store.mark_interrupted_on_startup().unwrap(), 0);
    }

    #[test]
    fn invocations_unique_per_run_and_legacy_schema_migrates() {
        let dir = tempfile_dir::TempDir::new();
        let db_path = dir.path.join("legacy.db");
        {
            let conn = open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE orchestration_invocations (
                    invocation_id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    node_id TEXT NOT NULL,
                    attempt INTEGER NOT NULL,
                    role_kind TEXT NOT NULL,
                    config_json TEXT NOT NULL DEFAULT '{}',
                    backend TEXT NOT NULL DEFAULT '',
                    started_at INTEGER NOT NULL,
                    finished_at INTEGER,
                    usage_json TEXT NOT NULL DEFAULT '{}',
                    error TEXT,
                    UNIQUE (node_id, attempt)
                );
                INSERT INTO orchestration_invocations (invocation_id, run_id, node_id, attempt, role_kind, started_at)
                VALUES ('inv-1', 'run-1', 'assess', 1, 'assess', 100);",
            )
            .unwrap();
        }
        ensure_schema(&db_path).unwrap();
        let store = Store::new(db_path);
        assert!(store
            .record_invocation("inv-2", "run-2", "assess", 1, "assess", "direct_api", &serde_json::json!({}))
            .is_ok());
    }
}