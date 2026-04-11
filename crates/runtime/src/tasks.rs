//! Task management system for structured work tracking.
//!
//! Matches Claude Code's TaskCreate/TaskUpdate/TaskList/TaskGet tool backend.
//! Tasks are in-memory work items with status tracking, descriptions,
//! and ordering for the current session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Task status lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Not yet started.
    Pending,
    /// Currently being worked on.
    InProgress,
    /// Completed successfully.
    Completed,
    /// Cancelled or abandoned.
    Cancelled,
    /// Blocked by another task.
    Blocked,
}

/// A single tracked task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Optional output/notes from completion.
    pub output: Option<String>,
    /// Task ordering (lower = higher priority).
    pub order: u64,
}

/// In-memory task manager for a session.
pub struct TaskManager {
    tasks: HashMap<u64, Task>,
    next_id: AtomicU64,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new task.
    pub fn create(&mut self, description: &str) -> &Task {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let now = Utc::now();
        let order = id; // default order matches creation order
        let task = Task {
            id,
            description: description.to_string(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            completed_at: None,
            output: None,
            order,
        };
        self.tasks.insert(id, task);
        self.tasks.get(&id).unwrap()
    }

    /// Get a task by ID.
    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Update task status.
    pub fn update_status(&mut self, id: u64, status: TaskStatus) -> Option<&Task> {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.status = status.clone();
            task.updated_at = Utc::now();
            if status == TaskStatus::Completed {
                task.completed_at = Some(Utc::now());
            }
            Some(task)
        } else {
            None
        }
    }

    /// Update task output/notes.
    pub fn set_output(&mut self, id: u64, output: &str) -> Option<&Task> {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.output = Some(output.to_string());
            task.updated_at = Utc::now();
            Some(task)
        } else {
            None
        }
    }

    /// List all tasks, ordered by their order field.
    pub fn list(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.values().collect();
        tasks.sort_by_key(|t| t.order);
        tasks
    }

    /// List tasks filtered by status.
    pub fn list_by_status(&self, status: &TaskStatus) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.values().filter(|t| &t.status == status).collect();
        tasks.sort_by_key(|t| t.order);
        tasks
    }

    /// Remove a task.
    pub fn remove(&mut self, id: u64) -> bool {
        self.tasks.remove(&id).is_some()
    }

    /// Number of tasks.
    pub fn count(&self) -> usize {
        self.tasks.len()
    }

    /// Mark a task as in-progress.
    pub fn start(&mut self, id: u64) -> Option<&Task> {
        self.update_status(id, TaskStatus::InProgress)
    }

    /// Mark a task as completed with optional output.
    pub fn complete(&mut self, id: u64, output: Option<&str>) -> Option<&Task> {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.status = TaskStatus::Completed;
            task.updated_at = Utc::now();
            task.completed_at = Some(Utc::now());
            if let Some(out) = output {
                task.output = Some(out.to_string());
            }
            Some(task)
        } else {
            None
        }
    }

    /// Cancel a task.
    pub fn cancel(&mut self, id: u64) -> Option<&Task> {
        self.update_status(id, TaskStatus::Cancelled)
    }

    /// Summary counts by status.
    pub fn summary(&self) -> TaskSummary {
        let mut summary = TaskSummary::default();
        for task in self.tasks.values() {
            match task.status {
                TaskStatus::Pending => summary.pending += 1,
                TaskStatus::InProgress => summary.in_progress += 1,
                TaskStatus::Completed => summary.completed += 1,
                TaskStatus::Cancelled => summary.cancelled += 1,
                TaskStatus::Blocked => summary.blocked += 1,
            }
        }
        summary.total = self.tasks.len();
        summary
    }

    /// Render tasks as a markdown checklist.
    pub fn render_markdown(&self) -> String {
        let mut md = String::new();
        for task in self.list() {
            let check = match task.status {
                TaskStatus::Completed => "[x]",
                TaskStatus::Cancelled => "[-]",
                _ => "[ ]",
            };
            let status_label = match task.status {
                TaskStatus::Pending => "",
                TaskStatus::InProgress => " (in progress)",
                TaskStatus::Blocked => " (blocked)",
                TaskStatus::Completed | TaskStatus::Cancelled => "",
            };
            md.push_str(&format!(
                "- {} #{}: {}{}\n",
                check, task.id, task.description, status_label
            ));
        }
        md
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary counts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    pub total: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub cancelled: usize,
    pub blocked: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_task() {
        let mut mgr = TaskManager::new();
        let task = mgr.create("Implement feature X");
        assert_eq!(task.id, 1);
        assert_eq!(task.description, "Implement feature X");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.completed_at.is_none());
    }

    #[test]
    fn test_get_task() {
        let mut mgr = TaskManager::new();
        mgr.create("Task A");
        assert!(mgr.get(1).is_some());
        assert!(mgr.get(99).is_none());
    }

    #[test]
    fn test_auto_increment_ids() {
        let mut mgr = TaskManager::new();
        let t1 = mgr.create("First").id;
        let t2 = mgr.create("Second").id;
        let t3 = mgr.create("Third").id;
        assert_eq!(t1, 1);
        assert_eq!(t2, 2);
        assert_eq!(t3, 3);
    }

    #[test]
    fn test_update_status() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        mgr.update_status(1, TaskStatus::InProgress);
        assert_eq!(mgr.get(1).unwrap().status, TaskStatus::InProgress);
    }

    #[test]
    fn test_complete_with_output() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        mgr.complete(1, Some("Done successfully"));
        let task = mgr.get(1).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.completed_at.is_some());
        assert_eq!(task.output.as_deref(), Some("Done successfully"));
    }

    #[test]
    fn test_cancel() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        mgr.cancel(1);
        assert_eq!(mgr.get(1).unwrap().status, TaskStatus::Cancelled);
    }

    #[test]
    fn test_list_ordered() {
        let mut mgr = TaskManager::new();
        mgr.create("A");
        mgr.create("B");
        mgr.create("C");
        let tasks = mgr.list();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].description, "A");
        assert_eq!(tasks[2].description, "C");
    }

    #[test]
    fn test_list_by_status() {
        let mut mgr = TaskManager::new();
        mgr.create("Pending 1");
        mgr.create("Pending 2");
        mgr.create("Will complete");
        mgr.complete(3, None);
        assert_eq!(mgr.list_by_status(&TaskStatus::Pending).len(), 2);
        assert_eq!(mgr.list_by_status(&TaskStatus::Completed).len(), 1);
        assert_eq!(mgr.list_by_status(&TaskStatus::InProgress).len(), 0);
    }

    #[test]
    fn test_remove() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        assert!(mgr.remove(1));
        assert!(!mgr.remove(1));
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_summary() {
        let mut mgr = TaskManager::new();
        mgr.create("A");
        mgr.create("B");
        mgr.create("C");
        mgr.start(1);
        mgr.complete(2, None);
        mgr.cancel(3);
        let summary = mgr.summary();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.cancelled, 1);
        assert_eq!(summary.pending, 0);
    }

    #[test]
    fn test_render_markdown() {
        let mut mgr = TaskManager::new();
        mgr.create("First task");
        mgr.create("Second task");
        mgr.complete(1, None);
        mgr.start(2);
        let md = mgr.render_markdown();
        assert!(md.contains("[x] #1: First task"));
        assert!(md.contains("[ ] #2: Second task (in progress)"));
    }

    #[test]
    fn test_set_output() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        mgr.set_output(1, "Some notes");
        assert_eq!(mgr.get(1).unwrap().output.as_deref(), Some("Some notes"));
    }

    #[test]
    fn test_update_nonexistent() {
        let mut mgr = TaskManager::new();
        assert!(mgr.update_status(99, TaskStatus::Completed).is_none());
        assert!(mgr.complete(99, None).is_none());
        assert!(mgr.set_output(99, "x").is_none());
    }

    #[test]
    fn test_task_lifecycle() {
        let mut mgr = TaskManager::new();
        let id = mgr.create("Build feature").id;
        assert_eq!(mgr.get(id).unwrap().status, TaskStatus::Pending);

        mgr.start(id);
        assert_eq!(mgr.get(id).unwrap().status, TaskStatus::InProgress);

        mgr.complete(id, Some("Feature built and tested"));
        let task = mgr.get(id).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.completed_at.is_some());
        assert_eq!(task.output.as_deref(), Some("Feature built and tested"));
    }

    #[test]
    fn test_blocked_status() {
        let mut mgr = TaskManager::new();
        mgr.create("Blocked task");
        mgr.update_status(1, TaskStatus::Blocked);
        assert_eq!(mgr.get(1).unwrap().status, TaskStatus::Blocked);
        assert_eq!(mgr.list_by_status(&TaskStatus::Blocked).len(), 1);
    }

    #[test]
    fn test_updated_at_changes() {
        let mut mgr = TaskManager::new();
        let created_at = mgr.create("Task").updated_at;
        // After update, updated_at should be >= created_at
        mgr.start(1);
        let updated_at = mgr.get(1).unwrap().updated_at;
        assert!(updated_at >= created_at);
    }

    #[test]
    fn test_render_markdown_cancelled() {
        let mut mgr = TaskManager::new();
        mgr.create("Cancelled task");
        mgr.cancel(1);
        let md = mgr.render_markdown();
        assert!(md.contains("[-] #1: Cancelled task"));
    }

    #[test]
    fn test_render_markdown_blocked() {
        let mut mgr = TaskManager::new();
        mgr.create("Blocked task");
        mgr.update_status(1, TaskStatus::Blocked);
        let md = mgr.render_markdown();
        assert!(md.contains("[ ] #1: Blocked task (blocked)"));
    }

    #[test]
    fn test_summary_empty() {
        let mgr = TaskManager::new();
        let summary = mgr.summary();
        assert_eq!(summary.total, 0);
        assert_eq!(summary.pending, 0);
    }

    #[test]
    fn test_remove_and_recount() {
        let mut mgr = TaskManager::new();
        mgr.create("A");
        mgr.create("B");
        mgr.create("C");
        assert_eq!(mgr.count(), 3);
        mgr.remove(2);
        assert_eq!(mgr.count(), 2);
        assert!(mgr.get(2).is_none());
        assert!(mgr.get(1).is_some());
        assert!(mgr.get(3).is_some());
    }

    #[test]
    fn test_complete_without_output() {
        let mut mgr = TaskManager::new();
        mgr.create("Task");
        mgr.complete(1, None);
        let task = mgr.get(1).unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.output.is_none());
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn test_default_task_manager() {
        let mgr = TaskManager::default();
        assert_eq!(mgr.count(), 0);
    }

    #[test]
    fn test_task_serialization() {
        let mut mgr = TaskManager::new();
        mgr.create("Serialize me");
        let task = mgr.get(1).unwrap();
        let json = serde_json::to_string(task).unwrap();
        let parsed: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.description, "Serialize me");
        assert_eq!(parsed.status, TaskStatus::Pending);
    }

    #[test]
    fn test_summary_serialization() {
        let summary = TaskSummary {
            total: 5,
            pending: 2,
            in_progress: 1,
            completed: 1,
            cancelled: 1,
            blocked: 0,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: TaskSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total, 5);
        assert_eq!(parsed.in_progress, 1);
    }
}
