//! Git worktree manager for agent isolation.
//!
//! Matches Claude Code's Agent tool worktree isolation feature.
//! Creates temporary git worktrees so subagents can work on an isolated
//! copy of the repository without affecting the main working tree.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Information about a created worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    /// Path to the worktree directory.
    pub path: PathBuf,
    /// Branch name created for this worktree.
    pub branch: String,
    /// The base branch/commit it was created from.
    pub base_ref: String,
    /// Whether the worktree has been cleaned up.
    pub cleaned_up: bool,
}

/// Manages git worktrees for isolated agent execution.
pub struct WorktreeManager {
    /// Root of the main git repository.
    repo_root: PathBuf,
    /// Directory where worktrees are created.
    worktree_base: PathBuf,
    /// Active worktrees.
    active: Vec<WorktreeInfo>,
}

impl WorktreeManager {
    /// Create a new WorktreeManager for the given repository.
    pub fn new(repo_root: &Path) -> Self {
        let worktree_base = repo_root.join(".git").join("cisco-code-worktrees");
        Self {
            repo_root: repo_root.to_path_buf(),
            worktree_base,
            active: Vec::new(),
        }
    }

    /// Create a new worktree with a unique branch.
    pub fn create(&mut self, agent_id: &str) -> Result<WorktreeInfo> {
        let branch = format!("cisco-code/agent-{}", agent_id);
        let worktree_path = self.worktree_base.join(agent_id);

        // Build the git worktree add command
        let output = std::process::Command::new("git")
            .args(["worktree", "add", "-b", &branch])
            .arg(&worktree_path)
            .arg("HEAD")
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to create worktree: {}", stderr.trim());
        }

        let base_ref = self.current_ref()?;
        let info = WorktreeInfo {
            path: worktree_path,
            branch,
            base_ref,
            cleaned_up: false,
        };
        self.active.push(info.clone());
        Ok(info)
    }

    /// Remove a worktree and optionally delete its branch.
    pub fn remove(&mut self, agent_id: &str, delete_branch: bool) -> Result<bool> {
        let worktree_path = self.worktree_base.join(agent_id);

        if !worktree_path.exists() {
            return Ok(false);
        }

        // Find the branch name before removing
        let branch = self
            .active
            .iter()
            .find(|w| w.path == worktree_path)
            .map(|w| w.branch.clone());

        // Remove the worktree
        let output = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree_path)
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Failed to remove worktree: {}", stderr.trim());
        }

        // Optionally delete the branch
        if delete_branch {
            if let Some(branch) = branch {
                let _ = std::process::Command::new("git")
                    .args(["branch", "-D", &branch])
                    .current_dir(&self.repo_root)
                    .output();
            }
        }

        // Mark as cleaned up
        for wt in &mut self.active {
            if wt.path == worktree_path {
                wt.cleaned_up = true;
            }
        }

        Ok(true)
    }

    /// Check if a worktree has changes.
    pub fn has_changes(worktree_path: &Path) -> Result<bool> {
        let output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(worktree_path)
            .output()?;

        Ok(!output.stdout.is_empty())
    }

    /// Clean up a worktree only if it has no changes.
    pub fn cleanup_if_clean(&mut self, agent_id: &str) -> Result<CleanupResult> {
        let worktree_path = self.worktree_base.join(agent_id);

        if !worktree_path.exists() {
            return Ok(CleanupResult::NotFound);
        }

        if Self::has_changes(&worktree_path)? {
            let info = self
                .active
                .iter()
                .find(|w| w.path == worktree_path)
                .cloned();
            return Ok(CleanupResult::HasChanges(info));
        }

        self.remove(agent_id, true)?;
        Ok(CleanupResult::Cleaned)
    }

    /// List active worktrees.
    pub fn list_active(&self) -> Vec<&WorktreeInfo> {
        self.active.iter().filter(|w| !w.cleaned_up).collect()
    }

    /// Get repo root.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Get the current HEAD ref.
    fn current_ref(&self) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&self.repo_root)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Ok("HEAD".to_string())
        }
    }

    /// Generate the branch name for an agent.
    pub fn branch_name(agent_id: &str) -> String {
        format!("cisco-code/agent-{}", agent_id)
    }

    /// Generate the worktree path for an agent.
    pub fn worktree_path(&self, agent_id: &str) -> PathBuf {
        self.worktree_base.join(agent_id)
    }
}

/// Result of a cleanup attempt.
#[derive(Debug, Clone)]
pub enum CleanupResult {
    /// Worktree was clean and removed.
    Cleaned,
    /// Worktree has uncommitted changes (returned with info).
    HasChanges(Option<WorktreeInfo>),
    /// Worktree not found.
    NotFound,
}

/// Parse a `git worktree list --porcelain` output.
pub fn parse_worktree_list(output: &str) -> Vec<WorktreeListEntry> {
    let mut entries = Vec::new();
    let mut current = WorktreeListEntry::default();
    let mut has_data = false;

    for line in output.lines() {
        if line.is_empty() {
            if has_data {
                entries.push(current);
                current = WorktreeListEntry::default();
                has_data = false;
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current.path = PathBuf::from(path);
            has_data = true;
        } else if let Some(head) = line.strip_prefix("HEAD ") {
            current.head = head.to_string();
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current.branch = Some(branch.to_string());
        } else if line == "bare" {
            current.bare = true;
        } else if line == "detached" {
            current.detached = true;
        }
    }

    if has_data {
        entries.push(current);
    }

    entries
}

/// A parsed entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, Default)]
pub struct WorktreeListEntry {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branch_name() {
        assert_eq!(
            WorktreeManager::branch_name("abc123"),
            "cisco-code/agent-abc123"
        );
    }

    #[test]
    fn test_worktree_path() {
        let mgr = WorktreeManager::new(Path::new("/tmp/repo"));
        let path = mgr.worktree_path("test-agent");
        assert!(path.to_string_lossy().contains("cisco-code-worktrees"));
        assert!(path.to_string_lossy().contains("test-agent"));
    }

    #[test]
    fn test_new_manager() {
        let mgr = WorktreeManager::new(Path::new("/tmp/repo"));
        assert_eq!(mgr.repo_root(), Path::new("/tmp/repo"));
        assert!(mgr.list_active().is_empty());
    }

    #[test]
    fn test_parse_worktree_list() {
        let output = "worktree /home/user/project\nHEAD abc1234\nbranch refs/heads/main\n\nworktree /home/user/project/.git/worktrees/agent-1\nHEAD def5678\nbranch refs/heads/cisco-code/agent-1\n\n";

        let entries = parse_worktree_list(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, PathBuf::from("/home/user/project"));
        assert_eq!(entries[0].head, "abc1234");
        assert_eq!(
            entries[0].branch.as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            entries[1].branch.as_deref(),
            Some("refs/heads/cisco-code/agent-1")
        );
    }

    #[test]
    fn test_parse_worktree_list_detached() {
        let output = "worktree /tmp/detached\nHEAD abc1234\ndetached\n\n";
        let entries = parse_worktree_list(output);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].detached);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn test_parse_worktree_list_bare() {
        let output = "worktree /tmp/bare.git\nHEAD abc1234\nbare\n\n";
        let entries = parse_worktree_list(output);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].bare);
    }

    #[test]
    fn test_parse_empty_output() {
        let entries = parse_worktree_list("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_worktree_info_serialization() {
        let info = WorktreeInfo {
            path: PathBuf::from("/tmp/worktree"),
            branch: "cisco-code/agent-test".into(),
            base_ref: "abc1234".into(),
            cleaned_up: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: WorktreeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.branch, "cisco-code/agent-test");
        assert!(!parsed.cleaned_up);
    }

    #[test]
    fn test_worktree_info_cleaned_up() {
        let info = WorktreeInfo {
            path: PathBuf::from("/tmp/worktree"),
            branch: "cisco-code/agent-done".into(),
            base_ref: "abc1234".into(),
            cleaned_up: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: WorktreeInfo = serde_json::from_str(&json).unwrap();
        assert!(parsed.cleaned_up);
    }

    #[test]
    fn test_parse_worktree_list_multiple() {
        let output = "\
worktree /home/user/project
HEAD abc1234
branch refs/heads/main

worktree /home/user/project/.git/worktrees/a1
HEAD def5678
branch refs/heads/cisco-code/agent-a1

worktree /home/user/project/.git/worktrees/a2
HEAD 9876543
branch refs/heads/cisco-code/agent-a2

";
        let entries = parse_worktree_list(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(entries[1].branch.as_deref(), Some("refs/heads/cisco-code/agent-a1"));
        assert_eq!(entries[2].branch.as_deref(), Some("refs/heads/cisco-code/agent-a2"));
    }

    #[test]
    fn test_parse_worktree_list_no_trailing_newline() {
        let output = "worktree /tmp/repo\nHEAD abc1234\nbranch refs/heads/main";
        let entries = parse_worktree_list(output);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/repo"));
    }

    #[test]
    fn test_branch_name_format() {
        let name = WorktreeManager::branch_name("test-123");
        assert!(name.starts_with("cisco-code/"));
        assert!(name.contains("test-123"));
    }

    #[test]
    fn test_worktree_base_dir() {
        let mgr = WorktreeManager::new(Path::new("/home/user/project"));
        let path = mgr.worktree_path("agent-1");
        assert_eq!(
            path,
            PathBuf::from("/home/user/project/.git/cisco-code-worktrees/agent-1")
        );
    }

    #[test]
    fn test_list_active_initially_empty() {
        let mgr = WorktreeManager::new(Path::new("/tmp/repo"));
        assert!(mgr.list_active().is_empty());
    }

    #[test]
    fn test_worktree_list_entry_default() {
        let entry = WorktreeListEntry::default();
        assert_eq!(entry.path, PathBuf::new());
        assert_eq!(entry.head, "");
        assert!(entry.branch.is_none());
        assert!(!entry.bare);
        assert!(!entry.detached);
    }
}
