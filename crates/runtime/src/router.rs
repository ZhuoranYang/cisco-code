//! Session routing — maps (user_id, channel, thread_id) to session_id.
//!
//! When a message arrives from Slack/Webex/WebSocket, the router determines
//! which session it belongs to. If no session exists for the tuple, a new
//! one is created.
//!
//! The router is backed by the `Store` trait, so routing state persists
//! across daemon restarts.

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;

use crate::session::{Session, SessionMetadata};
use crate::store::{Store, StoredSession};

/// Resolves incoming messages to sessions.
pub struct SessionRouter {
    store: Arc<dyn Store>,
}

impl SessionRouter {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// Resolve an existing session or create a new one.
    ///
    /// Returns the session_id. If a session already exists for the given
    /// (user_id, channel, thread_id) tuple, returns that session's ID.
    /// Otherwise, creates a new session, binds it, and returns the new ID.
    pub async fn resolve_or_create(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<String> {
        // Check for existing binding
        if let Some(session_id) = self.store.resolve_session(user_id, channel, thread_id).await? {
            // Verify session still exists (could have been deleted)
            if self.store.get_session(&session_id).await?.is_some() {
                return Ok(session_id);
            }
            // Session was deleted — fall through to create new one
        }

        // Create new session
        let session = Session::new();
        let now = Utc::now();
        let stored = StoredSession {
            id: session.id.clone(),
            user_id: user_id.to_string(),
            metadata: SessionMetadata::default(),
            created_at: now,
            updated_at: now,
        };
        self.store.create_session(&stored).await?;

        // Bind the route
        self.store
            .bind_session(user_id, channel, thread_id, &session.id)
            .await?;

        Ok(session.id)
    }

    /// Look up an existing session without creating one.
    pub async fn resolve(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<Option<String>> {
        self.store.resolve_session(user_id, channel, thread_id).await
    }

    /// Explicitly bind a channel/thread to an existing session.
    pub async fn bind(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
        session_id: &str,
    ) -> Result<()> {
        self.store
            .bind_session(user_id, channel, thread_id, session_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store_sqlite::SqliteStore;

    #[tokio::test]
    async fn test_resolve_or_create_new() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let router = SessionRouter::new(store.clone());

        let sid = router
            .resolve_or_create("user1", "slack", Some("thread-1"))
            .await
            .unwrap();
        assert!(!sid.is_empty());

        // Same tuple → same session
        let sid2 = router
            .resolve_or_create("user1", "slack", Some("thread-1"))
            .await
            .unwrap();
        assert_eq!(sid, sid2);
    }

    #[tokio::test]
    async fn test_different_threads_different_sessions() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let router = SessionRouter::new(store);

        let sid1 = router
            .resolve_or_create("user1", "slack", Some("t1"))
            .await
            .unwrap();
        let sid2 = router
            .resolve_or_create("user1", "slack", Some("t2"))
            .await
            .unwrap();
        assert_ne!(sid1, sid2);
    }

    #[tokio::test]
    async fn test_resolve_returns_none() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let router = SessionRouter::new(store);

        let result = router.resolve("user1", "slack", None).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_explicit_bind() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());
        let router = SessionRouter::new(store.clone());

        // Create a session first
        let session = Session::new();
        let stored = StoredSession {
            id: session.id.clone(),
            user_id: "user1".into(),
            metadata: SessionMetadata::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        store.create_session(&stored).await.unwrap();

        // Bind explicitly
        router
            .bind("user1", "webex", None, &session.id)
            .await
            .unwrap();

        let resolved = router.resolve("user1", "webex", None).await.unwrap();
        assert_eq!(resolved, Some(session.id));
    }
}
