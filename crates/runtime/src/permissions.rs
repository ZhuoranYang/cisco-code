//! Permission engine for tool execution control.
//!
//! Design insight from Claude Code: Granular permission system with:
//! - Mode-based defaults (default, accept-reads, bypass, deny-all)
//! - Path-based allowlists (allow specific file paths for write tools)
//! - Dangerous command/pattern detection
//! - Tool-specific overrides with glob patterns
//! - Session approval tracking
//!
//! The engine classifies tools by their PermissionLevel and checks against
//! the active mode, path rules, and dangerous patterns to determine if
//! execution is allowed, denied, or needs user confirmation.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::Instant;

use cisco_code_protocol::PermissionLevel;
use regex::Regex;

use crate::config::PermissionMode;

/// Result of a permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Tool execution is allowed without user confirmation.
    Allow,
    /// Tool execution requires user confirmation.
    Ask {
        /// Human-readable reason why confirmation is needed.
        reason: String,
    },
    /// Tool execution is denied.
    Deny {
        /// Human-readable reason for denial.
        reason: String,
    },
}

/// Per-tool permission override.
#[derive(Debug, Clone)]
pub struct ToolPermissionRule {
    /// Tool name or glob pattern (e.g., "Bash", "mcp:*").
    pub pattern: String,
    /// Override decision for matching tools.
    pub decision: PermissionOverride,
}

/// Override for a specific tool.
#[derive(Debug, Clone)]
pub enum PermissionOverride {
    AlwaysAllow,
    AlwaysDeny,
    /// Use the default mode-based decision.
    Default,
}

/// A path-based permission rule (allow writes to specific paths).
#[derive(Debug, Clone)]
pub struct PathRule {
    /// Regex pattern matching allowed file paths.
    pub pattern: Regex,
    /// Whether this rule allows or denies access.
    pub allow: bool,
}

impl PathRule {
    /// Create a rule allowing writes to paths matching the pattern.
    pub fn allow(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            pattern: Regex::new(pattern)?,
            allow: true,
        })
    }

    /// Create a rule denying writes to paths matching the pattern.
    pub fn deny(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            pattern: Regex::new(pattern)?,
            allow: false,
        })
    }
}

/// Dangerous patterns in Bash commands that should always require confirmation.
/// Matches Claude Code's dangerous command detection.
const DANGEROUS_BASH_PATTERNS: &[&str] = &[
    // Destructive file operations
    r"rm\s+(-[a-zA-Z]*f|-[a-zA-Z]*r|--force|--recursive)",
    r"rm\s+-rf\s+/",
    // Git destructive operations
    r"git\s+(push\s+--force|push\s+-f|reset\s+--hard|clean\s+-f|checkout\s+--\s+\.)",
    r"git\s+branch\s+-D",
    // Database destructive operations
    r"(?i)(DROP\s+(TABLE|DATABASE|INDEX|VIEW))",
    r"(?i)(TRUNCATE\s+TABLE)",
    r"(?im)(DELETE\s+FROM\s+\w+\s*(;|$|\n))",  // DELETE without WHERE
    // System-level danger
    r"chmod\s+(-R\s+)?777",
    r"mkfs\.",
    r"dd\s+.*of=/dev/",
    // Credential exposure
    r"curl\s+.*(-d|--data).*password",
    r"echo\s+.*>>\s*/etc/",
    // Process management
    r"kill\s+-9",
    r"pkill\s+-9",
    r"killall",
    // Network
    r"iptables\s+.*-F",
    r"ufw\s+disable",
];

/// Dangerous patterns in file paths that should trigger warnings.
const SENSITIVE_PATH_PATTERNS: &[&str] = &[
    r"\.env($|\.)",                        // .env files
    r"credentials\.(json|yaml|yml|env|cfg|conf|ini|xml|properties|toml)$",
    r"\.ssh/",
    r"\.aws/",
    r"\.gnupg/",
    r"id_rsa",
    r"\.pem$",
    r"\.key$",
    r"secrets?\.(json|yaml|yml|env|cfg|conf|ini|xml|properties|toml)$",
    r"passwords?\.(txt|json|yaml|yml|env|cfg|conf|ini|xml)$",
    r"/etc/(passwd|shadow|sudoers)",
];

/// Pre-compiled dangerous command regexes (compiled once, reused on every call).
static COMPILED_DANGEROUS_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    DANGEROUS_BASH_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok().map(|re| (re, *p)))
        .collect()
});

/// Pre-compiled sensitive path regexes (compiled once, reused on every call).
static COMPILED_SENSITIVE_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    SENSITIVE_PATH_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok().map(|re| (re, *p)))
        .collect()
});

/// Check if a command contains dangerous patterns.
pub fn detect_dangerous_command(command: &str) -> Option<String> {
    for (re, pattern) in COMPILED_DANGEROUS_PATTERNS.iter() {
        if re.is_match(command) {
            return Some(format!("Dangerous command detected: matches pattern `{pattern}`"));
        }
    }
    None
}

/// Check if a file path is sensitive.
pub fn detect_sensitive_path(path: &str) -> Option<String> {
    for (re, pattern) in COMPILED_SENSITIVE_PATTERNS.iter() {
        if re.is_match(path) {
            return Some(format!("Sensitive file path: matches `{pattern}`"));
        }
    }
    None
}

/// Tracks recent tool denials to prevent infinite retry loops.
///
/// If a tool is denied N times within a time window, the tracker
/// automatically escalates to a hard deny without prompting the user.
pub struct DenialTracker {
    /// (tool_name, timestamp) pairs of recent denials.
    denials: Vec<(String, Instant)>,
    /// Maximum denials before auto-deny.
    max_denials: usize,
    /// Time window for counting denials.
    window: std::time::Duration,
}

impl DenialTracker {
    pub fn new() -> Self {
        Self {
            denials: Vec::new(),
            max_denials: 3,
            window: std::time::Duration::from_secs(60),
        }
    }

    /// Record that a tool was denied by the user.
    pub fn record_denial(&mut self, tool_name: &str) {
        self.denials.push((tool_name.to_string(), Instant::now()));
        self.prune_old();
    }

    /// Check if a tool should be auto-denied (too many recent denials).
    pub fn should_auto_deny(&mut self, tool_name: &str) -> bool {
        self.prune_old();
        let count = self
            .denials
            .iter()
            .filter(|(name, _)| name == tool_name)
            .count();
        count >= self.max_denials
    }

    /// Remove entries older than the window.
    fn prune_old(&mut self) {
        let cutoff = Instant::now() - self.window;
        self.denials.retain(|(_, ts)| *ts > cutoff);
    }

    /// Get the number of recent denials for a tool.
    pub fn denial_count(&self, tool_name: &str) -> usize {
        let cutoff = Instant::now() - self.window;
        self.denials
            .iter()
            .filter(|(name, ts)| name == tool_name && *ts > cutoff)
            .count()
    }
}

impl Default for DenialTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// The permission engine evaluates tool access based on mode + per-tool rules.
pub struct PermissionEngine {
    mode: PermissionMode,
    /// Per-tool overrides, checked before mode-based rules.
    overrides: Vec<ToolPermissionRule>,
    /// Path-based allowlists for file write operations.
    path_rules: Vec<PathRule>,
    /// Tools that the user has approved in this session (runtime "remember" list).
    session_approved: Vec<String>,
    /// Specific (tool, input_summary) pairs approved for this session.
    session_approved_specific: HashSet<(String, String)>,
    /// Tracks repeated denials to auto-deny after threshold.
    denial_tracker: DenialTracker,
}

impl PermissionEngine {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            overrides: Vec::new(),
            path_rules: Vec::new(),
            session_approved: Vec::new(),
            session_approved_specific: HashSet::new(),
            denial_tracker: DenialTracker::new(),
        }
    }

    /// Add a per-tool permission override.
    pub fn add_override(&mut self, rule: ToolPermissionRule) {
        self.overrides.push(rule);
    }

    /// Add a path-based permission rule.
    pub fn add_path_rule(&mut self, rule: PathRule) {
        self.path_rules.push(rule);
    }

    /// Record that the user approved a tool for this session (blanket approval).
    pub fn approve_for_session(&mut self, tool_name: &str) {
        if !self.session_approved.contains(&tool_name.to_string()) {
            self.session_approved.push(tool_name.to_string());
        }
    }

    /// Record that the user approved a specific (tool, input) combination.
    pub fn approve_specific(&mut self, tool_name: &str, input_summary: &str) {
        self.session_approved_specific
            .insert((tool_name.to_string(), input_summary.to_string()));
    }

    /// Access the denial tracker (for recording denials from the agent loop).
    pub fn denial_tracker_mut(&mut self) -> &mut DenialTracker {
        &mut self.denial_tracker
    }

    /// Check whether a file path is allowed by path rules.
    fn check_path(&self, path: &str) -> Option<PermissionDecision> {
        // Deny rules take priority
        for rule in &self.path_rules {
            if !rule.allow && rule.pattern.is_match(path) {
                return Some(PermissionDecision::Deny {
                    reason: format!("Path '{path}' is blocked by deny rule"),
                });
            }
        }
        // Then check allow rules
        for rule in &self.path_rules {
            if rule.allow && rule.pattern.is_match(path) {
                return Some(PermissionDecision::Allow);
            }
        }
        None
    }

    /// Check whether a tool is allowed to execute.
    pub fn check(
        &mut self,
        tool_name: &str,
        permission_level: PermissionLevel,
        input_summary: &str,
    ) -> PermissionDecision {
        // 1. Check per-tool overrides first (before everything else,
        //    so AlwaysAllow/AlwaysDeny overrides take full effect)
        for rule in &self.overrides {
            if matches_pattern(&rule.pattern, tool_name) {
                match rule.decision {
                    PermissionOverride::AlwaysAllow => return PermissionDecision::Allow,
                    PermissionOverride::AlwaysDeny => {
                        return PermissionDecision::Deny {
                            reason: format!(
                                "Tool '{tool_name}' is blocked by permission rule"
                            ),
                        }
                    }
                    PermissionOverride::Default => break, // fall through to remaining checks
                }
            }
        }

        // 1b. Check for dangerous patterns in Bash commands (always warn,
        //     unless an explicit AlwaysAllow override was matched above)
        if tool_name == "Bash" {
            if let Some(warning) = detect_dangerous_command(input_summary) {
                return PermissionDecision::Ask {
                    reason: format!("⚠ {warning}. Command: {input_summary}"),
                };
            }
        }

        // 1c. Check for sensitive file paths in write tools
        if matches!(tool_name, "Write" | "Edit") {
            if let Some(warning) = detect_sensitive_path(input_summary) {
                return PermissionDecision::Ask {
                    reason: format!("⚠ {warning}: {input_summary}"),
                };
            }
        }

        // 1d. Check denial tracker for auto-deny (after per-tool overrides,
        //     so that AlwaysAllow overrides are not blocked by the tracker)
        if self.denial_tracker.should_auto_deny(tool_name) {
            return PermissionDecision::Deny {
                reason: format!(
                    "Tool '{}' has been denied too many times recently. \
                     Please adjust your approach.",
                    tool_name
                ),
            };
        }

        // 2. Check session approvals (specific first, then blanket)
        if self
            .session_approved_specific
            .contains(&(tool_name.to_string(), input_summary.to_string()))
        {
            return PermissionDecision::Allow;
        }
        if self.session_approved.contains(&tool_name.to_string()) {
            return PermissionDecision::Allow;
        }

        // 3. Check path-based rules for write/edit operations
        if matches!(
            permission_level,
            PermissionLevel::WorkspaceWrite | PermissionLevel::Execute
        ) {
            if let Some(decision) = self.check_path(input_summary) {
                return decision;
            }
        }

        // 4. Mode-based decision
        match self.mode {
            PermissionMode::BypassPermissions => PermissionDecision::Allow,

            PermissionMode::DenyAll => PermissionDecision::Deny {
                reason: "All tool execution is disabled (deny-all mode)".to_string(),
            },

            PermissionMode::AcceptReads => match permission_level {
                PermissionLevel::ReadOnly => PermissionDecision::Allow,
                PermissionLevel::WorkspaceWrite => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' wants to write files: {input_summary}"
                    ),
                },
                PermissionLevel::Execute => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' wants to execute a command: {input_summary}"
                    ),
                },
                PermissionLevel::Elevated => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' requires elevated permissions: {input_summary}"
                    ),
                },
            },

            PermissionMode::Default => match permission_level {
                PermissionLevel::ReadOnly => PermissionDecision::Ask {
                    reason: format!("'{tool_name}' wants to read: {input_summary}"),
                },
                PermissionLevel::WorkspaceWrite => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' wants to write files: {input_summary}"
                    ),
                },
                PermissionLevel::Execute => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' wants to execute a command: {input_summary}"
                    ),
                },
                PermissionLevel::Elevated => PermissionDecision::Ask {
                    reason: format!(
                        "'{tool_name}' requires elevated permissions: {input_summary}"
                    ),
                },
            },

            // Plan mode: only read-only tools are allowed. All writes/executions
            // are denied. Matches Claude Code's plan mode behavior.
            PermissionMode::Plan => match permission_level {
                PermissionLevel::ReadOnly => PermissionDecision::Allow,
                _ => PermissionDecision::Deny {
                    reason: format!(
                        "'{tool_name}' is not allowed in plan mode (read-only). \
                         Exit plan mode first to make changes."
                    ),
                },
            },
        }
    }

    /// Get the current permission mode.
    pub fn mode(&self) -> &PermissionMode {
        &self.mode
    }

    /// Set a new permission mode.
    pub fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }

    /// Get the number of path rules configured.
    pub fn path_rule_count(&self) -> usize {
        self.path_rules.len()
    }
}

/// Simple glob-like pattern matching for tool names.
/// Supports:
/// - Exact match: "Bash" matches "Bash"
/// - Wildcard suffix: "mcp:*" matches "mcp:github", "mcp:slack"
/// - Universal wildcard: "*" matches everything
fn matches_pattern(pattern: &str, tool_name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return tool_name.starts_with(prefix);
    }
    pattern == tool_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_allows_everything() {
        let mut engine = PermissionEngine::new(PermissionMode::BypassPermissions);
        assert_eq!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Allow
        );
        assert_eq!(
            engine.check("Write", PermissionLevel::WorkspaceWrite, "file.rs"),
            PermissionDecision::Allow
        );
        assert_eq!(
            engine.check("Read", PermissionLevel::ReadOnly, "file.rs"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_deny_all_blocks_everything() {
        let mut engine = PermissionEngine::new(PermissionMode::DenyAll);
        let result = engine.check("Read", PermissionLevel::ReadOnly, "file.rs");
        assert!(matches!(result, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn test_accept_reads_allows_readonly() {
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        assert_eq!(
            engine.check("Read", PermissionLevel::ReadOnly, "file.rs"),
            PermissionDecision::Allow
        );
        assert_eq!(
            engine.check("Grep", PermissionLevel::ReadOnly, "pattern"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_accept_reads_asks_for_writes() {
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        let result = engine.check("Write", PermissionLevel::WorkspaceWrite, "new.rs");
        assert!(matches!(result, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn test_accept_reads_asks_for_execute() {
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        let result = engine.check("Bash", PermissionLevel::Execute, "rm -rf /");
        assert!(matches!(result, PermissionDecision::Ask { .. }));
    }

    #[test]
    fn test_default_asks_for_everything() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        assert!(matches!(
            engine.check("Read", PermissionLevel::ReadOnly, "file.rs"),
            PermissionDecision::Ask { .. }
        ));
        assert!(matches!(
            engine.check("Write", PermissionLevel::WorkspaceWrite, "file.rs"),
            PermissionDecision::Ask { .. }
        ));
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn test_per_tool_override_always_allow() {
        let mut engine = PermissionEngine::new(PermissionMode::DenyAll);
        engine.add_override(ToolPermissionRule {
            pattern: "Read".into(),
            decision: PermissionOverride::AlwaysAllow,
        });
        assert_eq!(
            engine.check("Read", PermissionLevel::ReadOnly, "file.rs"),
            PermissionDecision::Allow
        );
        // Other tools still denied
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn test_per_tool_override_always_deny() {
        let mut engine = PermissionEngine::new(PermissionMode::BypassPermissions);
        engine.add_override(ToolPermissionRule {
            pattern: "Bash".into(),
            decision: PermissionOverride::AlwaysDeny,
        });
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Deny { .. }
        ));
        // Other tools still allowed
        assert_eq!(
            engine.check("Read", PermissionLevel::ReadOnly, "file.rs"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_wildcard_pattern_matching() {
        let mut engine = PermissionEngine::new(PermissionMode::DenyAll);
        engine.add_override(ToolPermissionRule {
            pattern: "mcp:*".into(),
            decision: PermissionOverride::AlwaysAllow,
        });
        assert_eq!(
            engine.check("mcp:github", PermissionLevel::Execute, ""),
            PermissionDecision::Allow
        );
        assert_eq!(
            engine.check("mcp:slack", PermissionLevel::Execute, ""),
            PermissionDecision::Allow
        );
        // Non-mcp tools still denied
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, ""),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn test_universal_wildcard() {
        assert!(matches_pattern("*", "anything"));
        assert!(matches_pattern("*", "Bash"));
        assert!(matches_pattern("*", "mcp:github"));
    }

    #[test]
    fn test_session_approval() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        // Before approval, asks
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Ask { .. }
        ));

        // After approval, allows
        engine.approve_for_session("Bash");
        assert_eq!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn test_session_approval_idempotent() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        engine.approve_for_session("Bash");
        engine.approve_for_session("Bash");
        assert_eq!(engine.session_approved.len(), 1);
    }

    #[test]
    fn test_override_takes_precedence_over_session_approval() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        engine.approve_for_session("Bash");
        engine.add_override(ToolPermissionRule {
            pattern: "Bash".into(),
            decision: PermissionOverride::AlwaysDeny,
        });
        // Override wins over session approval
        assert!(matches!(
            engine.check("Bash", PermissionLevel::Execute, "ls"),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn test_set_mode() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        assert!(matches!(engine.mode(), PermissionMode::Default));
        engine.set_mode(PermissionMode::BypassPermissions);
        assert!(matches!(engine.mode(), PermissionMode::BypassPermissions));
    }

    #[test]
    fn test_matches_pattern_exact() {
        assert!(matches_pattern("Bash", "Bash"));
        assert!(!matches_pattern("Bash", "bash"));
        assert!(!matches_pattern("Bash", "BashTool"));
    }

    #[test]
    fn test_matches_pattern_prefix_wildcard() {
        assert!(matches_pattern("mcp:*", "mcp:github"));
        assert!(matches_pattern("mcp:*", "mcp:"));
        assert!(!matches_pattern("mcp:*", "Bash"));
    }

    #[test]
    fn test_elevated_permission_in_accept_reads() {
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        let result = engine.check("DangerousTool", PermissionLevel::Elevated, "destructive op");
        assert!(matches!(result, PermissionDecision::Ask { .. }));
        if let PermissionDecision::Ask { reason } = result {
            assert!(reason.contains("elevated"));
        }
    }

    // --- Dangerous pattern detection tests ---

    #[test]
    fn test_dangerous_rm_rf() {
        let result = detect_dangerous_command("rm -rf /tmp/important");
        assert!(result.is_some());
    }

    #[test]
    fn test_dangerous_git_force_push() {
        let result = detect_dangerous_command("git push --force origin main");
        assert!(result.is_some());
    }

    #[test]
    fn test_dangerous_git_reset_hard() {
        let result = detect_dangerous_command("git reset --hard HEAD~5");
        assert!(result.is_some());
    }

    #[test]
    fn test_dangerous_drop_table() {
        let result = detect_dangerous_command("DROP TABLE users");
        assert!(result.is_some());
    }

    #[test]
    fn test_dangerous_chmod_777() {
        let result = detect_dangerous_command("chmod 777 /var/www");
        assert!(result.is_some());
    }

    #[test]
    fn test_safe_command_not_flagged() {
        assert!(detect_dangerous_command("ls -la").is_none());
        assert!(detect_dangerous_command("git status").is_none());
        assert!(detect_dangerous_command("cargo test").is_none());
        assert!(detect_dangerous_command("cat /tmp/file.txt").is_none());
    }

    #[test]
    fn test_sensitive_env_path() {
        assert!(detect_sensitive_path(".env").is_some());
        assert!(detect_sensitive_path(".env.local").is_some());
        assert!(detect_sensitive_path("/home/user/.ssh/id_rsa").is_some());
        assert!(detect_sensitive_path("credentials.json").is_some());
    }

    #[test]
    fn test_normal_path_not_flagged() {
        assert!(detect_sensitive_path("src/main.rs").is_none());
        assert!(detect_sensitive_path("README.md").is_none());
        assert!(detect_sensitive_path("package.json").is_none());
    }

    // --- Dangerous pattern integration in permission checks ---

    #[test]
    fn test_bash_dangerous_always_asks() {
        // Even in bypass mode, dangerous commands should still trigger a warning
        // (note: bypass mode skips this check for safety — this tests accept-reads)
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        let result = engine.check("Bash", PermissionLevel::Execute, "rm -rf /");
        match result {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("Dangerous"));
            }
            _ => panic!("expected Ask for dangerous command, got {:?}", result),
        }
    }

    #[test]
    fn test_write_sensitive_path_asks() {
        let mut engine = PermissionEngine::new(PermissionMode::AcceptReads);
        let result = engine.check("Write", PermissionLevel::WorkspaceWrite, ".env.production");
        match result {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("Sensitive"));
            }
            _ => panic!("expected Ask for sensitive path, got {:?}", result),
        }
    }

    // --- Path-based allowlist tests ---

    #[test]
    fn test_path_rule_allow() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        engine.add_path_rule(PathRule::allow(r"^src/").unwrap());

        let result = engine.check("Write", PermissionLevel::WorkspaceWrite, "src/main.rs");
        assert_eq!(result, PermissionDecision::Allow);
    }

    #[test]
    fn test_path_rule_deny_takes_priority() {
        let mut engine = PermissionEngine::new(PermissionMode::BypassPermissions);
        engine.add_path_rule(PathRule::allow(r"^src/").unwrap());
        engine.add_path_rule(PathRule::deny(r"\.secret").unwrap());

        let result = engine.check("Write", PermissionLevel::WorkspaceWrite, "src/app.secret");
        assert!(matches!(result, PermissionDecision::Deny { .. }));
    }

    #[test]
    fn test_path_rule_no_match_falls_through() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        engine.add_path_rule(PathRule::allow(r"^src/").unwrap());

        // Path doesn't match — falls through to mode-based check
        let result = engine.check("Write", PermissionLevel::WorkspaceWrite, "docs/readme.md");
        assert!(matches!(result, PermissionDecision::Ask { .. }));
    }

    // --- Specific approval tests ---

    #[test]
    fn test_approve_specific() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        engine.approve_specific("Write", "src/main.rs");

        assert_eq!(
            engine.check("Write", PermissionLevel::WorkspaceWrite, "src/main.rs"),
            PermissionDecision::Allow
        );
        // Different input still asks
        assert!(matches!(
            engine.check("Write", PermissionLevel::WorkspaceWrite, "src/lib.rs"),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn test_delete_without_where() {
        let result = detect_dangerous_command("DELETE FROM users;");
        assert!(result.is_some());
    }

    #[test]
    fn test_kill_9() {
        let result = detect_dangerous_command("kill -9 12345");
        assert!(result.is_some());
    }

    #[test]
    fn test_path_rule_count() {
        let mut engine = PermissionEngine::new(PermissionMode::Default);
        assert_eq!(engine.path_rule_count(), 0);
        engine.add_path_rule(PathRule::allow(r"^src/").unwrap());
        assert_eq!(engine.path_rule_count(), 1);
    }

    #[test]
    fn test_denial_tracker_basic() {
        let mut tracker = DenialTracker::new();
        assert!(!tracker.should_auto_deny("Bash"));

        tracker.record_denial("Bash");
        tracker.record_denial("Bash");
        assert!(!tracker.should_auto_deny("Bash"));

        tracker.record_denial("Bash");
        assert!(tracker.should_auto_deny("Bash"));
    }

    #[test]
    fn test_denial_tracker_different_tools() {
        let mut tracker = DenialTracker::new();
        tracker.record_denial("Bash");
        tracker.record_denial("Bash");
        tracker.record_denial("Bash");

        // Bash is auto-denied, but Read is not
        assert!(tracker.should_auto_deny("Bash"));
        assert!(!tracker.should_auto_deny("Read"));
    }

    #[test]
    fn test_denial_tracker_count() {
        let mut tracker = DenialTracker::new();
        assert_eq!(tracker.denial_count("Bash"), 0);
        tracker.record_denial("Bash");
        assert_eq!(tracker.denial_count("Bash"), 1);
        tracker.record_denial("Bash");
        assert_eq!(tracker.denial_count("Bash"), 2);
    }

    #[test]
    fn test_denial_auto_deny_in_permission_check() {
        let mut engine = PermissionEngine::new(PermissionMode::BypassPermissions);
        // Even in bypass mode, after 3 denials the tracker should auto-deny
        engine.denial_tracker_mut().record_denial("Bash");
        engine.denial_tracker_mut().record_denial("Bash");
        engine.denial_tracker_mut().record_denial("Bash");

        let result = engine.check("Bash", PermissionLevel::Execute, "ls");
        assert!(matches!(result, PermissionDecision::Deny { .. }));
    }
}
