//! System prompt builder with multi-source instruction loading.
//!
//! Matching Claude Code's full prompt assembly:
//!
//! Instruction file priority (first found wins per directory):
//!   Agents.md > cisco-code.md > CLAUDE.md
//!
//! Config directory priority (all are checked, results merged):
//!   .cisco-code/ (primary) > .claude/ > .codex/
//!
//! User home priority:
//!   ~/.cisco-code/ > ~/.claude/ > ~/.codex/
//!
//! Per-directory files searched:
//!   - Agents.md / cisco-code.md / CLAUDE.md (instruction file)
//!   - CLAUDE.local.md (local overrides, gitignored)
//!   - <config>/rules/*.md (rule fragments)
//!   - <config>/CLAUDE.md or <config>/instructions.md (nested instructions)
//!
//! Sections (in order):
//! 1. CORE — agent identity and behavior rules
//! 2. TOOL GUIDELINES — correct tool usage patterns
//! 3. PROJECT INSTRUCTIONS — merged instruction files
//! 4. CACHE BOUNDARY — separates static from dynamic
//! 5. ENVIRONMENT — cwd, platform, shell, git, model
//! 6. SCRATCHPAD — per-session temp directory
//! 7. MEMORY — user/project memories
//! 8. DATE — current date

use std::path::{Path, PathBuf};

/// Marker that separates cacheable (static) prompt from dynamic per-turn content.
/// Matches Claude Code's `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__`.
pub const CACHE_BOUNDARY: &str = "\n<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->\n";

/// Config directory names we search (in priority order).
const CONFIG_DIRS: &[&str] = &[".cisco-code", ".claude", ".codex"];

/// User home config directory names (in priority order).
const HOME_CONFIG_DIRS: &[&str] = &[".cisco-code", ".claude", ".codex"];

/// Instruction file names (in priority order — first match wins per directory).
const INSTRUCTION_FILES: &[&str] = &[
    "Agents.md",
    "cisco-code.md",
    "CLAUDE.md",
    "AGENTS.md",
];

/// Local override file (gitignored, not committed).
const LOCAL_OVERRIDE_FILE: &str = "CLAUDE.local.md";

/// Max chars per single instruction file to prevent prompt bloat.
const MAX_INSTRUCTION_FILE_CHARS: usize = 8_000;

/// Max total instruction chars across all files.
const MAX_TOTAL_INSTRUCTION_CHARS: usize = 24_000;

/// Max parent directories to walk upward.
const MAX_PARENT_WALK: usize = 10;

/// Git repository context for the environment section.
#[derive(Debug, Clone)]
pub struct GitContext {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub has_uncommitted: bool,
    /// Summary of uncommitted changes (e.g., "3 files modified, 1 untracked").
    pub status_summary: Option<String>,
    /// Recent commit messages.
    pub recent_commits: Vec<String>,
}

/// A discovered instruction file with its source.
#[derive(Debug, Clone)]
pub struct InstructionFile {
    pub path: PathBuf,
    pub content: String,
    pub source: InstructionSource,
}

/// Where an instruction file came from.
#[derive(Debug, Clone, PartialEq)]
pub enum InstructionSource {
    /// User-level (~/.cisco-code/CLAUDE.md)
    User,
    /// User rules (~/.cisco-code/rules/*.md)
    UserRule,
    /// Project-level (in CWD or parent)
    Project,
    /// Local override (CLAUDE.local.md)
    Local,
    /// Rule fragment (<config>/rules/*.md)
    Rule,
}

/// Builds the system prompt from layered sections.
pub struct PromptBuilder {
    cwd: String,
    model: Option<String>,
    custom_instructions: Option<String>,
    memory_content: Option<String>,
    git_context: Option<GitContext>,
    scratchpad_dir: Option<String>,
    skills_context: Option<String>,
    todo_content: Option<String>,
    mcp_instructions: Option<String>,
    /// Active plan content (injected when a plan exists from a previous plan mode session).
    plan_content: Option<String>,
    /// Plan file path (for reference in prompts).
    plan_file_path: Option<String>,
    /// Whether we're currently in plan mode.
    in_plan_mode: bool,
}

impl PromptBuilder {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            model: None,
            custom_instructions: None,
            memory_content: None,
            git_context: None,
            scratchpad_dir: None,
            skills_context: None,
            todo_content: None,
            mcp_instructions: None,
            plan_content: None,
            plan_file_path: None,
            in_plan_mode: false,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.custom_instructions = Some(instructions.into());
        self
    }

    pub fn with_memory(mut self, memory: impl Into<String>) -> Self {
        self.memory_content = Some(memory.into());
        self
    }

    pub fn with_git_context(mut self, ctx: GitContext) -> Self {
        self.git_context = Some(ctx);
        self
    }

    pub fn with_scratchpad(mut self, dir: impl Into<String>) -> Self {
        self.scratchpad_dir = Some(dir.into());
        self
    }

    pub fn with_skills(mut self, skills: impl Into<String>) -> Self {
        self.skills_context = Some(skills.into());
        self
    }

    pub fn with_todos(mut self, todos: impl Into<String>) -> Self {
        self.todo_content = Some(todos.into());
        self
    }

    pub fn with_mcp_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.mcp_instructions = Some(instructions.into());
        self
    }

    /// Set the active plan content (from a previous plan mode session).
    pub fn with_plan(mut self, content: impl Into<String>, file_path: impl Into<String>) -> Self {
        self.plan_content = Some(content.into());
        self.plan_file_path = Some(file_path.into());
        self
    }

    /// Mark that we're currently in plan mode.
    pub fn in_plan_mode(mut self, active: bool) -> Self {
        self.in_plan_mode = active;
        self
    }

    /// Build the full system prompt with cache boundary.
    pub fn build(&self) -> String {
        let mut sections = Vec::new();

        // ---- Cacheable (static) sections ----

        // 1. Core agent identity and behavior
        sections.push(self.core_section());

        // 2. Tool usage guidelines
        sections.push(self.tool_guidelines_section());

        // 3. Project-specific instructions
        if let Some(ref instructions) = self.custom_instructions {
            sections.push(format!(
                "# Project Instructions\n\nIMPORTANT: These instructions OVERRIDE any default behavior.\n\n{instructions}"
            ));
        }

        // 4. Skills context (if any active skills)
        if let Some(ref skills) = self.skills_context {
            sections.push(format!("# Available Skills\n\n{skills}"));
        }

        // Cache boundary — everything above can be cached, below is per-turn
        sections.push(CACHE_BOUNDARY.to_string());

        // ---- Dynamic (per-turn) sections ----

        // 5. Environment context
        sections.push(self.environment_section());

        // 6. Scratchpad
        if let Some(ref dir) = self.scratchpad_dir {
            sections.push(format!(
                "# Scratchpad\n\nYou have a dedicated scratchpad directory at `{dir}`. \
                Use this for temporary files, intermediate results, and working space. \
                This directory persists for the duration of this session. \
                Prefer this over /tmp for any temporary files you need to create."
            ));
        }

        // 7. Memory
        if let Some(ref memory) = self.memory_content {
            sections.push(format!("# Memory\n\n{memory}"));
        }

        // 8. Todos
        if let Some(ref todos) = self.todo_content {
            sections.push(format!("# Current Todos\n\n{todos}"));
        }

        // 9. MCP instructions
        if let Some(ref mcp) = self.mcp_instructions {
            sections.push(format!("# MCP Server Instructions\n\n{mcp}"));
        }

        // 10. Plan context
        if self.in_plan_mode {
            sections.push(
                "# Plan Mode Active\n\n\
                 You are currently in **plan mode**. Focus on research, analysis, and planning.\n\
                 DO NOT write or edit any code files. Only use read-only tools.\n\
                 When your plan is ready, call `ExitPlanMode` with your plan to present it for approval."
                    .to_string(),
            );
        }
        if let Some(ref plan) = self.plan_content {
            let mut plan_section = String::from("# Active Plan\n\n");
            if let Some(ref path) = self.plan_file_path {
                plan_section.push_str(&format!("Plan file: `{path}`\n\n"));
            }
            plan_section.push_str("Follow this plan. Mark steps as you complete them.\n\n");
            plan_section.push_str(plan);
            sections.push(plan_section);
        }

        // 11. Current date
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        sections.push(format!("# Current Date\nToday is {date}."));

        sections.join("\n\n")
    }

    /// Build only the static (cacheable) portion.
    pub fn build_static(&self) -> String {
        let full = self.build();
        if let Some(idx) = full.find(CACHE_BOUNDARY) {
            full[..idx].to_string()
        } else {
            full
        }
    }

    /// Build only the dynamic (per-turn) portion.
    pub fn build_dynamic(&self) -> String {
        let full = self.build();
        if let Some(idx) = full.find(CACHE_BOUNDARY) {
            full[idx + CACHE_BOUNDARY.len()..].to_string()
        } else {
            String::new()
        }
    }

    /// Build structured system blocks with cache control metadata.
    ///
    /// Returns a two-block vector:
    /// - Block 0: static/cacheable content with `cache_control: ephemeral`
    /// - Block 1: dynamic per-turn content without cache control
    ///
    /// This enables Anthropic's prompt caching — the static portion
    /// (agent identity, tool guidelines, project instructions) is tokenized
    /// once and reused across turns.
    pub fn build_system_blocks(&self) -> Vec<cisco_code_api::SystemBlock> {
        // Build the full prompt once and split at the cache boundary
        let full = self.build();
        let (static_part, dynamic_part) = if let Some(idx) = full.find(CACHE_BOUNDARY) {
            (
                full[..idx].to_string(),
                full[idx + CACHE_BOUNDARY.len()..].to_string(),
            )
        } else {
            (full, String::new())
        };

        let mut blocks = vec![cisco_code_api::SystemBlock {
            text: static_part,
            cache_control: Some(cisco_code_api::CacheControl {
                cache_type: "ephemeral".into(),
            }),
        }];

        if !dynamic_part.is_empty() {
            blocks.push(cisco_code_api::SystemBlock {
                text: dynamic_part,
                cache_control: None,
            });
        }

        blocks
    }

    fn core_section(&self) -> String {
        r#"You are Cisco Code, an AI coding assistant built by Cisco.
You are an interactive agent that helps users with software engineering tasks.
Use the tools available to you to assist the user.

# System
- All text you output outside of tool use is displayed to the user.
- You can use Github-flavored markdown for formatting.
- Tool results and user messages may include system tags — these contain information from the system, not the user.
- When working with tool results, note important information immediately, as results may be cleared later.

# Core Principles
- Read before you write. Understand existing code before suggesting modifications.
- Be precise and concise in your responses. Lead with the answer, not the reasoning.
- Prefer editing existing files over creating new ones.
- Don't add features, refactor code, or make "improvements" beyond what was asked.
- Be careful not to introduce security vulnerabilities (OWASP top 10).
- Only add comments where the logic isn't self-evident. Don't add docstrings to code you didn't change.

# Doing Tasks
- Break complex tasks into smaller steps.
- Use the appropriate tool for each step.
- If an approach fails, diagnose why before switching tactics — don't retry blindly.
- When referencing code, include file_path:line_number patterns.
- Go straight to the point. Try the simplest approach first.

# Executing Actions With Care
- For reversible, local actions (editing files, running tests): proceed freely.
- For hard-to-reverse or shared-state actions (git push, deleting branches, sending messages): confirm with the user first.
- Never skip git hooks (--no-verify) unless the user explicitly asks.
- Prefer new commits over amending existing ones."#
            .to_string()
    }

    fn environment_section(&self) -> String {
        let platform = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());

        let mut env = format!(
            "# Environment\n- Working directory: {cwd}\n- Platform: {platform} ({arch})\n- Shell: {shell}",
            cwd = self.cwd,
        );

        if let Some(ref model) = self.model {
            env.push_str(&format!("\n- Model: {model}"));
        }

        if let Some(ref git) = self.git_context {
            if git.is_repo {
                env.push_str("\n- Git repository: yes");
                if let Some(ref branch) = git.branch {
                    env.push_str(&format!("\n- Git branch: {branch}"));
                }
                if git.has_uncommitted {
                    env.push_str("\n- Uncommitted changes: yes");
                    if let Some(ref summary) = git.status_summary {
                        env.push_str(&format!(" ({summary})"));
                    }
                }
                if !git.recent_commits.is_empty() {
                    env.push_str("\n- Recent commits:");
                    for commit in git.recent_commits.iter().take(5) {
                        env.push_str(&format!("\n  - {commit}"));
                    }
                }
            } else {
                env.push_str("\n- Git repository: no");
            }
        }

        env
    }

    fn tool_guidelines_section(&self) -> String {
        r#"# Tool Usage
- Use Read to read files (not Bash with cat/head/tail).
- Use Edit for precise string replacements in files. Read the file first.
- Use Write only for new files or complete rewrites. Read existing files first.
- Use Grep to search file contents (not Bash with grep/rg).
- Use Glob to find files by pattern (not Bash with find/ls).
- Use Bash for shell commands that have no dedicated tool equivalent.
- Use Agent for complex, multi-step subtasks. Launch multiple agents concurrently when possible.
- For git: never use -i (interactive) flags. Always use HEREDOC for commit messages.
- For file edits: old_string must be unique in the file. Include enough context.
- Never create documentation files unless explicitly requested."#
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Multi-source instruction file discovery
// ---------------------------------------------------------------------------

/// Load project instructions from the full fallback chain.
///
/// Priority per directory: Agents.md > cisco-code.md > CLAUDE.md > AGENTS.md
/// Also loads: CLAUDE.local.md, <config>/rules/*.md, <config>/CLAUDE.md
///
/// Searches: CWD, parent directories (up to MAX_PARENT_WALK), user home.
/// Config dirs searched: .cisco-code/, .claude/, .codex/
pub fn load_project_instructions(cwd: &str) -> Option<String> {
    let files = discover_instruction_files(cwd);
    if files.is_empty() {
        return None;
    }

    let mut combined = String::new();
    let mut total_chars = 0;

    for file in &files {
        let content = &file.content;
        let truncated = if content.len() > MAX_INSTRUCTION_FILE_CHARS {
            // Find a safe UTF-8 char boundary to avoid panicking
            let mut end = MAX_INSTRUCTION_FILE_CHARS;
            while end > 0 && !content.is_char_boundary(end) {
                end -= 1;
            }
            &content[..end]
        } else {
            content.as_str()
        };

        if total_chars + truncated.len() > MAX_TOTAL_INSTRUCTION_CHARS {
            break;
        }

        if !combined.is_empty() {
            combined.push_str("\n\n---\n\n");
        }
        combined.push_str(truncated);
        total_chars += truncated.len();
    }

    Some(combined)
}

/// Discover all instruction files from the multi-source fallback chain.
pub fn discover_instruction_files(cwd: &str) -> Vec<InstructionFile> {
    let mut files = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    // 1. User-level instructions (~/.cisco-code/, ~/.claude/, ~/.codex/)
    if let Some(home) = home_dir() {
        for config_dir in HOME_CONFIG_DIRS {
            let base = home.join(config_dir);

            // <home>/<config>/CLAUDE.md or instructions.md
            for name in &["CLAUDE.md", "instructions.md"] {
                let path = base.join(name);
                if path.exists() && seen_paths.insert(path.clone()) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        files.push(InstructionFile {
                            path,
                            content,
                            source: InstructionSource::User,
                        });
                        break; // Only take first match per config dir
                    }
                }
            }

            // <home>/<config>/rules/*.md
            let rules_dir = base.join("rules");
            if rules_dir.is_dir() {
                load_rules_dir(&rules_dir, InstructionSource::UserRule, &mut files, &mut seen_paths);
            }
        }
    }

    // 2. Walk from CWD upward through parent directories
    let cwd_path = Path::new(cwd).to_path_buf();
    let mut ancestors: Vec<PathBuf> = Vec::new();
    let mut dir = cwd_path.clone();
    ancestors.push(dir.clone());
    for _ in 0..MAX_PARENT_WALK {
        if !dir.pop() {
            break;
        }
        ancestors.push(dir.clone());
    }

    // Process from root down to CWD (so closer dirs override further)
    ancestors.reverse();

    for dir in &ancestors {
        // Primary instruction file (Agents.md > cisco-code.md > CLAUDE.md > AGENTS.md)
        for name in INSTRUCTION_FILES {
            let path = dir.join(name);
            if path.exists() && seen_paths.insert(path.clone()) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    files.push(InstructionFile {
                        path,
                        content,
                        source: InstructionSource::Project,
                    });
                    break; // Only first match per directory
                }
            }
        }

        // Local override (CLAUDE.local.md)
        let local_path = dir.join(LOCAL_OVERRIDE_FILE);
        if local_path.exists() && seen_paths.insert(local_path.clone()) {
            if let Ok(content) = std::fs::read_to_string(&local_path) {
                files.push(InstructionFile {
                    path: local_path,
                    content,
                    source: InstructionSource::Local,
                });
            }
        }

        // Config subdirectories (.cisco-code/, .claude/, .codex/)
        for config_dir in CONFIG_DIRS {
            let config_path = dir.join(config_dir);
            if !config_path.is_dir() {
                continue;
            }

            // <dir>/<config>/CLAUDE.md or instructions.md
            for name in &["CLAUDE.md", "instructions.md"] {
                let path = config_path.join(name);
                if path.exists() && seen_paths.insert(path.clone()) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        files.push(InstructionFile {
                            path,
                            content,
                            source: InstructionSource::Project,
                        });
                        break;
                    }
                }
            }

            // <dir>/<config>/rules/*.md
            let rules_dir = config_path.join("rules");
            if rules_dir.is_dir() {
                load_rules_dir(&rules_dir, InstructionSource::Rule, &mut files, &mut seen_paths);
            }
        }
    }

    files
}

/// Load all .md files from a rules directory.
fn load_rules_dir(
    dir: &Path,
    source: InstructionSource,
    files: &mut Vec<InstructionFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") && seen.insert(path.clone()) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                files.push(InstructionFile {
                    path,
                    content,
                    source: source.clone(),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Memory loading with fallback chain
// ---------------------------------------------------------------------------

/// Load memory content from the fallback chain.
///
/// Searches: .cisco-code/memory/MEMORY.md, .claude/memory/MEMORY.md,
///           ~/.cisco-code/memory/MEMORY.md, ~/.claude/memory/MEMORY.md
pub fn load_memory_content(cwd: &str) -> Option<String> {
    // Project-level memory
    for config_dir in CONFIG_DIRS {
        let path = Path::new(cwd).join(config_dir).join("memory").join("MEMORY.md");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return Some(content);
        }
    }

    // User-level memory
    if let Some(home) = home_dir() {
        for config_dir in HOME_CONFIG_DIRS {
            let path = home.join(config_dir).join("memory").join("MEMORY.md");
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(content);
            }
        }

        // Also check per-project memory under user home (Claude Code pattern)
        // ~/.claude/projects/<sanitized-cwd>/memory/MEMORY.md
        let sanitized = sanitize_path(cwd);
        for config_dir in HOME_CONFIG_DIRS {
            let path = home
                .join(config_dir)
                .join("projects")
                .join(&sanitized)
                .join("memory")
                .join("MEMORY.md");
            if let Ok(content) = std::fs::read_to_string(&path) {
                return Some(content);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Scratchpad
// ---------------------------------------------------------------------------

/// Create a per-session scratchpad directory.
/// Returns the path if successfully created.
pub fn create_scratchpad(session_id: &str) -> Option<String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let base = std::env::var("CISCO_CODE_TMPDIR")
        .unwrap_or_else(|_| format!("/tmp/cisco-code-{user}"));
    let dir = Path::new(&base).join(session_id);
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(dir.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

/// Clean up a scratchpad directory.
pub fn cleanup_scratchpad(path: &str) {
    let _ = std::fs::remove_dir_all(path);
}

// ---------------------------------------------------------------------------
// Skills discovery
// ---------------------------------------------------------------------------

/// Discover available skills from the config directory chain + bundled skills.
///
/// Priority (first match wins per name):
/// 1. Project-level: .cisco-code/skills/, .claude/skills/, .codex/skills/
/// 2. Legacy: .claude/commands/
/// 3. User-level: ~/.cisco-code/skills/, ~/.claude/skills/, ~/.codex/skills/
/// 4. Bundled skills (compiled into the binary)
pub fn discover_skills(cwd: &str) -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    // Project-level skills
    for config_dir in CONFIG_DIRS {
        let skills_dir = Path::new(cwd).join(config_dir).join("skills");
        if skills_dir.is_dir() {
            load_skills_from_dir(&skills_dir, &mut skills, &mut seen_names);
        }
    }

    // Legacy .claude/commands/ directory
    let legacy_dir = Path::new(cwd).join(".claude").join("commands");
    if legacy_dir.is_dir() {
        load_skills_from_dir(&legacy_dir, &mut skills, &mut seen_names);
    }

    // User-level skills
    if let Some(home) = home_dir() {
        for config_dir in HOME_CONFIG_DIRS {
            let skills_dir = home.join(config_dir).join("skills");
            if skills_dir.is_dir() {
                load_skills_from_dir(&skills_dir, &mut skills, &mut seen_names);
            }
        }
    }

    // Bundled skills (lowest priority — user/project skills can override)
    for bundled in load_bundled_skills() {
        if !seen_names.contains(&bundled.name) {
            seen_names.insert(bundled.name.clone());
            skills.push(bundled);
        }
    }

    skills
}

/// Load bundled skills that are compiled into the binary.
///
/// These are the default skills shipped with cisco-code, matching Claude Code's
/// built-in skill set. Users can override any of these by placing a skill with
/// the same name in their project or user skills directory.
pub fn load_bundled_skills() -> Vec<SkillInfo> {
    // Embed skill files at compile time
    static BUNDLED: &[(&str, &str)] = &[
        ("commit", include_str!("../bundled_skills/commit.md")),
        ("code-review", include_str!("../bundled_skills/code-review.md")),
        ("simplify", include_str!("../bundled_skills/simplify.md")),
        ("update-config", include_str!("../bundled_skills/update-config.md")),
        ("remember", include_str!("../bundled_skills/remember.md")),
        ("verify", include_str!("../bundled_skills/verify.md")),
        ("frontend-design", include_str!("../bundled_skills/frontend-design.md")),
        ("claude-api", include_str!("../bundled_skills/claude-api.md")),
        ("loop", include_str!("../bundled_skills/loop.md")),
        ("plan", include_str!("../bundled_skills/plan.md")),
    ];

    BUNDLED
        .iter()
        .filter_map(|(default_name, content)| {
            parse_bundled_skill(default_name, content)
        })
        .collect()
}

/// Parse a bundled skill from its embedded content.
fn parse_bundled_skill(default_name: &str, content: &str) -> Option<SkillInfo> {
    let (frontmatter, body) = if content.starts_with("---") {
        parse_skill_frontmatter(content)
    } else {
        (SkillFrontmatter::default(), content.to_string())
    };

    Some(SkillInfo {
        name: frontmatter.name.unwrap_or_else(|| default_name.to_string()),
        description: frontmatter.description.unwrap_or_default(),
        content: body,
        path: PathBuf::from(format!("<bundled>/{default_name}.md")),
        user_invocable: frontmatter.user_invocable.unwrap_or(true),
        context: frontmatter.context.unwrap_or_default(),
        model: frontmatter.model,
        allowed_tools: frontmatter.allowed_tools,
        bundled: true,
    })
}

/// Look up a skill by name from the full discovery chain.
/// Returns the skill's expanded content if found.
pub fn resolve_skill(cwd: &str, name: &str) -> Option<SkillInfo> {
    discover_skills(cwd)
        .into_iter()
        .find(|s| s.name == name)
}

/// Execution context for a skill — determines how it runs.
///
/// - `Inline` (default): The skill's content is expanded directly into the main
///   conversation context, just like a prompt injection.
/// - `Fork`: The skill executes in an isolated sub-conversation via the Agent tool.
///   The main conversation receives a JSON descriptor that the agent loop routes
///   through the sub-agent infrastructure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillContext {
    /// Expand skill content inline in the current conversation (default).
    Inline,
    /// Execute skill in an isolated sub-agent conversation.
    Fork,
}

impl Default for SkillContext {
    fn default() -> Self {
        Self::Inline
    }
}

impl std::fmt::Display for SkillContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inline => write!(f, "inline"),
            Self::Fork => write!(f, "fork"),
        }
    }
}

impl std::str::FromStr for SkillContext {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "fork" => Ok(Self::Fork),
            "inline" | "" => Ok(Self::Inline),
            other => Err(format!("unknown skill context: '{other}' (expected 'inline' or 'fork')")),
        }
    }
}

/// A discovered skill definition.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub content: String,
    pub path: PathBuf,
    /// Whether the user can invoke this skill directly.
    pub user_invocable: bool,
    /// Execution context: inline (default) or fork (sub-agent).
    pub context: SkillContext,
    /// Optional model override for this skill.
    pub model: Option<String>,
    /// Optional list of allowed tools.
    pub allowed_tools: Option<Vec<String>>,
    /// Whether this is a bundled (compiled-in) skill vs filesystem-discovered.
    pub bundled: bool,
}

/// Load skills from a directory.
/// Skills can be:
/// - A markdown file directly: skill-name.md
/// - A directory with SKILL.md: skill-name/SKILL.md
fn load_skills_from_dir(
    dir: &Path,
    skills: &mut Vec<SkillInfo>,
    seen: &mut std::collections::HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry
            .file_name()
            .to_string_lossy()
            .trim_end_matches(".md")
            .to_string();

        if seen.contains(&name) {
            continue;
        }

        // Directory with SKILL.md inside
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                if let Some(skill) = parse_skill_file(&skill_md, &name) {
                    seen.insert(name);
                    skills.push(skill);
                }
            }
            continue;
        }

        // Direct .md file
        if path.extension().is_some_and(|ext| ext == "md") {
            if let Some(skill) = parse_skill_file(&path, &name) {
                seen.insert(name);
                skills.push(skill);
            }
        }
    }
}

/// Parse a skill file with YAML frontmatter.
fn parse_skill_file(path: &Path, default_name: &str) -> Option<SkillInfo> {
    let content = std::fs::read_to_string(path).ok()?;

    let (frontmatter, body) = if content.starts_with("---") {
        parse_skill_frontmatter(&content)
    } else {
        (SkillFrontmatter::default(), content.clone())
    };

    Some(SkillInfo {
        name: frontmatter.name.unwrap_or_else(|| default_name.to_string()),
        description: frontmatter.description.unwrap_or_default(),
        content: body,
        path: path.to_path_buf(),
        user_invocable: frontmatter.user_invocable.unwrap_or(true),
        context: frontmatter.context.unwrap_or_default(),
        model: frontmatter.model,
        allowed_tools: frontmatter.allowed_tools,
        bundled: false,
    })
}

#[derive(Debug, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    user_invocable: Option<bool>,
    context: Option<SkillContext>,
    model: Option<String>,
    allowed_tools: Option<Vec<String>>,
}

fn parse_skill_frontmatter(content: &str) -> (SkillFrontmatter, String) {
    let rest = &content[3..];
    let end = match rest.find("---") {
        Some(idx) => idx,
        None => return (SkillFrontmatter::default(), content.to_string()),
    };

    let yaml = &rest[..end];
    let body = rest[end + 3..].trim().to_string();

    let mut fm = SkillFrontmatter::default();
    for line in yaml.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            fm.name = Some(val.trim().trim_matches('"').to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            fm.description = Some(val.trim().trim_matches('"').to_string());
        } else if let Some(val) = line.strip_prefix("user-invocable:") {
            fm.user_invocable = val.trim().parse().ok();
        } else if let Some(val) = line.strip_prefix("context:") {
            fm.context = val.trim().trim_matches('"').parse().ok();
        } else if let Some(val) = line.strip_prefix("model:") {
            fm.model = Some(val.trim().trim_matches('"').to_string());
        } else if let Some(val) = line.strip_prefix("allowed-tools:") {
            let tools: Vec<String> = val
                .trim()
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !tools.is_empty() {
                fm.allowed_tools = Some(tools);
            }
        }
    }

    (fm, body)
}

// ---------------------------------------------------------------------------
// Git context detection
// ---------------------------------------------------------------------------

/// Detect git context for the current working directory.
///
/// Uses `git` commands (blocking) to get branch, status, and recent commits.
/// Falls back gracefully if git is not installed or the directory is not a repo.
pub fn detect_git_context(cwd: &str) -> GitContext {
    use std::process::{Command, Stdio};

    let not_a_repo = GitContext {
        is_repo: false,
        branch: None,
        has_uncommitted: false,
        status_summary: None,
        recent_commits: Vec::new(),
    };

    // Use git rev-parse to check if we're in a repo (handles subdirectories)
    let in_repo = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !in_repo {
        return not_a_repo;
    }

    // Get branch name
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    // Get status (porcelain format for machine parsing)
    let (has_uncommitted, status_summary) = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            let output = String::from_utf8_lossy(&o.stdout);
            let lines: Vec<&str> = output.lines().filter(|l| !l.is_empty()).collect();
            if lines.is_empty() {
                (false, None)
            } else {
                // Count by first two status chars (XY format)
                let modified = lines.iter().filter(|l| {
                    let b = l.as_bytes();
                    b.len() >= 2 && (b[0] == b'M' || b[1] == b'M')
                }).count();
                let added = lines.iter().filter(|l| {
                    let b = l.as_bytes();
                    (b.len() >= 2 && b[0] == b'A') || l.starts_with("??")
                }).count();
                let deleted = lines.iter().filter(|l| {
                    let b = l.as_bytes();
                    b.len() >= 2 && (b[0] == b'D' || b[1] == b'D')
                }).count();
                let renamed = lines.iter().filter(|l| {
                    let b = l.as_bytes();
                    b.len() >= 2 && b[0] == b'R'
                }).count();
                let mut parts = Vec::new();
                if modified > 0 { parts.push(format!("{modified} modified")); }
                if added > 0 { parts.push(format!("{added} added")); }
                if deleted > 0 { parts.push(format!("{deleted} deleted")); }
                if renamed > 0 { parts.push(format!("{renamed} renamed")); }
                let summary = if parts.is_empty() {
                    format!("{} changes", lines.len())
                } else {
                    parts.join(", ")
                };
                (true, Some(summary))
            }
        })
        .unwrap_or((false, None));

    // Get recent commits
    let recent_commits = Command::new("git")
        .args(["log", "--oneline", "-5"])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default();

    GitContext {
        is_repo: true,
        branch,
        has_uncommitted,
        status_summary,
        recent_commits,
    }
}

// ---------------------------------------------------------------------------
// Settings loading with fallback chain
// ---------------------------------------------------------------------------

/// Load settings.json from the config directory chain.
/// Returns the first valid JSON found as a serde_json::Value.
///
/// Searches: .cisco-code/settings.json, .claude/settings.json, .codex/settings.json,
///           ~/.cisco-code/settings.json, ~/.claude/settings.json, ~/.codex/settings.json
pub fn load_settings(cwd: &str) -> Option<serde_json::Value> {
    // Project-level settings (merged)
    let mut merged: Option<serde_json::Value> = None;

    for config_dir in CONFIG_DIRS {
        let path = Path::new(cwd).join(config_dir).join("settings.json");
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                merged = Some(match merged {
                    Some(base) => merge_json(base, val),
                    None => val,
                });
            }
        }

        // Also check settings.local.json
        let local_path = Path::new(cwd).join(config_dir).join("settings.local.json");
        if let Ok(content) = std::fs::read_to_string(&local_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                merged = Some(match merged {
                    Some(base) => merge_json(base, val),
                    None => val,
                });
            }
        }
    }

    // User-level settings
    if let Some(home) = home_dir() {
        for config_dir in HOME_CONFIG_DIRS {
            let path = home.join(config_dir).join("settings.json");
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    // User settings are lower priority — only set if no project settings
                    merged = Some(match merged {
                        Some(base) => merge_json(val, base), // project wins
                        None => val,
                    });
                }
            }
        }
    }

    merged
}

/// Shallow merge two JSON objects (second wins on conflict).
fn merge_json(base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    match (base, overlay) {
        (serde_json::Value::Object(mut base_map), serde_json::Value::Object(overlay_map)) => {
            for (key, val) in overlay_map {
                base_map.insert(key, val);
            }
            serde_json::Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

// ---------------------------------------------------------------------------
// Todo context
// ---------------------------------------------------------------------------

/// Load the current todo list as formatted context for the system prompt.
pub fn load_todo_context(cwd: &str) -> Option<String> {
    let path = Path::new(cwd).join(".cisco-code/todos.json");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    let todos: Vec<serde_json::Value> = serde_json::from_str(&content).ok()?;
    if todos.is_empty() {
        return None;
    }

    let text: Vec<String> = todos
        .iter()
        .filter_map(|t| {
            let status = t["status"].as_str().unwrap_or("pending");
            let content = t["content"].as_str()?;
            let marker = if status == "done" { "x" } else { " " };
            let priority = t["priority"]
                .as_u64()
                .map(|p| format!(" (P{p})"))
                .unwrap_or_default();
            Some(format!("- [{marker}] {content}{priority}"))
        })
        .collect();

    Some(text.join("\n"))
}

// (Async detect_git_context removed — the sync version above handles
// all git detection including status_summary and recent_commits.)

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Sanitize a path for use as a directory name (matching Claude Code's approach).
fn sanitize_path(path: &str) -> String {
    path.replace('/', "-")
        .replace('\\', "-")
        .trim_start_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prompt_builder_basic() {
        let prompt = PromptBuilder::new("/tmp/test").build();
        assert!(prompt.contains("Cisco Code"));
        assert!(prompt.contains("/tmp/test"));
        assert!(prompt.contains("Tool Usage"));
    }

    #[test]
    fn test_prompt_builder_with_instructions() {
        let prompt = PromptBuilder::new("/tmp")
            .with_instructions("Always use snake_case.")
            .build();
        assert!(prompt.contains("Project Instructions"));
        assert!(prompt.contains("Always use snake_case."));
    }

    #[test]
    fn test_prompt_builder_without_instructions() {
        let prompt = PromptBuilder::new("/tmp").build();
        assert!(!prompt.contains("Project Instructions"));
    }

    #[test]
    fn test_prompt_contains_environment_info() {
        let prompt = PromptBuilder::new("/workspace").build();
        assert!(prompt.contains("Working directory: /workspace"));
        assert!(prompt.contains("Platform:"));
    }

    #[test]
    fn test_prompt_contains_cache_boundary() {
        let prompt = PromptBuilder::new("/tmp").build();
        assert!(prompt.contains("SYSTEM_PROMPT_DYNAMIC_BOUNDARY"));
    }

    #[test]
    fn test_prompt_with_scratchpad() {
        let prompt = PromptBuilder::new("/tmp")
            .with_scratchpad("/tmp/cisco-code-1000/abc123")
            .build();
        assert!(prompt.contains("Scratchpad"));
        assert!(prompt.contains("/tmp/cisco-code-1000/abc123"));
    }

    #[test]
    fn test_prompt_with_skills() {
        let prompt = PromptBuilder::new("/tmp")
            .with_skills("- commit: Create a git commit\n- review: Code review")
            .build();
        assert!(prompt.contains("Available Skills"));
        assert!(prompt.contains("commit"));
    }

    #[test]
    fn test_build_static_and_dynamic() {
        let builder = PromptBuilder::new("/tmp")
            .with_instructions("test instructions");
        let static_part = builder.build_static();
        let dynamic_part = builder.build_dynamic();

        assert!(static_part.contains("test instructions"));
        assert!(dynamic_part.contains("Environment"));
        assert!(!static_part.contains("SYSTEM_PROMPT_DYNAMIC_BOUNDARY"));
    }

    #[test]
    fn test_load_project_instructions_agents_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Agents.md"), "Agent rules").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert_eq!(result, Some("Agent rules".to_string()));
    }

    #[test]
    fn test_load_project_instructions_cisco_code_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cisco-code.md"), "Custom rules here").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert_eq!(result, Some("Custom rules here".to_string()));
    }

    #[test]
    fn test_load_project_instructions_claude_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "Claude rules").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert_eq!(result, Some("Claude rules".to_string()));
    }

    #[test]
    fn test_load_project_instructions_priority() {
        let dir = tempfile::tempdir().unwrap();
        // Agents.md takes priority over everything
        std::fs::write(dir.path().join("Agents.md"), "agents").unwrap();
        std::fs::write(dir.path().join("cisco-code.md"), "cisco").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "claude").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert!(result.unwrap().contains("agents"));
    }

    #[test]
    fn test_load_project_instructions_none() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert!(result.is_none());
    }

    #[test]
    fn test_load_project_instructions_with_rules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "Base rules").unwrap();
        let rules_dir = dir.path().join(".cisco-code").join("rules");
        std::fs::create_dir_all(&rules_dir).unwrap();
        std::fs::write(rules_dir.join("no-python.md"), "Never use Python").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        let content = result.unwrap();
        assert!(content.contains("Base rules"));
        assert!(content.contains("Never use Python"));
    }

    #[test]
    fn test_load_project_instructions_local_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "Shared rules").unwrap();
        std::fs::write(dir.path().join("CLAUDE.local.md"), "My local tweaks").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        let content = result.unwrap();
        assert!(content.contains("Shared rules"));
        assert!(content.contains("My local tweaks"));
    }

    #[test]
    fn test_load_project_instructions_config_dir_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // Only .claude/CLAUDE.md exists (fallback from .cisco-code/)
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("CLAUDE.md"), "From .claude dir").unwrap();

        let result = load_project_instructions(&dir.path().to_string_lossy());
        assert!(result.unwrap().contains("From .claude dir"));
    }

    #[test]
    fn test_discover_instruction_files_dedup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "rules").unwrap();

        let files = discover_instruction_files(&dir.path().to_string_lossy());
        // Should not have duplicates
        let paths: Vec<_> = files.iter().map(|f| &f.path).collect();
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(paths.len(), unique.len());
    }

    #[test]
    fn test_sanitize_path() {
        assert_eq!(sanitize_path("/Users/user/project"), "Users-user-project");
        assert_eq!(sanitize_path("C:\\Users\\project"), "C:-Users-project");
    }

    #[test]
    fn test_discover_skills_no_filesystem_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skills = discover_skills(&dir.path().to_string_lossy());
        // No filesystem skills, but bundled skills are always included
        let bundled_count = load_bundled_skills().len();
        assert_eq!(skills.len(), bundled_count);
    }

    #[test]
    fn test_discover_skills_md_file() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".cisco-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("deploy.md"),
            "---\nname: deploy\ndescription: Deploy to production\n---\n\nRun the deploy pipeline.",
        )
        .unwrap();

        let skills = discover_skills(&dir.path().to_string_lossy());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "deploy");
        assert_eq!(skills[0].description, "Deploy to production");
        assert!(skills[0].content.contains("deploy pipeline"));
    }

    #[test]
    fn test_discover_skills_directory() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(".cisco-code").join("skills").join("lint");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: lint\ndescription: Run linters\nuser-invocable: true\n---\n\nRun all linters.",
        )
        .unwrap();

        let skills = discover_skills(&dir.path().to_string_lossy());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "lint");
        assert!(skills[0].user_invocable);
    }

    #[test]
    fn test_discover_skills_claude_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // Only .claude/skills/ exists (fallback)
        let skills_dir = dir.path().join(".claude").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("review.md"), "Review code changes").unwrap();

        let skills = discover_skills(&dir.path().to_string_lossy());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "review");
    }

    #[test]
    fn test_load_memory_content_cisco_code() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join(".cisco-code").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), "- [User](user.md)").unwrap();

        let result = load_memory_content(&dir.path().to_string_lossy());
        assert!(result.unwrap().contains("User"));
    }

    #[test]
    fn test_load_memory_content_claude_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // Only .claude/memory/ exists
        let mem_dir = dir.path().join(".claude").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), "- [From Claude](c.md)").unwrap();

        let result = load_memory_content(&dir.path().to_string_lossy());
        assert!(result.unwrap().contains("From Claude"));
    }

    #[test]
    fn test_git_context_detection() {
        let ctx = detect_git_context("/nonexistent/path");
        assert!(!ctx.is_repo);
    }

    #[test]
    fn test_parse_skill_frontmatter() {
        let content = "---\nname: test\ndescription: A test skill\nuser-invocable: false\nmodel: \"haiku\"\nallowed-tools: [\"Read\", \"Grep\"]\n---\n\nDo the thing.";
        let (fm, body) = parse_skill_frontmatter(content);
        assert_eq!(fm.name, Some("test".to_string()));
        assert_eq!(fm.description, Some("A test skill".to_string()));
        assert_eq!(fm.user_invocable, Some(false));
        assert_eq!(fm.model, Some("haiku".to_string()));
        assert_eq!(fm.allowed_tools, Some(vec!["Read".to_string(), "Grep".to_string()]));
        assert_eq!(body, "Do the thing.");
    }

    #[test]
    fn test_parse_skill_no_frontmatter() {
        let content = "Just a plain markdown file.";
        let (fm, body) = parse_skill_frontmatter(content);
        assert!(fm.name.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_merge_json() {
        let base = serde_json::json!({"a": 1, "b": 2});
        let overlay = serde_json::json!({"b": 3, "c": 4});
        let merged = merge_json(base, overlay);
        assert_eq!(merged["a"], 1);
        assert_eq!(merged["b"], 3); // overlay wins
        assert_eq!(merged["c"], 4);
    }

    #[test]
    fn test_load_bundled_skills() {
        let skills = load_bundled_skills();
        assert!(skills.len() >= 10, "Expected at least 10 bundled skills, got {}", skills.len());

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"code-review"));
        assert!(names.contains(&"simplify"));
        assert!(names.contains(&"update-config"));
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"verify"));
        assert!(names.contains(&"frontend-design"));
        assert!(names.contains(&"claude-api"));
        assert!(names.contains(&"loop"));
        assert!(names.contains(&"plan"));
    }

    #[test]
    fn test_bundled_skills_are_user_invocable() {
        let skills = load_bundled_skills();
        for skill in &skills {
            assert!(skill.user_invocable, "Bundled skill '{}' should be user-invocable", skill.name);
        }
    }

    #[test]
    fn test_bundled_skills_have_descriptions() {
        let skills = load_bundled_skills();
        for skill in &skills {
            assert!(
                !skill.description.is_empty(),
                "Bundled skill '{}' should have a description",
                skill.name
            );
        }
    }

    #[test]
    fn test_bundled_skills_have_content() {
        let skills = load_bundled_skills();
        for skill in &skills {
            assert!(
                !skill.content.is_empty(),
                "Bundled skill '{}' should have content",
                skill.name
            );
        }
    }

    #[test]
    fn test_bundled_skills_marked_as_bundled() {
        let skills = load_bundled_skills();
        for skill in &skills {
            assert!(skill.bundled, "Skill '{}' should be marked as bundled", skill.name);
        }
    }

    #[test]
    fn test_discover_skills_includes_bundled() {
        let dir = tempfile::tempdir().unwrap();
        // No filesystem skills — should still get bundled ones
        let skills = discover_skills(&dir.path().to_string_lossy());
        assert!(skills.len() >= 10, "Should include bundled skills even with no filesystem skills");

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"verify"));
    }

    #[test]
    fn test_filesystem_skills_override_bundled() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".cisco-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        // Create a filesystem skill that overrides the bundled "commit" skill
        std::fs::write(
            skills_dir.join("commit.md"),
            "---\nname: commit\ndescription: Custom commit skill\n---\n\nMy custom commit flow.",
        )
        .unwrap();

        let skills = discover_skills(&dir.path().to_string_lossy());
        let commit = skills.iter().find(|s| s.name == "commit").unwrap();
        assert_eq!(commit.description, "Custom commit skill");
        assert!(commit.content.contains("My custom commit flow"));
        assert!(!commit.bundled, "Filesystem skill should override bundled");
    }

    #[test]
    fn test_resolve_skill_bundled() {
        let dir = tempfile::tempdir().unwrap();
        let skill = resolve_skill(&dir.path().to_string_lossy(), "commit");
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.name, "commit");
        assert!(skill.bundled);
    }

    #[test]
    fn test_resolve_skill_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let skill = resolve_skill(&dir.path().to_string_lossy(), "nonexistent-skill-xyz");
        assert!(skill.is_none());
    }

    #[test]
    fn test_build_system_blocks() {
        let builder = PromptBuilder::new("/tmp/test")
            .with_instructions("My project rules");
        let blocks = builder.build_system_blocks();

        assert_eq!(blocks.len(), 2, "Expected 2 blocks (static + dynamic)");

        // First block: static, cached
        assert!(blocks[0].text.contains("Cisco Code"));
        assert!(blocks[0].text.contains("My project rules"));
        assert!(blocks[0].cache_control.is_some());
        assert_eq!(blocks[0].cache_control.as_ref().unwrap().cache_type, "ephemeral");

        // Second block: dynamic, uncached
        assert!(blocks[1].text.contains("Environment"));
        assert!(blocks[1].cache_control.is_none());
    }

    #[test]
    fn test_build_system_blocks_dynamic_content() {
        let builder = PromptBuilder::new("/tmp/test")
            .with_todos("- [ ] Fix bug\n- [x] Write tests");
        let blocks = builder.build_system_blocks();

        // Todos should be in the dynamic (uncached) block
        assert!(blocks.len() >= 2);
        let dynamic = &blocks[1];
        assert!(dynamic.text.contains("Fix bug"));
        assert!(dynamic.text.contains("Write tests"));
        assert!(dynamic.cache_control.is_none());
    }

    #[test]
    fn test_prompt_with_todos() {
        let prompt = PromptBuilder::new("/tmp")
            .with_todos("- [ ] Fix bug\n- [x] Write tests")
            .build();
        assert!(prompt.contains("Current Todos"));
        assert!(prompt.contains("Fix bug"));
    }

    #[test]
    fn test_prompt_with_mcp_instructions() {
        let prompt = PromptBuilder::new("/tmp")
            .with_mcp_instructions("## GitHub\nUse the GitHub MCP server for PR operations.")
            .build();
        assert!(prompt.contains("MCP Server Instructions"));
        assert!(prompt.contains("GitHub MCP server"));
    }

    #[test]
    fn test_load_todo_context_with_todos() {
        let dir = tempfile::tempdir().unwrap();
        let cisco_dir = dir.path().join(".cisco-code");
        std::fs::create_dir_all(&cisco_dir).unwrap();
        std::fs::write(
            cisco_dir.join("todos.json"),
            r#"[
                {"id":"1","content":"Fix auth bug","status":"pending"},
                {"id":"2","content":"Add tests","status":"in_progress"},
                {"id":"3","content":"Deploy","status":"done"}
            ]"#,
        ).unwrap();

        let result = load_todo_context(&dir.path().to_string_lossy());
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("Fix auth bug"));
        assert!(text.contains("Add tests"));
        assert!(text.contains("Deploy"));
        // Done items should have [x], others [ ]
        assert!(text.contains("[x]"));
        assert!(text.contains("[ ]"));
    }

    #[test]
    fn test_load_todo_context_empty() {
        let dir = tempfile::tempdir().unwrap();
        let cisco_dir = dir.path().join(".cisco-code");
        std::fs::create_dir_all(&cisco_dir).unwrap();
        std::fs::write(cisco_dir.join("todos.json"), "[]").unwrap();

        let result = load_todo_context(&dir.path().to_string_lossy());
        assert!(result.is_none());
    }

    #[test]
    fn test_load_todo_context_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_todo_context(&dir.path().to_string_lossy());
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // SkillContext tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_skill_context_default_is_inline() {
        assert_eq!(SkillContext::default(), SkillContext::Inline);
    }

    #[test]
    fn test_skill_context_display() {
        assert_eq!(SkillContext::Inline.to_string(), "inline");
        assert_eq!(SkillContext::Fork.to_string(), "fork");
    }

    #[test]
    fn test_skill_context_from_str() {
        assert_eq!("inline".parse::<SkillContext>().unwrap(), SkillContext::Inline);
        assert_eq!("fork".parse::<SkillContext>().unwrap(), SkillContext::Fork);
        assert_eq!("FORK".parse::<SkillContext>().unwrap(), SkillContext::Fork);
        assert_eq!("Inline".parse::<SkillContext>().unwrap(), SkillContext::Inline);
        assert_eq!("".parse::<SkillContext>().unwrap(), SkillContext::Inline);
        assert!("unknown".parse::<SkillContext>().is_err());
    }

    #[test]
    fn test_parse_skill_frontmatter_with_context_fork() {
        let content = "---\nname: deploy\ndescription: Deploy to prod\ncontext: fork\nmodel: \"sonnet\"\nallowed-tools: [Read, Grep]\n---\n\nDeploy the application.";
        let (fm, body) = parse_skill_frontmatter(content);
        assert_eq!(fm.name, Some("deploy".to_string()));
        assert_eq!(fm.context, Some(SkillContext::Fork));
        assert_eq!(fm.model, Some("sonnet".to_string()));
        assert_eq!(fm.allowed_tools, Some(vec!["Read".to_string(), "Grep".to_string()]));
        assert_eq!(body, "Deploy the application.");
    }

    #[test]
    fn test_parse_skill_frontmatter_with_context_inline() {
        let content = "---\nname: review\ncontext: inline\n---\n\nReview code.";
        let (fm, _body) = parse_skill_frontmatter(content);
        assert_eq!(fm.context, Some(SkillContext::Inline));
    }

    #[test]
    fn test_parse_skill_frontmatter_without_context() {
        let content = "---\nname: review\ndescription: Review code\n---\n\nReview it.";
        let (fm, _body) = parse_skill_frontmatter(content);
        assert_eq!(fm.context, None); // None means default (Inline)
    }

    #[test]
    fn test_skill_info_context_default_for_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".cisco-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        // Skill without context field -> should default to Inline
        std::fs::write(
            skills_dir.join("test-skill.md"),
            "---\nname: test-skill\ndescription: A test\n---\n\nDo stuff.",
        )
        .unwrap();

        let skill = resolve_skill(&dir.path().to_string_lossy(), "test-skill").unwrap();
        assert_eq!(skill.context, SkillContext::Inline);
    }

    #[test]
    fn test_skill_info_context_fork_for_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".cisco-code").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("deploy.md"),
            "---\nname: deploy\ndescription: Deploy to prod\ncontext: fork\nmodel: sonnet\nallowed-tools: [Read, Grep, Bash]\n---\n\nRun deploy.",
        )
        .unwrap();

        let skill = resolve_skill(&dir.path().to_string_lossy(), "deploy").unwrap();
        assert_eq!(skill.context, SkillContext::Fork);
        assert_eq!(skill.model, Some("sonnet".to_string()));
        assert_eq!(
            skill.allowed_tools,
            Some(vec!["Read".to_string(), "Grep".to_string(), "Bash".to_string()])
        );
    }

    #[test]
    fn test_bundled_skills_default_to_inline_context() {
        let skills = load_bundled_skills();
        for skill in &skills {
            assert_eq!(
                skill.context,
                SkillContext::Inline,
                "Bundled skill '{}' should default to Inline context",
                skill.name,
            );
        }
    }
}
