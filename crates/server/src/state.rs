//! Shared application state for the HTTP server.

use std::sync::Arc;

use cisco_code_runtime::{RuntimeConfig, Store};

use crate::executor::JobExecutor;
use crate::jobs::JobManager;
use crate::provider_factory::ProviderFactory;

/// Shared state available to all request handlers.
///
/// All fields are behind `Arc` so the state can be cheaply cloned into
/// each axum handler (required by axum's `State` extractor).
#[derive(Clone)]
pub struct AppState {
    /// Job manager for creating and tracking agent jobs.
    pub jobs: Arc<JobManager>,
    /// Job executor — runs jobs against ConversationRuntime.
    pub executor: Arc<JobExecutor>,
    /// Persistent store (SQLite for local, PostgreSQL for server).
    pub store: Arc<dyn Store>,
    /// Provider factory for creating LLM instances per job.
    pub provider_factory: Arc<dyn ProviderFactory>,
    /// Base runtime config (jobs may override model/turns).
    pub config: RuntimeConfig,
    /// Server version string.
    pub version: String,
    /// Working directory for new jobs.
    pub default_cwd: String,
    /// Default model for new jobs.
    pub default_model: String,
}

impl AppState {
    /// Create a fully-wired AppState with all dependencies.
    pub fn new(
        store: Arc<dyn Store>,
        provider_factory: Arc<dyn ProviderFactory>,
        config: RuntimeConfig,
        default_cwd: String,
        max_concurrent: usize,
    ) -> Self {
        let jobs = Arc::new(JobManager::new(max_concurrent));
        let executor = Arc::new(JobExecutor::new(
            jobs.clone(),
            provider_factory.clone(),
            store.clone(),
            config.clone(),
        ));
        let default_model = config.model.clone();
        Self {
            jobs,
            executor,
            store,
            provider_factory,
            config,
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_cwd,
            default_model,
        }
    }

    /// Convenience: create a minimal state for testing (no real store/provider).
    #[cfg(test)]
    pub fn test(default_cwd: &str, default_model: &str) -> Self {
        use crate::jobs::JobManager;
        let jobs = Arc::new(JobManager::new(10));
        Self {
            jobs: jobs.clone(),
            executor: Arc::new(JobExecutor::new(
                jobs,
                Arc::new(crate::provider_factory::tests::NoopProviderFactory),
                Arc::new(crate::tests::NoopStore),
                RuntimeConfig::default(),
            )),
            store: Arc::new(crate::tests::NoopStore),
            provider_factory: Arc::new(crate::provider_factory::tests::NoopProviderFactory),
            config: RuntimeConfig::default(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_cwd: default_cwd.into(),
            default_model: default_model.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_clone() {
        let state = AppState::test("/tmp", "test-model");
        let cloned = state.clone();
        assert_eq!(cloned.default_cwd, "/tmp");
        assert!(Arc::ptr_eq(&state.jobs, &cloned.jobs));
    }

    #[test]
    fn test_app_state_version() {
        let state = AppState::test("/workspace", "claude-sonnet-4-6");
        assert!(!state.version.is_empty());
        assert_eq!(state.default_model, "claude-sonnet-4-6");
    }
}
