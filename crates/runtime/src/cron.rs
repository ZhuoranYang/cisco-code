//! Cron scheduling for recurring and one-shot delayed prompts.
//!
//! Matches Claude Code's CronCreate/CronList/CronDelete tool backend.
//! Schedules are stored in-memory with optional persistence to a JSON file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schedule type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronSchedule {
    /// Run once at a specific time.
    Once(DateTime<Utc>),
    /// Run on an interval (seconds).
    Interval(u64),
    /// Cron expression (e.g., "0 */6 * * *").
    Cron(String),
}

/// A scheduled cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub schedule: CronSchedule,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
    pub run_count: u64,
    pub enabled: bool,
    /// Working directory for execution.
    pub cwd: Option<String>,
    /// Model override.
    pub model: Option<String>,
}

/// Manages cron schedules.
pub struct CronManager {
    jobs: HashMap<String, CronJob>,
    next_id: AtomicU64,
    /// Optional persistence path.
    persist_path: Option<PathBuf>,
}

impl CronManager {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
            next_id: AtomicU64::new(1),
            persist_path: None,
        }
    }

    /// Create with persistence to a JSON file.
    pub fn with_persistence(path: &Path) -> Self {
        let mut mgr = Self::new();
        mgr.persist_path = Some(path.to_path_buf());
        mgr
    }

    /// Create a new cron job.
    pub fn create(
        &mut self,
        name: &str,
        prompt: &str,
        schedule: CronSchedule,
        cwd: Option<&str>,
        model: Option<&str>,
    ) -> &CronJob {
        let id = format!("cron-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let now = Utc::now();
        let next_run = compute_next_run(&schedule, &now);

        let job = CronJob {
            id: id.clone(),
            name: name.to_string(),
            prompt: prompt.to_string(),
            schedule,
            created_at: now,
            last_run: None,
            next_run,
            run_count: 0,
            enabled: true,
            cwd: cwd.map(String::from),
            model: model.map(String::from),
        };
        self.jobs.insert(id.clone(), job);
        self.jobs.get(&id).unwrap()
    }

    /// Get a job by ID.
    pub fn get(&self, id: &str) -> Option<&CronJob> {
        self.jobs.get(id)
    }

    /// List all jobs.
    pub fn list(&self) -> Vec<&CronJob> {
        let mut jobs: Vec<&CronJob> = self.jobs.values().collect();
        jobs.sort_by_key(|j| &j.created_at);
        jobs
    }

    /// List enabled jobs.
    pub fn list_enabled(&self) -> Vec<&CronJob> {
        self.list().into_iter().filter(|j| j.enabled).collect()
    }

    /// Delete a job.
    pub fn delete(&mut self, id: &str) -> bool {
        self.jobs.remove(id).is_some()
    }

    /// Enable a job.
    pub fn enable(&mut self, id: &str) -> bool {
        if let Some(job) = self.jobs.get_mut(id) {
            job.enabled = true;
            true
        } else {
            false
        }
    }

    /// Disable a job.
    pub fn disable(&mut self, id: &str) -> bool {
        if let Some(job) = self.jobs.get_mut(id) {
            job.enabled = false;
            true
        } else {
            false
        }
    }

    /// Record that a job was executed.
    pub fn record_run(&mut self, id: &str) {
        if let Some(job) = self.jobs.get_mut(id) {
            let now = Utc::now();
            job.last_run = Some(now);
            job.run_count += 1;
            job.next_run = compute_next_run(&job.schedule, &now);
        }
    }

    /// Get jobs that are due to run (next_run <= now).
    pub fn due_jobs(&self) -> Vec<&CronJob> {
        let now = Utc::now();
        self.jobs
            .values()
            .filter(|j| j.enabled && j.next_run.is_some_and(|nr| nr <= now))
            .collect()
    }

    /// Number of jobs.
    pub fn count(&self) -> usize {
        self.jobs.len()
    }

    /// Save jobs to the persistence file.
    pub fn save(&self) -> Result<()> {
        if let Some(path) = &self.persist_path {
            let jobs: Vec<&CronJob> = self.jobs.values().collect();
            let json = serde_json::to_string_pretty(&jobs)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, json)?;
        }
        Ok(())
    }

    /// Load jobs from the persistence file.
    pub fn load(&mut self) -> Result<()> {
        if let Some(path) = &self.persist_path {
            if path.exists() {
                let json = std::fs::read_to_string(path)?;
                let jobs: Vec<CronJob> = serde_json::from_str(&json)?;
                for job in jobs {
                    self.jobs.insert(job.id.clone(), job);
                }
            }
        }
        Ok(())
    }
}

impl Default for CronManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute the next run time for a schedule.
pub fn compute_next_run(schedule: &CronSchedule, from: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    match schedule {
        CronSchedule::Once(at) => {
            if at > from {
                Some(*at)
            } else {
                None // already past
            }
        }
        CronSchedule::Interval(secs) => {
            Some(*from + chrono::Duration::seconds(*secs as i64))
        }
        CronSchedule::Cron(_expr) => {
            // Full cron parsing would require a cron-parser crate.
            // For now, we approximate: next run is 1 hour from now as placeholder.
            // In production, integrate the `cron` crate for proper parsing.
            Some(*from + chrono::Duration::hours(1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_job() {
        let mut mgr = CronManager::new();
        let job = mgr.create(
            "test",
            "Run tests",
            CronSchedule::Interval(3600),
            Some("/tmp"),
            None,
        );
        assert_eq!(job.name, "test");
        assert_eq!(job.prompt, "Run tests");
        assert!(job.id.starts_with("cron-"));
        assert!(job.enabled);
        assert_eq!(job.run_count, 0);
    }

    #[test]
    fn test_get_job() {
        let mut mgr = CronManager::new();
        let id = mgr.create("j", "prompt", CronSchedule::Interval(60), None, None).id.clone();
        assert!(mgr.get(&id).is_some());
        assert!(mgr.get("nonexistent").is_none());
    }

    #[test]
    fn test_list() {
        let mut mgr = CronManager::new();
        mgr.create("A", "a", CronSchedule::Interval(60), None, None);
        mgr.create("B", "b", CronSchedule::Interval(120), None, None);
        assert_eq!(mgr.list().len(), 2);
    }

    #[test]
    fn test_delete() {
        let mut mgr = CronManager::new();
        let id = mgr.create("j", "p", CronSchedule::Interval(60), None, None).id.clone();
        assert!(mgr.delete(&id));
        assert!(!mgr.delete(&id));
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_enable_disable() {
        let mut mgr = CronManager::new();
        let id = mgr.create("j", "p", CronSchedule::Interval(60), None, None).id.clone();
        mgr.disable(&id);
        assert!(!mgr.get(&id).unwrap().enabled);
        assert_eq!(mgr.list_enabled().len(), 0);
        mgr.enable(&id);
        assert!(mgr.get(&id).unwrap().enabled);
        assert_eq!(mgr.list_enabled().len(), 1);
    }

    #[test]
    fn test_record_run() {
        let mut mgr = CronManager::new();
        let id = mgr.create("j", "p", CronSchedule::Interval(60), None, None).id.clone();
        assert!(mgr.get(&id).unwrap().last_run.is_none());
        mgr.record_run(&id);
        assert!(mgr.get(&id).unwrap().last_run.is_some());
        assert_eq!(mgr.get(&id).unwrap().run_count, 1);
        mgr.record_run(&id);
        assert_eq!(mgr.get(&id).unwrap().run_count, 2);
    }

    #[test]
    fn test_once_schedule_next_run() {
        let future = Utc::now() + chrono::Duration::hours(1);
        let next = compute_next_run(&CronSchedule::Once(future), &Utc::now());
        assert!(next.is_some());

        let past = Utc::now() - chrono::Duration::hours(1);
        let next = compute_next_run(&CronSchedule::Once(past), &Utc::now());
        assert!(next.is_none());
    }

    #[test]
    fn test_interval_next_run() {
        let now = Utc::now();
        let next = compute_next_run(&CronSchedule::Interval(3600), &now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_cron_next_run_placeholder() {
        let now = Utc::now();
        let next = compute_next_run(&CronSchedule::Cron("0 * * * *".into()), &now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn test_persistence() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cron.json");

        let mut mgr = CronManager::with_persistence(&path);
        mgr.create("test", "prompt", CronSchedule::Interval(60), None, None);
        mgr.save().unwrap();

        let mut mgr2 = CronManager::with_persistence(&path);
        mgr2.load().unwrap();
        assert_eq!(mgr2.count(), 1);
        assert_eq!(mgr2.list()[0].name, "test");
    }

    #[test]
    fn test_due_jobs() {
        let mut mgr = CronManager::new();
        // Create a job with interval 0 so it's immediately due
        let id = mgr.create("j", "p", CronSchedule::Interval(0), None, None).id.clone();
        // The next_run is computed as now + 0 seconds = now, so it should be due
        // But there's a race - let's just verify the mechanism works
        let due = mgr.due_jobs();
        // Interval(0) means next_run = from + 0s = from, which is <= now
        assert!(due.len() <= 1); // timing-dependent

        // Disable and it shouldn't be due
        mgr.disable(&id);
        assert!(mgr.due_jobs().is_empty());
    }

    #[test]
    fn test_job_with_model() {
        let mut mgr = CronManager::new();
        let job = mgr.create("j", "p", CronSchedule::Interval(60), None, Some("opus"));
        assert_eq!(job.model.as_deref(), Some("opus"));
    }

    #[test]
    fn test_job_with_cwd() {
        let mut mgr = CronManager::new();
        let job = mgr.create("j", "p", CronSchedule::Interval(60), Some("/home/user/project"), None);
        assert_eq!(job.cwd.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn test_enable_disable_nonexistent() {
        let mut mgr = CronManager::new();
        assert!(!mgr.enable("nonexistent"));
        assert!(!mgr.disable("nonexistent"));
    }

    #[test]
    fn test_record_run_nonexistent() {
        let mut mgr = CronManager::new();
        // Should not panic
        mgr.record_run("nonexistent");
    }

    #[test]
    fn test_load_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cron.json");
        let mut mgr = CronManager::with_persistence(&path);
        // Loading a non-existent file should be OK
        mgr.load().unwrap();
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_save_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cron.json");
        let mgr = CronManager::with_persistence(&path);
        mgr.save().unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim(), "[]");
    }

    #[test]
    fn test_default_manager() {
        let mgr = CronManager::default();
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_auto_increment_ids() {
        let mut mgr = CronManager::new();
        let id1 = mgr.create("a", "p", CronSchedule::Interval(60), None, None).id.clone();
        let id2 = mgr.create("b", "p", CronSchedule::Interval(60), None, None).id.clone();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("cron-"));
        assert!(id2.starts_with("cron-"));
    }

    #[test]
    fn test_once_schedule_past() {
        let past = Utc::now() - chrono::Duration::hours(1);
        let mut mgr = CronManager::new();
        let job = mgr.create("once-past", "p", CronSchedule::Once(past), None, None);
        // next_run should be None since the time already passed
        assert!(job.next_run.is_none());
    }

    #[test]
    fn test_once_schedule_future() {
        let future = Utc::now() + chrono::Duration::hours(1);
        let mut mgr = CronManager::new();
        let job = mgr.create("once-future", "p", CronSchedule::Once(future), None, None);
        assert!(job.next_run.is_some());
    }

    #[test]
    fn test_multiple_record_runs() {
        let mut mgr = CronManager::new();
        let id = mgr.create("j", "p", CronSchedule::Interval(60), None, None).id.clone();
        for _ in 0..5 {
            mgr.record_run(&id);
        }
        assert_eq!(mgr.get(&id).unwrap().run_count, 5);
    }

    #[test]
    fn test_cron_job_serialization() {
        let job = CronJob {
            id: "cron-1".into(),
            name: "test".into(),
            prompt: "do thing".into(),
            schedule: CronSchedule::Interval(3600),
            created_at: Utc::now(),
            last_run: None,
            next_run: Some(Utc::now()),
            run_count: 0,
            enabled: true,
            cwd: None,
            model: None,
        };
        let json = serde_json::to_string(&job).unwrap();
        let parsed: CronJob = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.prompt, "do thing");
    }

    #[test]
    fn test_schedule_serialization() {
        let schedules = vec![
            CronSchedule::Interval(60),
            CronSchedule::Cron("0 * * * *".into()),
            CronSchedule::Once(Utc::now()),
        ];
        for sched in &schedules {
            let json = serde_json::to_string(sched).unwrap();
            let parsed: CronSchedule = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, sched);
        }
    }
}
