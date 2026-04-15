//! PlanManager — orchestrates plan mode lifecycle.
//!
//! Matches Claude Code v2.1.88's plan management:
//! - Slug cache per session
//! - File-based plan storage
//! - Resume/fork support (copy plan for new sessions)
//! - Plans directory resolution with path traversal defense

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::plan::{
    plan_file_path, read_plan, resolve_plans_directory, write_plan,
    PlanModeState, PlanSlugCache,
};

/// Manages plan mode lifecycle for a session.
///
/// The PlanManager is the single authority for:
/// - Where plans are stored (plans directory)
/// - What slug maps to what session
/// - Reading/writing plan content
/// - Copying plans across sessions (resume/fork)
pub struct PlanManager {
    /// Thread-safe slug cache shared across the runtime.
    slug_cache: Arc<PlanSlugCache>,
    /// Resolved plans directory path.
    plans_dir: PathBuf,
    /// Plan mode state (exit flags, pre-plan mode, etc.).
    pub state: PlanModeState,
}

impl PlanManager {
    /// Create a new PlanManager with default plans directory.
    pub fn new() -> Self {
        Self {
            slug_cache: Arc::new(PlanSlugCache::new()),
            plans_dir: resolve_plans_directory(None, None),
            state: PlanModeState::default(),
        }
    }

    /// Create a PlanManager with a custom plans directory.
    pub fn with_plans_dir(
        custom_dir: Option<&str>,
        project_root: Option<&Path>,
    ) -> Self {
        Self {
            slug_cache: Arc::new(PlanSlugCache::new()),
            plans_dir: resolve_plans_directory(custom_dir, project_root),
            state: PlanModeState::default(),
        }
    }

    /// Create a PlanManager with a shared slug cache (for multi-session scenarios).
    pub fn with_shared_cache(
        slug_cache: Arc<PlanSlugCache>,
        custom_dir: Option<&str>,
        project_root: Option<&Path>,
    ) -> Self {
        Self {
            slug_cache,
            plans_dir: resolve_plans_directory(custom_dir, project_root),
            state: PlanModeState::default(),
        }
    }

    /// Get the plans directory path.
    pub fn plans_dir(&self) -> &Path {
        &self.plans_dir
    }

    /// Get the plan slug for a session, creating one if needed.
    pub fn get_plan_slug(&self, session_id: &str) -> String {
        self.slug_cache.get_or_create(session_id, &self.plans_dir)
    }

    /// Get the plan slug without creating one. Returns None if no slug exists.
    pub fn get_plan_slug_if_exists(&self, session_id: &str) -> Option<String> {
        self.slug_cache.get(session_id)
    }

    /// Set a specific slug for a session (used during resume).
    pub fn set_plan_slug(&self, session_id: &str, slug: &str) {
        self.slug_cache.set(session_id, slug);
    }

    /// Clear the slug for a session.
    pub fn clear_plan_slug(&self, session_id: &str) {
        self.slug_cache.clear(session_id);
    }

    /// Clear all cached slugs.
    pub fn clear_all_slugs(&self) {
        self.slug_cache.clear_all();
    }

    /// Get the plan file path for a session.
    pub fn get_plan_file_path(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> PathBuf {
        let slug = self.get_plan_slug(session_id);
        plan_file_path(&self.plans_dir, &slug, agent_id)
    }

    /// Read the plan for a session from disk.
    pub fn get_plan(
        &self,
        session_id: &str,
        agent_id: Option<&str>,
    ) -> Option<String> {
        let slug = self.slug_cache.get(session_id)?;
        read_plan(&self.plans_dir, &slug, agent_id)
    }

    /// Write a plan to disk for a session.
    pub fn save_plan(
        &self,
        session_id: &str,
        content: &str,
        agent_id: Option<&str>,
    ) -> Result<PathBuf> {
        let slug = self.get_plan_slug(session_id);
        write_plan(&self.plans_dir, &slug, content, agent_id)
    }

    /// Copy a plan from one session to another (for resume).
    ///
    /// Matches Claude Code's `copyPlanForResume`:
    /// - Sets the target session's slug from the source
    /// - Reads the plan file directly
    /// - Returns the plan content if found
    pub fn copy_plan_for_resume(
        &self,
        source_session_id: &str,
        target_session_id: &str,
    ) -> Option<String> {
        let source_slug = self.slug_cache.get(source_session_id)?;
        // Set the same slug for the target session
        self.slug_cache.set(target_session_id, &source_slug);
        // Read the plan (it's the same file)
        read_plan(&self.plans_dir, &source_slug, None)
    }

    /// Fork a plan for a parallel session.
    ///
    /// Matches Claude Code's `copyPlanForFork`:
    /// - Reads the original plan
    /// - Generates a NEW slug for the forked session
    /// - Writes the plan content to the new file
    pub fn copy_plan_for_fork(
        &self,
        source_session_id: &str,
        target_session_id: &str,
    ) -> Result<Option<String>> {
        let source_slug = match self.slug_cache.get(source_session_id) {
            Some(s) => s,
            None => return Ok(None),
        };

        let content = match read_plan(&self.plans_dir, &source_slug, None) {
            Some(c) => c,
            None => return Ok(None),
        };

        // Generate a new slug for the forked session (don't reuse the original)
        let new_slug = self.get_plan_slug(target_session_id);
        write_plan(&self.plans_dir, &new_slug, &content, None)?;

        Ok(Some(content))
    }

    /// Enter plan mode: record the current permission mode as pre_plan_mode.
    pub fn enter_plan_mode(&mut self, current_mode: &str) {
        self.state.pre_plan_mode = Some(current_mode.to_string());
    }

    /// Exit plan mode: restore the pre-plan permission mode.
    ///
    /// Returns the mode to restore to, or "default" if unknown.
    pub fn exit_plan_mode(&mut self) -> String {
        let restore_to = self
            .state
            .pre_plan_mode
            .take()
            .unwrap_or_else(|| "default".to_string());
        self.state.handle_transition("plan", &restore_to);
        restore_to
    }

    /// Check if plan mode has been exited this session.
    pub fn has_exited_plan_mode(&self) -> bool {
        self.state.has_exited_plan_mode
    }

    /// Take the exit attachment flag (returns true once after exiting plan mode).
    pub fn take_exit_attachment(&mut self) -> bool {
        self.state.take_exit_attachment()
    }
}

impl Default for PlanManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_in_temp() -> (PlanManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let manager = PlanManager {
            slug_cache: Arc::new(PlanSlugCache::new()),
            plans_dir: dir.path().to_path_buf(),
            state: PlanModeState::default(),
        };
        (manager, dir)
    }

    #[test]
    fn test_plan_manager_new() {
        let manager = PlanManager::new();
        assert!(manager.plans_dir().to_string_lossy().contains("plans"));
    }

    #[test]
    fn test_get_plan_slug_creates_and_caches() {
        let (manager, _dir) = manager_in_temp();
        let slug1 = manager.get_plan_slug("sess-1");
        let slug2 = manager.get_plan_slug("sess-1");
        assert_eq!(slug1, slug2);
        assert!(!slug1.is_empty());
    }

    #[test]
    fn test_get_plan_slug_if_exists() {
        let (manager, _dir) = manager_in_temp();
        assert!(manager.get_plan_slug_if_exists("sess-1").is_none());
        manager.get_plan_slug("sess-1");
        assert!(manager.get_plan_slug_if_exists("sess-1").is_some());
    }

    #[test]
    fn test_set_and_clear_slug() {
        let (manager, _dir) = manager_in_temp();
        manager.set_plan_slug("sess-1", "my-custom-slug");
        assert_eq!(
            manager.get_plan_slug_if_exists("sess-1").unwrap(),
            "my-custom-slug"
        );
        manager.clear_plan_slug("sess-1");
        assert!(manager.get_plan_slug_if_exists("sess-1").is_none());
    }

    #[test]
    fn test_save_and_get_plan() {
        let (manager, _dir) = manager_in_temp();
        let content = "## My Plan\n\n1. Do things\n";
        manager.save_plan("sess-1", content, None).unwrap();

        let read_back = manager.get_plan("sess-1", None).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_get_plan_no_slug() {
        let (manager, _dir) = manager_in_temp();
        assert!(manager.get_plan("nonexistent", None).is_none());
    }

    #[test]
    fn test_get_plan_file_path() {
        let (manager, _dir) = manager_in_temp();
        let path = manager.get_plan_file_path("sess-1", None);
        assert!(path.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn test_copy_plan_for_resume() {
        let (manager, _dir) = manager_in_temp();
        manager.save_plan("source", "Resume plan content", None).unwrap();

        let content = manager.copy_plan_for_resume("source", "target");
        assert_eq!(content.unwrap(), "Resume plan content");

        // Target should now share the same slug
        assert_eq!(
            manager.get_plan_slug_if_exists("target").unwrap(),
            manager.get_plan_slug_if_exists("source").unwrap()
        );
    }

    #[test]
    fn test_copy_plan_for_fork() {
        let (manager, _dir) = manager_in_temp();
        manager.save_plan("source", "Fork plan content", None).unwrap();

        let content = manager.copy_plan_for_fork("source", "forked").unwrap();
        assert_eq!(content.unwrap(), "Fork plan content");

        // Forked should have a DIFFERENT slug
        assert_ne!(
            manager.get_plan_slug_if_exists("forked").unwrap(),
            manager.get_plan_slug_if_exists("source").unwrap()
        );

        // But same content
        assert_eq!(
            manager.get_plan("forked", None).unwrap(),
            "Fork plan content"
        );
    }

    #[test]
    fn test_copy_plan_for_fork_no_source() {
        let (manager, _dir) = manager_in_temp();
        let result = manager.copy_plan_for_fork("nonexistent", "target").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_enter_exit_plan_mode() {
        let (mut manager, _dir) = manager_in_temp();
        assert!(!manager.has_exited_plan_mode());

        manager.enter_plan_mode("accept_reads");
        assert_eq!(
            manager.state.pre_plan_mode.as_deref(),
            Some("accept_reads")
        );

        let restored = manager.exit_plan_mode();
        assert_eq!(restored, "accept_reads");
        assert!(manager.has_exited_plan_mode());
        assert!(manager.take_exit_attachment());
        assert!(!manager.take_exit_attachment()); // consumed
    }

    #[test]
    fn test_exit_plan_mode_no_pre_mode() {
        let (mut manager, _dir) = manager_in_temp();
        let restored = manager.exit_plan_mode();
        assert_eq!(restored, "default");
    }

    #[test]
    fn test_clear_all_slugs() {
        let (manager, _dir) = manager_in_temp();
        manager.get_plan_slug("a");
        manager.get_plan_slug("b");
        manager.clear_all_slugs();
        assert!(manager.get_plan_slug_if_exists("a").is_none());
        assert!(manager.get_plan_slug_if_exists("b").is_none());
    }
}
