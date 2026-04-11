//! SQLite implementation of the `Store` trait.
//!
//! Uses `rusqlite` (already in the workspace for audit logging) with
//! `tokio::sync::Mutex` to handle the `!Send` constraint of `Connection`.
//! All DB operations run via `spawn_blocking` to avoid blocking the async runtime.
//!
//! Schema: 6 tables (sessions, messages, cron_jobs, memories, session_routes, schema_version).
//! Location: `~/.cisco-code/cisco-code.db` (local) or configurable path (server).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::cron::{CronJob, CronSchedule};
use crate::memory::{MemoryEntry, MemoryType};
use crate::session::SessionMetadata;
use crate::store::{SessionSummary, Store, StoredSession};
use cisco_code_protocol::Message;

const SCHEMA_VERSION: i64 = 1;

/// SQLite-backed store for local daemon and development use.
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// Open (or create) a SQLite database at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open SQLite database at {path}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        // Run migrations synchronously during construction (before async runtime is needed).
        {
            let conn = store.conn.blocking_lock();
            Self::migrate(&conn)?;
        }
        Ok(store)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        {
            let conn = store.conn.blocking_lock();
            Self::migrate(&conn)?;
        }
        Ok(store)
    }

    /// Run schema migrations.
    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                user_id     TEXT NOT NULL DEFAULT 'local',
                name        TEXT,
                first_prompt TEXT,
                metadata    TEXT NOT NULL DEFAULT '{}',
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at);

            CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                seq         INTEGER NOT NULL,
                payload     TEXT NOT NULL,
                created_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, seq);

            CREATE TABLE IF NOT EXISTS cron_jobs (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                prompt      TEXT NOT NULL,
                schedule    TEXT NOT NULL,
                enabled     INTEGER NOT NULL DEFAULT 1,
                last_run    TEXT,
                next_run    TEXT,
                run_count   INTEGER NOT NULL DEFAULT 0,
                cwd         TEXT,
                model       TEXT,
                created_at  TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS memories (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id     TEXT NOT NULL DEFAULT 'local',
                filename    TEXT NOT NULL,
                name        TEXT NOT NULL,
                description TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                content     TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                UNIQUE(user_id, filename)
            );
            CREATE INDEX IF NOT EXISTS idx_memories_user ON memories(user_id);

            CREATE TABLE IF NOT EXISTS session_routes (
                user_id     TEXT NOT NULL,
                channel     TEXT NOT NULL,
                thread_id   TEXT NOT NULL DEFAULT '',
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                created_at  TEXT NOT NULL,
                PRIMARY KEY (user_id, channel, thread_id)
            );",
        )?;

        // Upsert schema version.
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))?;
        if count == 0 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        }

        Ok(())
    }

    /// Helper: run a blocking closure on the connection.
    async fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            f(&conn)
        })
        .await?
    }
}

#[async_trait]
impl Store for SqliteStore {
    // ── Sessions ──────────────────────────────────────────────────────────

    async fn create_session(&self, session: &StoredSession) -> Result<()> {
        let id = session.id.clone();
        let user_id = session.user_id.clone();
        let name = session.metadata.name.clone();
        let first_prompt = session.metadata.first_prompt.clone();
        let metadata = serde_json::to_string(&session.metadata)?;
        let created_at = session.created_at.to_rfc3339();
        let updated_at = session.updated_at.to_rfc3339();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO sessions (id, user_id, name, first_prompt, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, user_id, name, first_prompt, metadata, created_at, updated_at],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_session(&self, id: &str) -> Result<Option<StoredSession>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, user_id, metadata, created_at, updated_at FROM sessions WHERE id = ?1",
            )?;
            let result = stmt
                .query_row(params![id], |row| {
                    let id: String = row.get(0)?;
                    let user_id: String = row.get(1)?;
                    let metadata_json: String = row.get(2)?;
                    let created_at_str: String = row.get(3)?;
                    let updated_at_str: String = row.get(4)?;
                    Ok((id, user_id, metadata_json, created_at_str, updated_at_str))
                })
                .optional()?;

            match result {
                Some((id, user_id, metadata_json, created_at_str, updated_at_str)) => {
                    let metadata: SessionMetadata = serde_json::from_str(&metadata_json)
                        .unwrap_or_default();
                    let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    Ok(Some(StoredSession {
                        id,
                        user_id,
                        metadata,
                        created_at,
                        updated_at,
                    }))
                }
                None => Ok(None),
            }
        })
        .await
    }

    async fn list_sessions(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>> {
        let user_id = user_id.map(String::from);
        self.with_conn(move |conn| {
            let (sql, values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match &user_id {
                Some(uid) => (
                    "SELECT id, user_id, name, first_prompt, metadata, created_at, updated_at
                     FROM sessions WHERE user_id = ?1 ORDER BY updated_at DESC LIMIT ?2"
                        .to_string(),
                    vec![
                        Box::new(uid.clone()) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(limit as i64),
                    ],
                ),
                None => (
                    "SELECT id, user_id, name, first_prompt, metadata, created_at, updated_at
                     FROM sessions ORDER BY updated_at DESC LIMIT ?1"
                        .to_string(),
                    vec![Box::new(limit as i64) as Box<dyn rusqlite::types::ToSql>],
                ),
            };

            let mut stmt = conn.prepare(&sql)?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    let id: String = row.get(0)?;
                    let user_id: String = row.get(1)?;
                    let name: Option<String> = row.get(2)?;
                    let first_prompt: Option<String> = row.get(3)?;
                    let metadata_json: String = row.get(4)?;
                    let created_at_str: String = row.get(5)?;
                    let updated_at_str: String = row.get(6)?;
                    Ok((
                        id,
                        user_id,
                        name,
                        first_prompt,
                        metadata_json,
                        created_at_str,
                        updated_at_str,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let mut summaries = Vec::with_capacity(rows.len());
            for (id, user_id, name, first_prompt, metadata_json, created_at_str, updated_at_str) in
                rows
            {
                let metadata: SessionMetadata =
                    serde_json::from_str(&metadata_json).unwrap_or_default();
                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                summaries.push(SessionSummary {
                    id,
                    user_id,
                    name,
                    first_prompt,
                    message_count: metadata.message_count,
                    turn_count: metadata.turn_count,
                    cost_usd: metadata.cost_usd,
                    forked_from: metadata.forked_from.clone(),
                    created_at,
                    updated_at,
                });
            }
            Ok(summaries)
        })
        .await
    }

    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()> {
        let session_id = session_id.to_string();
        let payload = serde_json::to_string(message)?;
        let now = Utc::now().to_rfc3339();

        self.with_conn(move |conn| {
            // Get next sequence number.
            let seq: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .unwrap_or(1);

            conn.execute(
                "INSERT INTO messages (session_id, seq, payload, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![session_id, seq, payload, now],
            )?;

            // Update session's updated_at.
            conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![now, session_id],
            )?;

            Ok(())
        })
        .await
    }

    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let session_id = session_id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT payload FROM messages WHERE session_id = ?1 ORDER BY seq ASC",
            )?;
            let messages = stmt
                .query_map(params![session_id], |row| {
                    let payload: String = row.get(0)?;
                    Ok(payload)
                })?
                .filter_map(|r| r.ok())
                .filter_map(|payload| serde_json::from_str::<Message>(&payload).ok())
                .collect();
            Ok(messages)
        })
        .await
    }

    async fn update_metadata(
        &self,
        session_id: &str,
        metadata: &SessionMetadata,
    ) -> Result<()> {
        let session_id = session_id.to_string();
        let metadata_json = serde_json::to_string(metadata)?;
        let name = metadata.name.clone();
        let first_prompt = metadata.first_prompt.clone();
        let now = Utc::now().to_rfc3339();

        self.with_conn(move |conn| {
            conn.execute(
                "UPDATE sessions SET metadata = ?1, name = ?2, first_prompt = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![metadata_json, name, first_prompt, now, session_id],
            )?;
            Ok(())
        })
        .await
    }

    async fn delete_session(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            // CASCADE deletes messages and routes.
            let changed = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
            Ok(changed > 0)
        })
        .await
    }

    // ── Cron Jobs ─────────────────────────────────────────────────────────

    async fn save_cron_job(&self, job: &CronJob) -> Result<()> {
        let id = job.id.clone();
        let name = job.name.clone();
        let prompt = job.prompt.clone();
        let schedule = serde_json::to_string(&job.schedule)?;
        let enabled = job.enabled;
        let last_run = job.last_run.map(|dt| dt.to_rfc3339());
        let next_run = job.next_run.map(|dt| dt.to_rfc3339());
        let run_count = job.run_count as i64;
        let cwd = job.cwd.clone();
        let model = job.model.clone();
        let created_at = job.created_at.to_rfc3339();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO cron_jobs
                 (id, name, prompt, schedule, enabled, last_run, next_run, run_count, cwd, model, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id, name, prompt, schedule, enabled as i32, last_run, next_run,
                    run_count, cwd, model, created_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, prompt, schedule, enabled, last_run, next_run, run_count, cwd, model, created_at
                 FROM cron_jobs WHERE id = ?1",
            )?;
            let result = stmt
                .query_row(params![id], |row| {
                    Ok(row_to_cron_job(row))
                })
                .optional()?;
            Ok(result)
        })
        .await
    }

    async fn list_cron_jobs(&self) -> Result<Vec<CronJob>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, prompt, schedule, enabled, last_run, next_run, run_count, cwd, model, created_at
                 FROM cron_jobs ORDER BY created_at ASC",
            )?;
            let jobs = stmt
                .query_map([], |row| Ok(row_to_cron_job(row)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(jobs)
        })
        .await
    }

    async fn delete_cron_job(&self, id: &str) -> Result<bool> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let changed = conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])?;
            Ok(changed > 0)
        })
        .await
    }

    async fn update_cron_run(
        &self,
        id: &str,
        last_run: DateTime<Utc>,
        next_run: Option<DateTime<Utc>>,
        run_count: u64,
    ) -> Result<()> {
        let id = id.to_string();
        let last_run_str = last_run.to_rfc3339();
        let next_run_str = next_run.map(|dt| dt.to_rfc3339());

        self.with_conn(move |conn| {
            conn.execute(
                "UPDATE cron_jobs SET last_run = ?1, next_run = ?2, run_count = ?3 WHERE id = ?4",
                params![last_run_str, next_run_str, run_count as i64, id],
            )?;
            Ok(())
        })
        .await
    }

    // ── Memory ────────────────────────────────────────────────────────────

    async fn save_memory(&self, user_id: &str, entry: &MemoryEntry) -> Result<()> {
        let user_id = user_id.to_string();
        let filename = entry.filename.clone();
        let name = entry.name.clone();
        let description = entry.description.clone();
        let memory_type = serde_json::to_string(&entry.memory_type)?;
        let content = entry.content.clone();
        let now = Utc::now().to_rfc3339();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO memories
                 (user_id, filename, name, description, memory_type, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![user_id, filename, name, description, memory_type, content, now],
            )?;
            Ok(())
        })
        .await
    }

    async fn list_memories(&self, user_id: &str) -> Result<Vec<MemoryEntry>> {
        let user_id = user_id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT filename, name, description, memory_type, content
                 FROM memories WHERE user_id = ?1 ORDER BY name ASC",
            )?;
            let entries = stmt
                .query_map(params![user_id], |row| {
                    Ok(row_to_memory_entry(row))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(entries)
        })
        .await
    }

    async fn delete_memory(&self, user_id: &str, filename: &str) -> Result<bool> {
        let user_id = user_id.to_string();
        let filename = filename.to_string();
        self.with_conn(move |conn| {
            let changed = conn.execute(
                "DELETE FROM memories WHERE user_id = ?1 AND filename = ?2",
                params![user_id, filename],
            )?;
            Ok(changed > 0)
        })
        .await
    }

    async fn search_memories(
        &self,
        user_id: &str,
        keyword: &str,
    ) -> Result<Vec<MemoryEntry>> {
        let user_id = user_id.to_string();
        let pattern = format!("%{keyword}%");
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT filename, name, description, memory_type, content
                 FROM memories
                 WHERE user_id = ?1 AND (content LIKE ?2 OR name LIKE ?2 OR description LIKE ?2)
                 ORDER BY name ASC",
            )?;
            let entries = stmt
                .query_map(params![user_id, pattern], |row| {
                    Ok(row_to_memory_entry(row))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(entries)
        })
        .await
    }

    // ── Session Routing ───────────────────────────────────────────────────

    async fn resolve_session(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>> {
        let user_id = user_id.to_string();
        let channel = channel.to_string();
        let thread_id = thread_id.unwrap_or("").to_string();

        self.with_conn(move |conn| {
            let result: Option<String> = conn
                .query_row(
                    "SELECT session_id FROM session_routes
                     WHERE user_id = ?1 AND channel = ?2 AND thread_id = ?3",
                    params![user_id, channel, thread_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(result)
        })
        .await
    }

    async fn bind_session(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
        session_id: &str,
    ) -> Result<()> {
        let user_id = user_id.to_string();
        let channel = channel.to_string();
        let thread_id = thread_id.unwrap_or("").to_string();
        let session_id = session_id.to_string();
        let now = Utc::now().to_rfc3339();

        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO session_routes
                 (user_id, channel, thread_id, session_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![user_id, channel, thread_id, session_id, now],
            )?;
            Ok(())
        })
        .await
    }
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn row_to_cron_job(row: &rusqlite::Row<'_>) -> CronJob {
    let schedule_json: String = row.get_unwrap(3);
    let schedule: CronSchedule =
        serde_json::from_str(&schedule_json).unwrap_or(CronSchedule::Interval(3600));
    let enabled_int: i32 = row.get_unwrap(4);
    let last_run: Option<String> = row.get_unwrap(5);
    let next_run: Option<String> = row.get_unwrap(6);
    let run_count: i64 = row.get_unwrap(7);
    let created_at_str: String = row.get_unwrap(10);

    CronJob {
        id: row.get_unwrap(0),
        name: row.get_unwrap(1),
        prompt: row.get_unwrap(2),
        schedule,
        enabled: enabled_int != 0,
        last_run: last_run.as_deref().map(parse_datetime),
        next_run: next_run.as_deref().map(parse_datetime),
        run_count: run_count as u64,
        cwd: row.get_unwrap(8),
        model: row.get_unwrap(9),
        created_at: parse_datetime(&created_at_str),
    }
}

fn row_to_memory_entry(row: &rusqlite::Row<'_>) -> MemoryEntry {
    let memory_type_str: String = row.get_unwrap(3);
    let memory_type: MemoryType =
        serde_json::from_str(&format!("\"{memory_type_str}\"")).unwrap_or(MemoryType::Project);

    MemoryEntry {
        filename: row.get_unwrap(0),
        name: row.get_unwrap(1),
        description: row.get_unwrap(2),
        memory_type,
        content: row.get_unwrap(4),
    }
}

// We need this import for `.optional()` on query results.
use rusqlite::OptionalExtension;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::CronSchedule;
    use crate::memory::MemoryType;
    use crate::session::SessionMetadata;

    fn make_session(id: &str, user_id: &str) -> StoredSession {
        StoredSession {
            id: id.to_string(),
            user_id: user_id.to_string(),
            metadata: SessionMetadata {
                name: Some(format!("Session {id}")),
                first_prompt: Some("hello".into()),
                turn_count: 3,
                cost_usd: 0.05,
                ..Default::default()
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_user_message(text: &str) -> Message {
        Message::User(cisco_code_protocol::UserMessage {
            id: uuid::Uuid::new_v4(),
            content: vec![cisco_code_protocol::ContentBlock::Text {
                text: text.to_string(),
            }],
            attachments: None,
        })
    }

    #[tokio::test]
    async fn test_session_crud() {
        let store = SqliteStore::in_memory().unwrap();

        // Create
        let session = make_session("s1", "local");
        store.create_session(&session).await.unwrap();

        // Get
        let loaded = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(loaded.id, "s1");
        assert_eq!(loaded.user_id, "local");
        assert_eq!(loaded.metadata.name, Some("Session s1".into()));

        // List
        let list = store.list_sessions(Some("local"), 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "s1");
        assert_eq!(list[0].turn_count, 3);

        // Delete
        assert!(store.delete_session("s1").await.unwrap());
        assert!(store.get_session("s1").await.unwrap().is_none());
        assert!(!store.delete_session("s1").await.unwrap());
    }

    #[tokio::test]
    async fn test_message_append_and_get() {
        let store = SqliteStore::in_memory().unwrap();
        store.create_session(&make_session("s1", "local")).await.unwrap();

        store.append_message("s1", &make_user_message("Hello")).await.unwrap();
        store.append_message("s1", &make_user_message("World")).await.unwrap();

        let msgs = store.get_messages("s1").await.unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[tokio::test]
    async fn test_session_metadata_update() {
        let store = SqliteStore::in_memory().unwrap();
        store.create_session(&make_session("s1", "local")).await.unwrap();

        let mut meta = SessionMetadata::default();
        meta.name = Some("Renamed".into());
        meta.cost_usd = 1.23;
        meta.turn_count = 10;
        store.update_metadata("s1", &meta).await.unwrap();

        let loaded = store.get_session("s1").await.unwrap().unwrap();
        assert_eq!(loaded.metadata.name, Some("Renamed".into()));
        assert!((loaded.metadata.cost_usd - 1.23).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_cron_crud() {
        let store = SqliteStore::in_memory().unwrap();

        let job = CronJob {
            id: "cron-1".into(),
            name: "test job".into(),
            prompt: "/verify".into(),
            schedule: CronSchedule::Interval(300),
            created_at: Utc::now(),
            last_run: None,
            next_run: Some(Utc::now()),
            run_count: 0,
            enabled: true,
            cwd: Some("/tmp".into()),
            model: None,
        };

        store.save_cron_job(&job).await.unwrap();

        let loaded = store.get_cron_job("cron-1").await.unwrap().unwrap();
        assert_eq!(loaded.name, "test job");
        assert_eq!(loaded.prompt, "/verify");
        assert!(loaded.enabled);

        let list = store.list_cron_jobs().await.unwrap();
        assert_eq!(list.len(), 1);

        store
            .update_cron_run("cron-1", Utc::now(), Some(Utc::now()), 1)
            .await
            .unwrap();
        let updated = store.get_cron_job("cron-1").await.unwrap().unwrap();
        assert_eq!(updated.run_count, 1);
        assert!(updated.last_run.is_some());

        assert!(store.delete_cron_job("cron-1").await.unwrap());
        assert!(store.list_cron_jobs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_memory_crud() {
        let store = SqliteStore::in_memory().unwrap();

        let entry = MemoryEntry {
            name: "User Role".into(),
            description: "Zhuoran is a Rust developer".into(),
            memory_type: MemoryType::User,
            content: "Senior engineer at Cisco, building AI agents".into(),
            filename: "user_role.md".into(),
        };

        store.save_memory("local", &entry).await.unwrap();

        let list = store.list_memories("local").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "User Role");
        assert_eq!(list[0].filename, "user_role.md");

        // Search
        let found = store.search_memories("local", "Cisco").await.unwrap();
        assert_eq!(found.len(), 1);
        let not_found = store.search_memories("local", "nonexistent").await.unwrap();
        assert!(not_found.is_empty());

        // Delete
        assert!(store.delete_memory("local", "user_role.md").await.unwrap());
        assert!(store.list_memories("local").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_session_routing() {
        let store = SqliteStore::in_memory().unwrap();
        store.create_session(&make_session("s1", "user1")).await.unwrap();

        // No binding yet
        assert!(store
            .resolve_session("user1", "slack", None)
            .await
            .unwrap()
            .is_none());

        // Bind
        store
            .bind_session("user1", "slack", None, "s1")
            .await
            .unwrap();

        // Resolve
        let sid = store
            .resolve_session("user1", "slack", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sid, "s1");

        // Different channel = no binding
        assert!(store
            .resolve_session("user1", "webex", None)
            .await
            .unwrap()
            .is_none());

        // Thread-specific binding
        store
            .bind_session("user1", "slack", Some("thread-42"), "s1")
            .await
            .unwrap();
        let sid = store
            .resolve_session("user1", "slack", Some("thread-42"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sid, "s1");
    }

    #[tokio::test]
    async fn test_list_sessions_all_users() {
        let store = SqliteStore::in_memory().unwrap();
        store.create_session(&make_session("s1", "user1")).await.unwrap();
        store.create_session(&make_session("s2", "user2")).await.unwrap();

        // List all
        let all = store.list_sessions(None, 10).await.unwrap();
        assert_eq!(all.len(), 2);

        // List by user
        let u1 = store.list_sessions(Some("user1"), 10).await.unwrap();
        assert_eq!(u1.len(), 1);
        assert_eq!(u1[0].id, "s1");
    }

    #[tokio::test]
    async fn test_memory_upsert() {
        let store = SqliteStore::in_memory().unwrap();

        let entry = MemoryEntry {
            name: "Feedback".into(),
            description: "Don't use Python".into(),
            memory_type: MemoryType::Feedback,
            content: "User prefers pure Rust".into(),
            filename: "feedback_rust.md".into(),
        };
        store.save_memory("local", &entry).await.unwrap();

        // Upsert with same filename
        let entry2 = MemoryEntry {
            name: "Feedback Updated".into(),
            description: "Don't use Python, ever".into(),
            memory_type: MemoryType::Feedback,
            content: "User strongly prefers pure Rust, no Python runtime".into(),
            filename: "feedback_rust.md".into(),
        };
        store.save_memory("local", &entry2).await.unwrap();

        let list = store.list_memories("local").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Feedback Updated");
    }

    #[tokio::test]
    async fn test_cascade_delete() {
        let store = SqliteStore::in_memory().unwrap();
        store.create_session(&make_session("s1", "local")).await.unwrap();
        store.append_message("s1", &make_user_message("test")).await.unwrap();
        store.bind_session("local", "slack", None, "s1").await.unwrap();

        // Delete session cascades messages and routes
        store.delete_session("s1").await.unwrap();
        assert!(store.get_messages("s1").await.unwrap().is_empty());
        assert!(store.resolve_session("local", "slack", None).await.unwrap().is_none());
    }
}
