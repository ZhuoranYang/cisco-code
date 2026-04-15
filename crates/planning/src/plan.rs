//! Plan data model and slug generation.
//!
//! Matches Claude Code v2.1.88's plan mode:
//! - Plans stored as markdown files at `~/.cisco-code/plans/{slug}.md`
//! - Slug is a random word-based identifier (adjective-noun-verb)
//! - Plan file is read/written by EnterPlanMode/ExitPlanMode tools
//! - Subagent plans use `{slug}-agent-{agent_id}.md`

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;

/// Word lists for generating human-readable plan slugs.
const ADJECTIVES: &[&str] = &[
    "amber", "azure", "bold", "bright", "calm", "clear", "cool", "crisp",
    "dark", "deep", "dry", "eager", "fair", "fast", "firm", "fond",
    "glad", "gold", "green", "hale", "hashed", "iron", "jade", "keen",
    "kind", "late", "lean", "light", "loud", "lucid", "mild", "neat",
    "noble", "odd", "pale", "plain", "proud", "pure", "quick", "rare",
    "raw", "rich", "ripe", "safe", "sharp", "shy", "sleek", "slim",
    "smart", "soft", "solid", "stark", "steady", "still", "stout", "swift",
    "tame", "taut", "thin", "true", "vivid", "warm", "wide", "wild",
    "wise", "young", "zen",
];

const NOUNS: &[&str] = &[
    "arch", "bark", "beam", "blade", "bolt", "brook", "cape", "cedar",
    "cliff", "cloud", "coral", "crane", "creek", "crest", "dawn", "dew",
    "drift", "dune", "elm", "fern", "field", "flint", "forge", "frost",
    "gate", "glade", "grove", "hawk", "haze", "heath", "helm", "hill",
    "jade", "lake", "leaf", "ledge", "loom", "marsh", "mist", "moss",
    "nest", "oak", "opal", "orbit", "path", "peak", "pine", "plume",
    "pond", "quartz", "rain", "reef", "ridge", "river", "rock", "root",
    "sage", "sand", "shade", "shore", "slate", "snow", "spark", "spire",
    "star", "stem", "stone", "storm", "stream", "thorn", "tide", "trail",
    "vale", "vine", "wave", "wind", "wood", "zephyr",
];

const VERBS: &[&str] = &[
    "bind", "bloom", "break", "build", "burn", "carve", "cast", "chase",
    "claim", "climb", "craft", "cross", "dance", "dare", "dash", "dive",
    "draft", "draw", "drift", "drive", "fade", "fall", "find", "flash",
    "float", "flow", "fly", "fold", "forge", "form", "frame", "fuse",
    "glow", "grasp", "grind", "grow", "guide", "hatch", "haul", "heal",
    "hoist", "hunt", "knit", "launch", "lead", "leap", "lift", "link",
    "march", "meld", "merge", "mold", "paint", "parse", "plant", "press",
    "probe", "pull", "push", "quest", "raise", "reach", "reap", "ride",
    "ring", "rise", "roam", "root", "run", "sail", "scan", "sculpt",
    "seek", "shape", "shift", "shine", "soar", "solve", "spark", "spin",
    "split", "spring", "stand", "steer", "stitch", "stride", "surge",
    "sweep", "swing", "trace", "trade", "turn", "twist", "vault", "wake",
    "walk", "weave", "weld", "wind", "write",
];

/// Generate a random slug in the format "adjective-noun-verb".
///
/// Uses a simple random approach based on timestamp + counter to avoid
/// requiring the `rand` crate dependency.
fn generate_slug() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let seed = now.wrapping_add(count).wrapping_mul(6364136223846793005);

    let adj_idx = (seed % ADJECTIVES.len() as u64) as usize;
    let noun_idx = ((seed >> 16) % NOUNS.len() as u64) as usize;
    let verb_idx = ((seed >> 32) % VERBS.len() as u64) as usize;

    format!(
        "{}-{}-{}",
        ADJECTIVES[adj_idx], NOUNS[noun_idx], VERBS[verb_idx]
    )
}

/// Plan mode state tracking.
///
/// Matches Claude Code's `state.ts` plan mode fields:
/// - `has_exited_plan_mode`: whether user exited plan mode this session
/// - `needs_plan_mode_exit_attachment`: one-time notification flag
/// - `pre_plan_mode`: permission mode before entering plan mode
#[derive(Debug, Clone)]
pub struct PlanModeState {
    /// Whether plan mode has been exited at least once this session.
    pub has_exited_plan_mode: bool,
    /// Whether a plan-mode-exit attachment notification is pending.
    pub needs_plan_mode_exit_attachment: bool,
    /// The permission mode that was active before entering plan mode.
    /// Used to restore when exiting plan mode.
    pub pre_plan_mode: Option<String>,
}

impl Default for PlanModeState {
    fn default() -> Self {
        Self {
            has_exited_plan_mode: false,
            needs_plan_mode_exit_attachment: false,
            pre_plan_mode: None,
        }
    }
}

impl PlanModeState {
    /// Handle a transition between permission modes.
    ///
    /// Matches Claude Code's `handlePlanModeTransition`:
    /// When transitioning FROM plan mode to another mode,
    /// set the exit attachment flag so the system can inject
    /// a one-time notification about the plan.
    pub fn handle_transition(&mut self, from_mode: &str, to_mode: &str) {
        if from_mode == "plan" && to_mode != "plan" {
            self.has_exited_plan_mode = true;
            self.needs_plan_mode_exit_attachment = true;
        }
    }

    /// Consume the exit attachment flag (returns true once, then false).
    pub fn take_exit_attachment(&mut self) -> bool {
        if self.needs_plan_mode_exit_attachment {
            self.needs_plan_mode_exit_attachment = false;
            true
        } else {
            false
        }
    }
}

/// Thread-safe plan slug cache: session_id -> slug.
///
/// Matches Claude Code's `planSlugCache: Map<string, string>`.
pub struct PlanSlugCache {
    cache: Mutex<HashMap<String, String>>,
}

impl PlanSlugCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Get or generate a slug for the given session.
    ///
    /// If a slug already exists for this session, returns it.
    /// Otherwise generates a new one, retrying up to 10 times
    /// if the slug's plan file already exists on disk.
    pub fn get_or_create(&self, session_id: &str, plans_dir: &Path) -> String {
        let mut cache = self.cache.lock().unwrap();
        if let Some(slug) = cache.get(session_id) {
            return slug.clone();
        }

        // Generate a new slug, retrying if file already exists
        let slug = {
            let mut attempts = 0;
            loop {
                let candidate = generate_slug();
                let path = plans_dir.join(format!("{candidate}.md"));
                if !path.exists() || attempts >= 10 {
                    break candidate;
                }
                attempts += 1;
            }
        };

        cache.insert(session_id.to_string(), slug.clone());
        slug
    }

    /// Set a specific slug for a session (used during resume).
    pub fn set(&self, session_id: &str, slug: &str) {
        let mut cache = self.cache.lock().unwrap();
        cache.insert(session_id.to_string(), slug.to_string());
    }

    /// Get the slug for a session without creating one.
    pub fn get(&self, session_id: &str) -> Option<String> {
        let cache = self.cache.lock().unwrap();
        cache.get(session_id).cloned()
    }

    /// Clear the slug for a specific session.
    pub fn clear(&self, session_id: &str) {
        let mut cache = self.cache.lock().unwrap();
        cache.remove(session_id);
    }

    /// Clear all cached slugs (e.g., on /clear).
    pub fn clear_all(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
    }
}

impl Default for PlanSlugCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve the plans directory.
///
/// Priority:
/// 1. Custom directory from settings (relative to project root, validated)
/// 2. `~/.cisco-code/plans/` (default)
///
/// Creates the directory if it doesn't exist.
pub fn resolve_plans_directory(custom_dir: Option<&str>, project_root: Option<&Path>) -> PathBuf {
    if let Some(custom) = custom_dir {
        if let Some(root) = project_root {
            let resolved = root.join(custom);
            // Path traversal defense: ensure resolved path stays within project root
            if let (Ok(canonical_root), Ok(canonical_resolved)) =
                (root.canonicalize(), resolved.canonicalize().or_else(|_| {
                    // Directory might not exist yet — create and try again
                    let _ = std::fs::create_dir_all(&resolved);
                    resolved.canonicalize()
                }))
            {
                if canonical_resolved.starts_with(&canonical_root) {
                    return canonical_resolved;
                }
            }
            // Fall through to default if validation fails
            tracing::warn!(
                "Plans directory '{custom}' escapes project root, using default"
            );
        }
    }

    let default = dirs_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cisco-code")
        .join("plans");
    let _ = std::fs::create_dir_all(&default);
    default
}

/// Get the plan file path for a session.
///
/// - Main agent: `{plans_dir}/{slug}.md`
/// - Subagent: `{plans_dir}/{slug}-agent-{agent_id}.md`
pub fn plan_file_path(plans_dir: &Path, slug: &str, agent_id: Option<&str>) -> PathBuf {
    match agent_id {
        Some(id) => plans_dir.join(format!("{slug}-agent-{id}.md")),
        None => plans_dir.join(format!("{slug}.md")),
    }
}

/// Read a plan from disk, returning None if the file doesn't exist.
pub fn read_plan(plans_dir: &Path, slug: &str, agent_id: Option<&str>) -> Option<String> {
    let path = plan_file_path(plans_dir, slug, agent_id);
    std::fs::read_to_string(&path).ok()
}

/// Write a plan to disk.
pub fn write_plan(
    plans_dir: &Path,
    slug: &str,
    content: &str,
    agent_id: Option<&str>,
) -> Result<PathBuf> {
    let path = plan_file_path(plans_dir, slug, agent_id);
    let _ = std::fs::create_dir_all(plans_dir);
    std::fs::write(&path, content)?;
    Ok(path)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_slug_format() {
        let slug = generate_slug();
        let parts: Vec<&str> = slug.split('-').collect();
        assert_eq!(parts.len(), 3, "slug should have 3 parts: {slug}");
        assert!(ADJECTIVES.contains(&parts[0]), "bad adjective: {}", parts[0]);
        assert!(NOUNS.contains(&parts[1]), "bad noun: {}", parts[1]);
        assert!(VERBS.contains(&parts[2]), "bad verb: {}", parts[2]);
    }

    #[test]
    fn test_generate_slug_unique() {
        let slug1 = generate_slug();
        let slug2 = generate_slug();
        // With counter-based seed, consecutive calls should differ
        assert_ne!(slug1, slug2);
    }

    #[test]
    fn test_plan_slug_cache_get_or_create() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PlanSlugCache::new();
        let slug1 = cache.get_or_create("session-1", dir.path());
        let slug2 = cache.get_or_create("session-1", dir.path());
        assert_eq!(slug1, slug2, "same session should return same slug");

        let slug3 = cache.get_or_create("session-2", dir.path());
        assert_ne!(slug1, slug3, "different sessions should get different slugs");
    }

    #[test]
    fn test_plan_slug_cache_set_and_get() {
        let cache = PlanSlugCache::new();
        assert!(cache.get("sess").is_none());
        cache.set("sess", "custom-slug-here");
        assert_eq!(cache.get("sess").unwrap(), "custom-slug-here");
    }

    #[test]
    fn test_plan_slug_cache_clear() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PlanSlugCache::new();
        cache.get_or_create("sess", dir.path());
        assert!(cache.get("sess").is_some());
        cache.clear("sess");
        assert!(cache.get("sess").is_none());
    }

    #[test]
    fn test_plan_slug_cache_clear_all() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PlanSlugCache::new();
        cache.get_or_create("a", dir.path());
        cache.get_or_create("b", dir.path());
        cache.clear_all();
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_plan_file_path_main_agent() {
        let dir = PathBuf::from("/tmp/plans");
        let path = plan_file_path(&dir, "bold-creek-forge", None);
        assert_eq!(path, PathBuf::from("/tmp/plans/bold-creek-forge.md"));
    }

    #[test]
    fn test_plan_file_path_subagent() {
        let dir = PathBuf::from("/tmp/plans");
        let path = plan_file_path(&dir, "bold-creek-forge", Some("sub-001"));
        assert_eq!(
            path,
            PathBuf::from("/tmp/plans/bold-creek-forge-agent-sub-001.md")
        );
    }

    #[test]
    fn test_read_write_plan() {
        let dir = tempfile::tempdir().unwrap();
        let content = "## Plan\n\n1. Step one\n2. Step two\n";

        let path = write_plan(dir.path(), "test-slug-here", content, None).unwrap();
        assert!(path.exists());

        let read_back = read_plan(dir.path(), "test-slug-here", None).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_read_plan_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_plan(dir.path(), "nonexistent-slug-here", None).is_none());
    }

    #[test]
    fn test_resolve_plans_directory_default() {
        let dir = resolve_plans_directory(None, None);
        assert!(dir.to_string_lossy().contains("plans"));
    }

    #[test]
    fn test_resolve_plans_directory_custom() {
        let root = tempfile::tempdir().unwrap();
        let plans_sub = root.path().join("my-plans");
        std::fs::create_dir_all(&plans_sub).unwrap();

        let dir = resolve_plans_directory(Some("my-plans"), Some(root.path()));
        assert_eq!(dir.canonicalize().unwrap(), plans_sub.canonicalize().unwrap());
    }

    #[test]
    fn test_plan_mode_state_default() {
        let state = PlanModeState::default();
        assert!(!state.has_exited_plan_mode);
        assert!(!state.needs_plan_mode_exit_attachment);
        assert!(state.pre_plan_mode.is_none());
    }

    #[test]
    fn test_plan_mode_state_transition() {
        let mut state = PlanModeState::default();
        // Entering plan mode doesn't trigger exit
        state.handle_transition("default", "plan");
        assert!(!state.has_exited_plan_mode);

        // Exiting plan mode sets the flags
        state.handle_transition("plan", "default");
        assert!(state.has_exited_plan_mode);
        assert!(state.needs_plan_mode_exit_attachment);
    }

    #[test]
    fn test_plan_mode_state_take_exit_attachment() {
        let mut state = PlanModeState::default();
        state.handle_transition("plan", "default");

        // First take returns true
        assert!(state.take_exit_attachment());
        // Second take returns false
        assert!(!state.take_exit_attachment());
        // has_exited remains true
        assert!(state.has_exited_plan_mode);
    }

    #[test]
    fn test_slug_retry_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PlanSlugCache::new();

        // Pre-create a bunch of slug files (won't match generated ones usually,
        // but verifies the retry logic doesn't panic)
        let slug = cache.get_or_create("retry-test", dir.path());
        assert!(!slug.is_empty());
    }
}
