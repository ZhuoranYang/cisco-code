//! SQLite-based audit logging for enterprise compliance.
//!
//! Records all tool executions, session events, and errors to a persistent
//! SQLite database. Queryable by session, time range, and event type.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique entry ID (auto-generated on insert if empty).
    pub id: String,
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Session that produced this entry.
    pub session_id: String,
    /// Event type, e.g. "tool_execution", "permission_granted", "error".
    pub event_type: String,
    /// Name of the tool involved, if any.
    pub tool_name: Option<String>,
    /// Abbreviated input (first N chars) for compliance review.
    pub input_summary: Option<String>,
    /// Abbreviated output (first N chars) for compliance review.
    pub output_summary: Option<String>,
    /// Whether this event represents an error.
    pub is_error: bool,
    /// Duration of the operation in milliseconds, if applicable.
    pub duration_ms: Option<u64>,
    /// User identity, if known.
    pub user: Option<String>,
}

/// SQLite-backed audit logger.
pub struct AuditLogger {
    conn: Connection,
}

impl AuditLogger {
    /// Open (or create) an audit database at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let logger = Self { conn };
        logger.create_table()?;
        Ok(logger)
    }

    /// Create an in-memory audit database (for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let logger = Self { conn };
        logger.create_table()?;
        Ok(logger)
    }

    fn create_table(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id            TEXT PRIMARY KEY,
                timestamp     TEXT NOT NULL,
                session_id    TEXT NOT NULL,
                event_type    TEXT NOT NULL,
                tool_name     TEXT,
                input_summary TEXT,
                output_summary TEXT,
                is_error      INTEGER NOT NULL DEFAULT 0,
                duration_ms   INTEGER,
                user          TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_log(session_id);
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);",
        )?;
        Ok(())
    }

    /// Log an audit entry.
    ///
    /// If `entry.id` is empty, a new UUID is generated.
    pub fn log(&self, entry: &AuditEntry) -> Result<()> {
        let id = if entry.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            entry.id.clone()
        };
        self.conn.execute(
            "INSERT INTO audit_log (id, timestamp, session_id, event_type, tool_name,
             input_summary, output_summary, is_error, duration_ms, user)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                entry.timestamp.to_rfc3339(),
                entry.session_id,
                entry.event_type,
                entry.tool_name,
                entry.input_summary,
                entry.output_summary,
                entry.is_error as i32,
                entry.duration_ms.map(|d| d as i64),
                entry.user,
            ],
        )?;
        Ok(())
    }

    /// Query all entries for a session, ordered by timestamp ascending.
    pub fn query_session(&self, session_id: &str) -> Result<Vec<AuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, session_id, event_type, tool_name,
                    input_summary, output_summary, is_error, duration_ms, user
             FROM audit_log WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;

        let entries = stmt
            .query_map(params![session_id], |row| Ok(row_to_entry(row)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Query the most recent `limit` entries, ordered by timestamp descending.
    pub fn query_recent(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, session_id, event_type, tool_name,
                    input_summary, output_summary, is_error, duration_ms, user
             FROM audit_log ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let entries = stmt
            .query_map(params![limit as i64], |row| Ok(row_to_entry(row)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Count total entries.
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

/// Map a SQLite row to an `AuditEntry`.
fn row_to_entry(row: &rusqlite::Row<'_>) -> AuditEntry {
    let timestamp_str: String = row.get_unwrap(1);
    let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let is_error_int: i32 = row.get_unwrap(7);
    let duration_ms_opt: Option<i64> = row.get_unwrap(8);

    AuditEntry {
        id: row.get_unwrap(0),
        timestamp,
        session_id: row.get_unwrap(2),
        event_type: row.get_unwrap(3),
        tool_name: row.get_unwrap(4),
        input_summary: row.get_unwrap(5),
        output_summary: row.get_unwrap(6),
        is_error: is_error_int != 0,
        duration_ms: duration_ms_opt.map(|v| v as u64),
        user: row.get_unwrap(9),
    }
}

/// Create a new audit entry with common defaults filled in (convenience constructor).
pub fn make_entry(
    session_id: &str,
    event_type: &str,
    tool_name: Option<&str>,
) -> AuditEntry {
    AuditEntry {
        id: String::new(),
        timestamp: Utc::now(),
        session_id: session_id.to_string(),
        event_type: event_type.to_string(),
        tool_name: tool_name.map(String::from),
        input_summary: None,
        output_summary: None,
        is_error: false,
        duration_ms: None,
        user: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(session: &str, event_type: &str) -> AuditEntry {
        make_entry(session, event_type, None)
    }

    fn tool_entry(session: &str, tool: &str, is_error: bool) -> AuditEntry {
        AuditEntry {
            id: String::new(),
            timestamp: Utc::now(),
            session_id: session.to_string(),
            event_type: "tool_execution".to_string(),
            tool_name: Some(tool.to_string()),
            input_summary: Some("ls -la".to_string()),
            output_summary: Some("file.txt".to_string()),
            is_error,
            duration_ms: Some(150),
            user: Some("zhuoran".to_string()),
        }
    }

    #[test]
    fn test_create_and_count() {
        let logger = AuditLogger::in_memory().unwrap();
        assert_eq!(logger.count().unwrap(), 0);

        logger.log(&sample_entry("s1", "session_start")).unwrap();
        assert_eq!(logger.count().unwrap(), 1);

        logger.log(&sample_entry("s1", "session_start")).unwrap();
        assert_eq!(logger.count().unwrap(), 2);
    }

    #[test]
    fn test_log_and_query_session() {
        let logger = AuditLogger::in_memory().unwrap();
        logger.log(&sample_entry("s1", "session_start")).unwrap();
        logger.log(&tool_entry("s1", "Bash", false)).unwrap();
        logger.log(&sample_entry("s2", "session_start")).unwrap();

        let s1_entries = logger.query_session("s1").unwrap();
        assert_eq!(s1_entries.len(), 2);
        for e in &s1_entries {
            assert_eq!(e.session_id, "s1");
        }
    }

    #[test]
    fn test_query_recent() {
        let logger = AuditLogger::in_memory().unwrap();
        for i in 0..10 {
            let mut e = sample_entry("s1", &format!("event_{i}"));
            e.id = format!("id_{i}");
            logger.log(&e).unwrap();
        }

        let recent = logger.query_recent(3).unwrap();
        assert_eq!(recent.len(), 3);
        // Total count is still 10
        assert_eq!(logger.count().unwrap(), 10);
    }

    #[test]
    fn test_roundtrip_fields() {
        let logger = AuditLogger::in_memory().unwrap();
        let entry = AuditEntry {
            id: "fixed-id-123".to_string(),
            timestamp: Utc::now(),
            session_id: "sess-42".to_string(),
            event_type: "tool_execution".to_string(),
            tool_name: Some("Read".to_string()),
            input_summary: Some("/tmp/file.txt".to_string()),
            output_summary: Some("file contents...".to_string()),
            is_error: false,
            duration_ms: Some(250),
            user: Some("zhuoran".to_string()),
        };

        logger.log(&entry).unwrap();

        let entries = logger.query_session("sess-42").unwrap();
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.id, "fixed-id-123");
        assert_eq!(e.session_id, "sess-42");
        assert_eq!(e.event_type, "tool_execution");
        assert_eq!(e.tool_name.as_deref(), Some("Read"));
        assert_eq!(e.input_summary.as_deref(), Some("/tmp/file.txt"));
        assert_eq!(e.output_summary.as_deref(), Some("file contents..."));
        assert!(!e.is_error);
        assert_eq!(e.duration_ms, Some(250));
        assert_eq!(e.user.as_deref(), Some("zhuoran"));
    }

    #[test]
    fn test_error_entries() {
        let logger = AuditLogger::in_memory().unwrap();
        logger.log(&tool_entry("s1", "Bash", true)).unwrap();

        let entries = logger.query_session("s1").unwrap();
        assert!(entries[0].is_error);
    }

    #[test]
    fn test_empty_session_query() {
        let logger = AuditLogger::in_memory().unwrap();
        let entries = logger.query_session("nonexistent").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_null_optional_fields() {
        let logger = AuditLogger::in_memory().unwrap();
        let entry = AuditEntry {
            id: String::new(),
            timestamp: Utc::now(),
            session_id: "s1".to_string(),
            event_type: "generic".to_string(),
            tool_name: None,
            input_summary: None,
            output_summary: None,
            is_error: false,
            duration_ms: None,
            user: None,
        };
        logger.log(&entry).unwrap();

        let entries = logger.query_session("s1").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].tool_name.is_none());
        assert!(entries[0].input_summary.is_none());
        assert!(entries[0].output_summary.is_none());
        assert!(entries[0].duration_ms.is_none());
        assert!(entries[0].user.is_none());
    }

    #[test]
    fn test_make_entry_helper() {
        let entry = make_entry("sess-1", "tool_execution", Some("Bash"));
        assert_eq!(entry.session_id, "sess-1");
        assert_eq!(entry.event_type, "tool_execution");
        assert_eq!(entry.tool_name.as_deref(), Some("Bash"));
        assert!(!entry.is_error);
        assert!(entry.id.is_empty());
    }
}
