//! Job management for async agent task execution.
//!
//! A "job" is a user-submitted task that the agent works on asynchronously.
//! Jobs have lifecycle states and produce streaming events.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

use cisco_code_protocol::StreamEvent;

/// Unique job identifier.
pub type JobId = String;

/// Job submission request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRequest {
    /// The user's prompt / task description.
    pub prompt: String,
    /// Optional session ID to continue.
    pub session_id: Option<String>,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional max turns.
    pub max_turns: Option<u32>,
    /// Optional working directory.
    pub cwd: Option<String>,
}

/// Current status of a job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Queued, waiting to start.
    Queued,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled by user.
    Cancelled,
}

/// A job tracks the lifecycle of an agent task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub status: JobStatus,
    pub request: JobRequest,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Final output text (populated on completion).
    pub output: Option<String>,
    /// Error message (populated on failure).
    pub error: Option<String>,
    /// Number of turns executed.
    pub turns: u32,
}

impl Job {
    /// Create a new queued job.
    pub fn new(request: JobRequest) -> Self {
        let session_id = request
            .session_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        Self {
            id: Uuid::new_v4().to_string(),
            status: JobStatus::Queued,
            request,
            session_id,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            output: None,
            error: None,
            turns: 0,
        }
    }

    /// Mark job as running.
    pub fn start(&mut self) {
        self.status = JobStatus::Running;
        self.started_at = Some(Utc::now());
    }

    /// Mark job as completed.
    pub fn complete(&mut self, output: String, turns: u32) {
        self.status = JobStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.output = Some(output);
        self.turns = turns;
    }

    /// Mark job as failed.
    pub fn fail(&mut self, error: String) {
        self.status = JobStatus::Failed;
        self.completed_at = Some(Utc::now());
        self.error = Some(error);
    }

    /// Mark job as cancelled.
    pub fn cancel(&mut self) {
        self.status = JobStatus::Cancelled;
        self.completed_at = Some(Utc::now());
    }

    /// Duration in seconds (if started).
    pub fn duration_secs(&self) -> Option<f64> {
        let start = self.started_at?;
        let end = self.completed_at.unwrap_or_else(Utc::now);
        Some((end - start).num_milliseconds() as f64 / 1000.0)
    }
}

/// Manages job lifecycle and event distribution.
pub struct JobManager {
    /// All jobs indexed by ID.
    jobs: RwLock<HashMap<JobId, Job>>,
    /// Event senders for streaming subscribers (job_id → Vec<sender>).
    subscribers: RwLock<HashMap<JobId, Vec<mpsc::Sender<StreamEvent>>>>,
    /// Maximum concurrent jobs.
    max_concurrent: usize,
    /// Currently running job count.
    running_count: Mutex<usize>,
}

impl JobManager {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            subscribers: RwLock::new(HashMap::new()),
            max_concurrent,
            running_count: Mutex::new(0),
        }
    }

    /// Submit a new job. Returns the job ID.
    pub async fn submit(&self, request: JobRequest) -> Result<Job, JobError> {
        let running = self.running_count.lock().await;
        if *running >= self.max_concurrent {
            return Err(JobError::QueueFull {
                max: self.max_concurrent,
            });
        }
        drop(running);

        let job = Job::new(request);
        let id = job.id.clone();
        self.jobs.write().await.insert(id.clone(), job.clone());
        self.subscribers.write().await.insert(id, Vec::new());
        Ok(job)
    }

    /// Get a job by ID.
    pub async fn get(&self, job_id: &str) -> Option<Job> {
        self.jobs.read().await.get(job_id).cloned()
    }

    /// List all jobs, optionally filtered by status.
    pub async fn list(&self, status_filter: Option<&JobStatus>) -> Vec<Job> {
        let jobs = self.jobs.read().await;
        let mut result: Vec<Job> = if let Some(status) = status_filter {
            jobs.values().filter(|j| &j.status == status).cloned().collect()
        } else {
            jobs.values().cloned().collect()
        };
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        result
    }

    /// Mark a job as running.
    pub async fn mark_running(&self, job_id: &str) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(job_id).ok_or(JobError::NotFound)?;
        job.start();
        let mut running = self.running_count.lock().await;
        *running += 1;
        Ok(())
    }

    /// Mark a job as completed.
    pub async fn mark_completed(&self, job_id: &str, output: String, turns: u32) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(job_id).ok_or(JobError::NotFound)?;
        job.complete(output, turns);
        let mut running = self.running_count.lock().await;
        *running = running.saturating_sub(1);
        Ok(())
    }

    /// Mark a job as failed.
    pub async fn mark_failed(&self, job_id: &str, error: String) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(job_id).ok_or(JobError::NotFound)?;
        job.fail(error);
        let mut running = self.running_count.lock().await;
        *running = running.saturating_sub(1);
        Ok(())
    }

    /// Cancel a job.
    pub async fn cancel(&self, job_id: &str) -> Result<(), JobError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(job_id).ok_or(JobError::NotFound)?;
        if job.status == JobStatus::Running || job.status == JobStatus::Queued {
            job.cancel();
            if job.status == JobStatus::Running {
                let mut running = self.running_count.lock().await;
                *running = running.saturating_sub(1);
            }
            Ok(())
        } else {
            Err(JobError::InvalidTransition {
                from: format!("{:?}", job.status),
                to: "cancelled".into(),
            })
        }
    }

    /// Subscribe to streaming events for a job.
    pub async fn subscribe(&self, job_id: &str, buffer: usize) -> Result<mpsc::Receiver<StreamEvent>, JobError> {
        let mut subs = self.subscribers.write().await;
        let job_subs = subs.get_mut(job_id).ok_or(JobError::NotFound)?;
        let (tx, rx) = mpsc::channel(buffer);
        job_subs.push(tx);
        Ok(rx)
    }

    /// Broadcast an event to all subscribers of a job.
    pub async fn broadcast(&self, job_id: &str, event: StreamEvent) {
        let mut subs = self.subscribers.write().await;
        if let Some(job_subs) = subs.get_mut(job_id) {
            // Remove closed channels
            job_subs.retain(|tx| !tx.is_closed());
            for tx in job_subs.iter() {
                let _ = tx.try_send(event.clone());
            }
        }
    }

    /// Get the count of running jobs.
    pub async fn running_count(&self) -> usize {
        *self.running_count.lock().await
    }

    /// Get total job count.
    pub async fn total_count(&self) -> usize {
        self.jobs.read().await.len()
    }
}

/// Job management errors.
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("job not found")]
    NotFound,
    #[error("job queue full (max {max} concurrent jobs)")]
    QueueFull { max: usize },
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_creation() {
        let req = JobRequest {
            prompt: "Fix the bug".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        };
        let job = Job::new(req);
        assert_eq!(job.status, JobStatus::Queued);
        assert!(!job.session_id.is_empty());
        assert!(job.started_at.is_none());
        assert!(job.completed_at.is_none());
    }

    #[test]
    fn test_job_lifecycle() {
        let req = JobRequest {
            prompt: "task".into(),
            session_id: Some("sess-1".into()),
            model: Some("claude-sonnet-4-6".into()),
            max_turns: Some(5),
            cwd: None,
        };
        let mut job = Job::new(req);
        assert_eq!(job.session_id, "sess-1");

        job.start();
        assert_eq!(job.status, JobStatus::Running);
        assert!(job.started_at.is_some());

        job.complete("Done!".into(), 3);
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.output.as_deref(), Some("Done!"));
        assert_eq!(job.turns, 3);
        assert!(job.completed_at.is_some());
    }

    #[test]
    fn test_job_failure() {
        let mut job = Job::new(JobRequest {
            prompt: "fail".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        });
        job.start();
        job.fail("out of tokens".into());
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("out of tokens"));
    }

    #[test]
    fn test_job_cancel() {
        let mut job = Job::new(JobRequest {
            prompt: "cancel me".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        });
        job.cancel();
        assert_eq!(job.status, JobStatus::Cancelled);
    }

    #[test]
    fn test_job_duration() {
        let mut job = Job::new(JobRequest {
            prompt: "timing".into(),
            session_id: None,
            model: None,
            max_turns: None,
            cwd: None,
        });
        assert!(job.duration_secs().is_none());
        job.start();
        // Duration should be Some now (running)
        assert!(job.duration_secs().is_some());
    }

    #[tokio::test]
    async fn test_job_manager_submit() {
        let mgr = JobManager::new(10);
        let job = mgr
            .submit(JobRequest {
                prompt: "hello".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();

        assert_eq!(mgr.total_count().await, 1);
        let fetched = mgr.get(&job.id).await.unwrap();
        assert_eq!(fetched.request.prompt, "hello");
    }

    #[tokio::test]
    async fn test_job_manager_queue_full() {
        let mgr = JobManager::new(1);

        let job1 = mgr
            .submit(JobRequest {
                prompt: "a".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();
        mgr.mark_running(&job1.id).await.unwrap();

        let result = mgr
            .submit(JobRequest {
                prompt: "b".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_job_manager_lifecycle() {
        let mgr = JobManager::new(5);
        let job = mgr
            .submit(JobRequest {
                prompt: "test".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();

        mgr.mark_running(&job.id).await.unwrap();
        assert_eq!(mgr.running_count().await, 1);

        mgr.mark_completed(&job.id, "output".into(), 2).await.unwrap();
        assert_eq!(mgr.running_count().await, 0);

        let completed = mgr.get(&job.id).await.unwrap();
        assert_eq!(completed.status, JobStatus::Completed);
    }

    #[tokio::test]
    async fn test_job_manager_list_filter() {
        let mgr = JobManager::new(10);

        let j1 = mgr
            .submit(JobRequest {
                prompt: "a".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();
        let _j2 = mgr
            .submit(JobRequest {
                prompt: "b".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();

        mgr.mark_running(&j1.id).await.unwrap();

        let all = mgr.list(None).await;
        assert_eq!(all.len(), 2);

        let running = mgr.list(Some(&JobStatus::Running)).await;
        assert_eq!(running.len(), 1);

        let queued = mgr.list(Some(&JobStatus::Queued)).await;
        assert_eq!(queued.len(), 1);
    }

    #[tokio::test]
    async fn test_job_manager_subscribe_broadcast() {
        let mgr = JobManager::new(5);
        let job = mgr
            .submit(JobRequest {
                prompt: "stream".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();

        let mut rx = mgr.subscribe(&job.id, 16).await.unwrap();

        let event = StreamEvent::TextDelta {
            text: "hello".into(),
        };
        mgr.broadcast(&job.id, event).await;

        let received = rx.recv().await.unwrap();
        assert!(matches!(received, StreamEvent::TextDelta { ref text } if text == "hello"));
    }

    #[tokio::test]
    async fn test_job_manager_cancel() {
        let mgr = JobManager::new(5);
        let job = mgr
            .submit(JobRequest {
                prompt: "cancel".into(),
                session_id: None,
                model: None,
                max_turns: None,
                cwd: None,
            })
            .await
            .unwrap();

        mgr.cancel(&job.id).await.unwrap();
        let cancelled = mgr.get(&job.id).await.unwrap();
        assert_eq!(cancelled.status, JobStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_job_not_found() {
        let mgr = JobManager::new(5);
        assert!(mgr.get("nonexistent").await.is_none());
        assert!(mgr.mark_running("nonexistent").await.is_err());
    }
}
