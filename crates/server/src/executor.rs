//! Job executor — bridges the job manager with ConversationRuntime.
//!
//! When a job is submitted, the executor:
//! 1. Marks it as running
//! 2. Creates a provider via the ProviderFactory
//! 3. Creates a ConversationRuntime (optionally backed by Store)
//! 4. Calls `submit_message(prompt)` to run the agent loop
//! 5. Broadcasts StreamEvents to subscribers
//! 6. Marks the job as completed or failed
//!
//! Each job runs in its own tokio task for concurrency.

use std::sync::Arc;

use cisco_code_api::Provider;
use cisco_code_protocol::StreamEvent;
use cisco_code_runtime::{ConversationRuntime, RuntimeConfig, Store};
use cisco_code_tools::ToolRegistry;

use crate::jobs::JobManager;
use crate::provider_factory::ProviderFactory;

/// Executes jobs by creating runtimes and running the agent loop.
pub struct JobExecutor {
    /// Shared job manager for status updates and event broadcasting.
    jobs: Arc<JobManager>,
    /// Factory for creating LLM provider instances.
    provider_factory: Arc<dyn ProviderFactory>,
    /// Persistent store for session data.
    store: Arc<dyn Store>,
    /// Base runtime config (individual jobs may override model/turns).
    base_config: RuntimeConfig,
}

impl JobExecutor {
    pub fn new(
        jobs: Arc<JobManager>,
        provider_factory: Arc<dyn ProviderFactory>,
        store: Arc<dyn Store>,
        base_config: RuntimeConfig,
    ) -> Self {
        Self {
            jobs,
            provider_factory,
            store,
            base_config,
        }
    }

    /// Execute a job in a new tokio task.
    ///
    /// Returns immediately; the job runs asynchronously. Use the job manager
    /// to track status and subscribe to events.
    pub fn spawn(&self, job_id: String, prompt: String, session_id: Option<String>, model: Option<String>, max_turns: Option<u32>) {
        let jobs = self.jobs.clone();
        let factory = self.provider_factory.clone();
        let store = self.store.clone();
        let mut config = self.base_config.clone();

        // Apply per-job overrides
        if let Some(ref m) = model {
            config.model = m.clone();
        }
        if let Some(turns) = max_turns {
            config.max_turns = turns;
        }

        tokio::spawn(async move {
            // 1. Mark running
            if let Err(e) = jobs.mark_running(&job_id).await {
                tracing::error!(job_id, "Failed to mark job as running: {e}");
                return;
            }

            // 2. Create provider
            let provider: Box<dyn Provider> = match factory.create(&config.model).await {
                Ok(p) => p,
                Err(e) => {
                    let msg = format!("Provider creation failed: {e}");
                    tracing::error!(job_id, "{msg}");
                    let _ = jobs.mark_failed(&job_id, msg).await;
                    return;
                }
            };

            // 3. Create runtime with store
            let tools = match ToolRegistry::with_builtins() {
                Ok(t) => t,
                Err(e) => {
                    let msg = format!("Tool registry init failed: {e}");
                    tracing::error!(job_id, "{msg}");
                    let _ = jobs.mark_failed(&job_id, msg).await;
                    return;
                }
            };

            let mut runtime = match ConversationRuntime::with_store(
                provider,
                tools,
                config,
                store,
                session_id.as_deref(),
            )
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("Runtime creation failed: {e}");
                    tracing::error!(job_id, "{msg}");
                    let _ = jobs.mark_failed(&job_id, msg).await;
                    return;
                }
            };

            // 4. Set up streaming channel — events broadcast in real-time
            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);

            // Spawn a task to forward events to job subscribers as they arrive
            let broadcast_jobs = jobs.clone();
            let broadcast_job_id = job_id.clone();
            let broadcast_handle = tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    broadcast_jobs.broadcast(&broadcast_job_id, event).await;
                }
            });

            // 5. Run the agent loop with real-time streaming
            match runtime.submit_message_streaming(&prompt, event_tx).await {
                Ok(events) => {
                    // Wait for broadcast task to finish draining
                    let _ = broadcast_handle.await;

                    // Extract final text output
                    let output: String = events
                        .iter()
                        .filter_map(|e| match e {
                            StreamEvent::TextDelta { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();

                    let turns = runtime.turn_count();
                    let _ = jobs.mark_completed(&job_id, output, turns).await;
                }
                Err(e) => {
                    let _ = broadcast_handle.await;
                    let msg = format!("Agent loop failed: {e}");
                    tracing::error!(job_id, "{msg}");
                    let _ = jobs.mark_failed(&job_id, msg).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    // Integration tests for the executor require a real provider + store,
    // so they live in tests/ or are gated behind a feature flag. Unit tests
    // for the executor logic are limited to verifying construction.

    use super::*;
    use crate::jobs::JobManager;

    #[test]
    fn executor_fields_accessible() {
        // Verify the struct layout compiles correctly.
        // Actual execution tested in integration tests.
        let _ = std::mem::size_of::<JobExecutor>();
    }
}
