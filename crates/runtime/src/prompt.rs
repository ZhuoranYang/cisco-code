//! System prompt builder.
//!
//! Design insight from Astro-Assistant: Layered prompt assembly:
//! 1. SYSTEM — core agent instructions
//! 2. CONTEXT — git status, environment, repo structure
//! 3. INSTRUCTIONS — project-specific (CLAUDE.md / cisco-code.md)
//!
//! Design insight from Claude Code: The system prompt is assembled at each turn,
//! not fixed — it can incorporate dynamic context like tool results, environment
//! state, and mid-conversation reminders.

use std::path::Path;

/// Builds the system prompt from layered sections.
pub struct PromptBuilder {
    cwd: String,
    custom_instructions: Option<String>,
}

impl PromptBuilder {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            custom_instructions: None,
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.custom_instructions = Some(instructions.into());
        self
    }

    /// Build the full system prompt.
    pub fn build(&self) -> String {
        let mut sections = Vec::new();

        // Core agent identity and behavior
        sections.push(self.core_section());

        // Environment context
        sections.push(self.environment_section());

        // Tool usage guidelines
        sections.push(self.tool_guidelines_section());

        // Project-specific instructions
        if let Some(ref instructions) = self.custom_instructions {
            sections.push(format!("# Project Instructions\n\n{instructions}"));
        }

        sections.join("\n\n")
    }

    fn core_section(&self) -> String {
        r#"You are Cisco Code, an AI coding assistant built for Cisco engineers.
You are an interactive agent that helps users with software engineering tasks.
Use the tools available to you to assist the user.

# Core Principles
- Read before you write. Understand existing code before modifying it.
- Be precise and concise in your responses.
- Prefer editing existing files over creating new ones.
- Don't add features, refactor code, or make "improvements" beyond what was asked.
- Be careful not to introduce security vulnerabilities.

# Doing Tasks
- Break complex tasks into smaller steps.
- Use the appropriate tool for each step.
- If an approach fails, diagnose why before switching tactics.
- When referencing code, include file paths and line numbers."#
            .to_string()
    }

    fn environment_section(&self) -> String {
        let platform = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        format!(
            r#"# Environment
- Working directory: {cwd}
- Platform: {platform} ({arch})
- Shell: {shell}"#,
            cwd = self.cwd,
            shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into()),
        )
    }

    fn tool_guidelines_section(&self) -> String {
        r#"# Tool Usage
- Use Read to read files (not Bash with cat/head/tail).
- Use Edit for precise string replacements in files.
- Use Write only for new files or complete rewrites.
- Use Grep to search file contents (not Bash with grep/rg).
- Use Glob to find files by pattern (not Bash with find/ls).
- Use Bash for shell commands that have no dedicated tool equivalent."#
            .to_string()
    }
}

/// Try to load project-specific instructions from a config file.
pub fn load_project_instructions(cwd: &str) -> Option<String> {
    // Check for cisco-code.md, CLAUDE.md, or .cisco-code/instructions.md
    let candidates = [
        "cisco-code.md",
        "CLAUDE.md",
        ".cisco-code/instructions.md",
    ];

    for candidate in &candidates {
        let path = Path::new(cwd).join(candidate);
        if let Ok(content) = std::fs::read_to_string(&path) {
            return Some(content);
        }
    }

    None
}
