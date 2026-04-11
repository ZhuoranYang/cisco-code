//! cisco-code-server: HTTP server for remote agent access.
//!
//! Enables running cisco-code as a persistent service. Users interact via:
//! - REST API: Submit jobs, query status, manage sessions
//! - SSE: Real-time streaming of agent events
//! - WebSocket: Bidirectional session control (DirectConnect-style)
//!
//! Architecture: axum router → job manager → executor → ConversationRuntime
//! The CLI can run in `--remote` mode, acting as a thin client to this server.

pub mod executor;
pub mod jobs;
pub mod provider_factory;
pub mod routes;
pub mod state;
pub mod streaming;
pub mod websocket;

pub use executor::JobExecutor;
pub use jobs::{Job, JobId, JobManager, JobStatus};
pub use provider_factory::{DefaultProviderFactory, ProviderFactory};
pub use state::AppState;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use cisco_code_protocol::Message;
    use cisco_code_runtime::store::{SessionSummary, StoredSession};
    use cisco_code_runtime::{CronJob, MemoryEntry, Store};

    /// No-op store for tests that don't need persistence.
    pub struct NoopStore;

    #[async_trait]
    impl Store for NoopStore {
        async fn create_session(&self, _session: &StoredSession) -> Result<()> { Ok(()) }
        async fn get_session(&self, _id: &str) -> Result<Option<StoredSession>> { Ok(None) }
        async fn list_sessions(&self, _user_id: Option<&str>, _limit: usize) -> Result<Vec<SessionSummary>> { Ok(vec![]) }
        async fn append_message(&self, _session_id: &str, _message: &Message) -> Result<()> { Ok(()) }
        async fn get_messages(&self, _session_id: &str) -> Result<Vec<Message>> { Ok(vec![]) }
        async fn update_metadata(&self, _session_id: &str, _metadata: &cisco_code_runtime::SessionMetadata) -> Result<()> { Ok(()) }
        async fn delete_session(&self, _id: &str) -> Result<bool> { Ok(false) }
        async fn save_cron_job(&self, _job: &CronJob) -> Result<()> { Ok(()) }
        async fn get_cron_job(&self, _id: &str) -> Result<Option<CronJob>> { Ok(None) }
        async fn list_cron_jobs(&self) -> Result<Vec<CronJob>> { Ok(vec![]) }
        async fn delete_cron_job(&self, _id: &str) -> Result<bool> { Ok(false) }
        async fn update_cron_run(&self, _id: &str, _last_run: DateTime<Utc>, _next_run: Option<DateTime<Utc>>, _run_count: u64) -> Result<()> { Ok(()) }
        async fn save_memory(&self, _user_id: &str, _entry: &MemoryEntry) -> Result<()> { Ok(()) }
        async fn list_memories(&self, _user_id: &str) -> Result<Vec<MemoryEntry>> { Ok(vec![]) }
        async fn delete_memory(&self, _user_id: &str, _filename: &str) -> Result<bool> { Ok(false) }
        async fn search_memories(&self, _user_id: &str, _keyword: &str) -> Result<Vec<MemoryEntry>> { Ok(vec![]) }
        async fn resolve_session(&self, _user_id: &str, _channel: &str, _thread_id: Option<&str>) -> Result<Option<String>> { Ok(None) }
        async fn bind_session(&self, _user_id: &str, _channel: &str, _thread_id: Option<&str>, _session_id: &str) -> Result<()> { Ok(()) }
    }
}
