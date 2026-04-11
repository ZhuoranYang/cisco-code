//! Storage abstraction for cisco-code persistent state.
//!
//! The `Store` trait abstracts all persistence: sessions, messages, cron jobs,
//! memory entries, and session routing. Implementations:
//! - `SqliteStore` (local daemon, single-user, zero-config)
//! - PostgreSQL (server daemon, multi-user — future)
//!
//! Design: single trait (not 4 separate) because both SQLite and PostgreSQL
//! serve all domains from one connection pool.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cron::CronJob;
use crate::memory::MemoryEntry;
use crate::session::SessionMetadata;
use cisco_code_protocol::Message;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A session record as stored in the database.
///
/// Distinct from the in-memory `Session` which carries the full message list.
/// `StoredSession` holds the header; messages are stored separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub id: String,
    /// Owner of this session. "local" for single-user/local daemon.
    pub user_id: String,
    pub metadata: SessionMetadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Lightweight summary for session listing (no messages).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub user_id: String,
    pub name: Option<String>,
    pub first_prompt: Option<String>,
    pub message_count: usize,
    pub turn_count: u32,
    pub cost_usd: f64,
    pub forked_from: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Store trait
// ---------------------------------------------------------------------------

/// Unified storage abstraction for all cisco-code persistent state.
///
/// All methods are async to support both SQLite (via spawn_blocking) and
/// future PostgreSQL (natively async). Errors are propagated as `anyhow::Error`.
#[async_trait]
pub trait Store: Send + Sync {
    // ── Sessions ──────────────────────────────────────────────────────────

    /// Create a new session record.
    async fn create_session(&self, session: &StoredSession) -> Result<()>;

    /// Get a session by ID (header only, no messages).
    async fn get_session(&self, id: &str) -> Result<Option<StoredSession>>;

    /// List sessions, optionally filtered by user_id, newest first.
    async fn list_sessions(
        &self,
        user_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionSummary>>;

    /// Append a message to a session's message log.
    async fn append_message(&self, session_id: &str, message: &Message) -> Result<()>;

    /// Get all messages for a session, in order.
    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>>;

    /// Update session metadata (cost, turn count, name, etc.).
    async fn update_metadata(
        &self,
        session_id: &str,
        metadata: &SessionMetadata,
    ) -> Result<()>;

    /// Delete a session and its messages. Returns true if it existed.
    async fn delete_session(&self, id: &str) -> Result<bool>;

    // ── Cron Jobs ─────────────────────────────────────────────────────────

    /// Save (insert or replace) a cron job.
    async fn save_cron_job(&self, job: &CronJob) -> Result<()>;

    /// Get a cron job by ID.
    async fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>>;

    /// List all cron jobs.
    async fn list_cron_jobs(&self) -> Result<Vec<CronJob>>;

    /// Delete a cron job. Returns true if it existed.
    async fn delete_cron_job(&self, id: &str) -> Result<bool>;

    /// Record that a cron job ran.
    async fn update_cron_run(
        &self,
        id: &str,
        last_run: DateTime<Utc>,
        next_run: Option<DateTime<Utc>>,
        run_count: u64,
    ) -> Result<()>;

    // ── Memory ────────────────────────────────────────────────────────────

    /// Save (insert or replace) a memory entry.
    async fn save_memory(&self, user_id: &str, entry: &MemoryEntry) -> Result<()>;

    /// List all memory entries for a user.
    async fn list_memories(&self, user_id: &str) -> Result<Vec<MemoryEntry>>;

    /// Delete a memory entry. Returns true if it existed.
    async fn delete_memory(&self, user_id: &str, filename: &str) -> Result<bool>;

    /// Search memories by keyword (case-insensitive content match).
    async fn search_memories(
        &self,
        user_id: &str,
        keyword: &str,
    ) -> Result<Vec<MemoryEntry>>;

    // ── Session Routing ───────────────────────────────────────────────────

    /// Resolve an existing session for the given (user, channel, thread).
    async fn resolve_session(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>>;

    /// Bind a (user, channel, thread) tuple to a session ID.
    async fn bind_session(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
        session_id: &str,
    ) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_session_serializes() {
        let s = StoredSession {
            id: "test-123".into(),
            user_id: "local".into(),
            metadata: SessionMetadata::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: StoredSession = serde_json::from_str(&json).unwrap();
        assert_eq!(s.id, s2.id);
        assert_eq!(s.user_id, s2.user_id);
    }

    #[test]
    fn session_summary_serializes() {
        let s = SessionSummary {
            id: "test-456".into(),
            user_id: "local".into(),
            name: Some("My session".into()),
            first_prompt: Some("Hello world".into()),
            message_count: 10,
            turn_count: 5,
            cost_usd: 0.42,
            forked_from: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let s2: SessionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(s.id, s2.id);
        assert_eq!(s.name, s2.name);
        assert_eq!(s.message_count, 10);
    }
}
