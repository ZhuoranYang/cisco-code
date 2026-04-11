//! Slash command system.
//!
//! Slash commands are shortcuts that expand into prompts or execute direct actions.
//! They let users trigger common workflows without typing full prompts.
//!
//! Types:
//! - Prompt commands: expand to a prompt fed to the agent (e.g., /commit, /review)
//! - Action commands: execute directly without the agent loop (e.g., /clear, /model)
//!
//! Custom commands can be defined in .cisco-code/commands.toml.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A slash command definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashCommand {
    /// The command name (without the "/" prefix).
    pub name: String,
    /// Short description shown in /help.
    pub description: String,
    /// The type of command.
    pub kind: CommandKind,
}

/// What a command does when invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CommandKind {
    /// Expands into a prompt that gets sent to the agent.
    #[serde(rename = "prompt")]
    Prompt {
        /// The prompt template. {{args}} is replaced with user-provided arguments.
        template: String,
    },
    /// A built-in action handled by the CLI/runtime directly.
    #[serde(rename = "action")]
    Action {
        /// The action identifier.
        action: String,
    },
}

/// Result of parsing a slash command invocation.
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// Not a command (regular user input).
    NotACommand,
    /// Expands to a prompt — send this to the agent.
    ExpandedPrompt(String),
    /// A built-in action — handle directly.
    BuiltinAction {
        action: String,
        args: String,
    },
    /// Unknown command.
    Unknown(String),
}

/// Registry of available slash commands.
pub struct CommandRegistry {
    commands: HashMap<String, SlashCommand>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Create a registry with all built-in commands.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();

        // --- Prompt commands (expand to agent prompts) ---

        registry.register(SlashCommand {
            name: "commit".into(),
            description: "Create a git commit with AI-generated message".into(),
            kind: CommandKind::Prompt {
                template: "Look at the current git diff (staged and unstaged changes). Create a well-crafted git commit. Follow conventional commit format. Stage relevant files and commit. {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "review".into(),
            description: "Review code changes or a PR".into(),
            kind: CommandKind::Prompt {
                template: "Review the following code changes for bugs, security issues, and improvements. Be specific and actionable. {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "search".into(),
            description: "Search the codebase".into(),
            kind: CommandKind::Prompt {
                template: "Search the codebase thoroughly for: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "fix".into(),
            description: "Fix a bug or issue".into(),
            kind: CommandKind::Prompt {
                template: "Diagnose and fix the following issue: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "explain".into(),
            description: "Explain code or a concept".into(),
            kind: CommandKind::Prompt {
                template: "Explain the following in detail: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "refactor".into(),
            description: "Refactor code".into(),
            kind: CommandKind::Prompt {
                template: "Refactor the following code. Keep the same behavior but improve readability, structure, and maintainability: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "test".into(),
            description: "Write or run tests".into(),
            kind: CommandKind::Prompt {
                template: "Write comprehensive tests for: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "pr".into(),
            description: "Create a pull request".into(),
            kind: CommandKind::Prompt {
                template: "Create a pull request for the current branch. Write a clear title and description summarizing the changes. {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "doc".into(),
            description: "Generate documentation".into(),
            kind: CommandKind::Prompt {
                template: "Generate documentation for: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "security-review".into(),
            description: "Security-focused code review".into(),
            kind: CommandKind::Prompt {
                template: "Perform a thorough security review of the following code. Focus on OWASP top 10 vulnerabilities, injection attacks, authentication/authorization issues, data exposure, and insecure configurations: {{args}}".into(),
            },
        });

        registry.register(SlashCommand {
            name: "add-dir".into(),
            description: "Add a directory to the context".into(),
            kind: CommandKind::Prompt {
                template: "Add the directory {{args}} to the working context. Read its structure and key files.".into(),
            },
        });

        // --- Built-in action commands ---

        registry.register(SlashCommand {
            name: "help".into(),
            description: "Show available commands".into(),
            kind: CommandKind::Action { action: "help".into() },
        });

        registry.register(SlashCommand {
            name: "model".into(),
            description: "Show or switch the current model".into(),
            kind: CommandKind::Action { action: "model".into() },
        });

        registry.register(SlashCommand {
            name: "usage".into(),
            description: "Show token usage for this session".into(),
            kind: CommandKind::Action { action: "usage".into() },
        });

        registry.register(SlashCommand {
            name: "cost".into(),
            description: "Show session cost breakdown".into(),
            kind: CommandKind::Action { action: "cost".into() },
        });

        registry.register(SlashCommand {
            name: "status".into(),
            description: "Show session status (model, tokens, tools, cwd)".into(),
            kind: CommandKind::Action { action: "status".into() },
        });

        registry.register(SlashCommand {
            name: "clear".into(),
            description: "Clear conversation history".into(),
            kind: CommandKind::Action { action: "clear".into() },
        });

        registry.register(SlashCommand {
            name: "config".into(),
            description: "Show or modify configuration".into(),
            kind: CommandKind::Action { action: "config".into() },
        });

        registry.register(SlashCommand {
            name: "theme".into(),
            description: "Switch color theme".into(),
            kind: CommandKind::Action { action: "theme".into() },
        });

        registry.register(SlashCommand {
            name: "diff".into(),
            description: "Show git diff of changes in this session".into(),
            kind: CommandKind::Action { action: "diff".into() },
        });

        registry.register(SlashCommand {
            name: "resume".into(),
            description: "Resume a previous session".into(),
            kind: CommandKind::Action { action: "resume".into() },
        });

        registry.register(SlashCommand {
            name: "memory".into(),
            description: "Show or manage CLAUDE.md / memory files".into(),
            kind: CommandKind::Action { action: "memory".into() },
        });

        registry.register(SlashCommand {
            name: "mcp".into(),
            description: "Show MCP server status and tools".into(),
            kind: CommandKind::Action { action: "mcp".into() },
        });

        registry.register(SlashCommand {
            name: "doctor".into(),
            description: "Run diagnostic checks".into(),
            kind: CommandKind::Action { action: "doctor".into() },
        });

        registry.register(SlashCommand {
            name: "version".into(),
            description: "Show cisco-code version".into(),
            kind: CommandKind::Action { action: "version".into() },
        });

        registry.register(SlashCommand {
            name: "export".into(),
            description: "Export conversation to file".into(),
            kind: CommandKind::Action { action: "export".into() },
        });

        registry.register(SlashCommand {
            name: "context".into(),
            description: "Show current context window usage".into(),
            kind: CommandKind::Action { action: "context".into() },
        });

        registry.register(SlashCommand {
            name: "permissions".into(),
            description: "Show or change permission mode".into(),
            kind: CommandKind::Action { action: "permissions".into() },
        });

        registry.register(SlashCommand {
            name: "branch".into(),
            description: "Show current git branch".into(),
            kind: CommandKind::Action { action: "branch".into() },
        });

        registry.register(SlashCommand {
            name: "effort".into(),
            description: "Set reasoning effort level (low/medium/high)".into(),
            kind: CommandKind::Action { action: "effort".into() },
        });

        registry.register(SlashCommand {
            name: "vim".into(),
            description: "Toggle vim keybindings".into(),
            kind: CommandKind::Action { action: "vim".into() },
        });

        registry.register(SlashCommand {
            name: "fast".into(),
            description: "Toggle fast output mode".into(),
            kind: CommandKind::Action { action: "fast".into() },
        });

        registry.register(SlashCommand {
            name: "compact".into(),
            description: "Force context compaction now".into(),
            kind: CommandKind::Action { action: "compact".into() },
        });

        registry.register(SlashCommand {
            name: "login".into(),
            description: "Authenticate with a provider".into(),
            kind: CommandKind::Action { action: "login".into() },
        });

        registry.register(SlashCommand {
            name: "logout".into(),
            description: "Clear stored credentials".into(),
            kind: CommandKind::Action { action: "logout".into() },
        });

        registry.register(SlashCommand {
            name: "tasks".into(),
            description: "Show current task list".into(),
            kind: CommandKind::Action { action: "tasks".into() },
        });

        registry.register(SlashCommand {
            name: "plan".into(),
            description: "Toggle plan mode (design before implementation)".into(),
            kind: CommandKind::Action { action: "plan".into() },
        });

        registry.register(SlashCommand {
            name: "quit".into(),
            description: "Exit cisco-code".into(),
            kind: CommandKind::Action { action: "quit".into() },
        });

        registry.register(SlashCommand {
            name: "exit".into(),
            description: "Exit cisco-code".into(),
            kind: CommandKind::Action { action: "quit".into() },
        });

        registry.register(SlashCommand {
            name: "q".into(),
            description: "Exit cisco-code".into(),
            kind: CommandKind::Action { action: "quit".into() },
        });

        registry
    }

    /// Register a command.
    pub fn register(&mut self, command: SlashCommand) {
        self.commands.insert(command.name.clone(), command);
    }

    /// Load custom commands from a TOML file and add them.
    pub fn load_custom(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let content = std::fs::read_to_string(path)?;
        let custom: CustomCommands = toml::from_str(&content)?;
        for cmd in custom.commands {
            self.commands.insert(cmd.name.clone(), cmd);
        }
        Ok(())
    }

    /// Parse user input and resolve it against registered commands.
    pub fn parse(&self, input: &str) -> CommandResult {
        let input = input.trim();

        if !input.starts_with('/') {
            return CommandResult::NotACommand;
        }

        // Split "/command args..."
        let without_slash = &input[1..];
        let (name, args) = match without_slash.split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim().to_string()),
            None => (without_slash, String::new()),
        };

        match self.commands.get(name) {
            Some(cmd) => match &cmd.kind {
                CommandKind::Prompt { template } => {
                    let expanded = template.replace("{{args}}", &args);
                    CommandResult::ExpandedPrompt(expanded)
                }
                CommandKind::Action { action } => CommandResult::BuiltinAction {
                    action: action.clone(),
                    args,
                },
            },
            None => CommandResult::Unknown(name.into()),
        }
    }

    /// Get all commands for help display.
    pub fn all_commands(&self) -> Vec<&SlashCommand> {
        let mut cmds: Vec<_> = self.commands.values().collect();
        cmds.sort_by_key(|c| &c.name);
        cmds
    }

    /// Get prompt-type commands only.
    pub fn prompt_commands(&self) -> Vec<&SlashCommand> {
        self.all_commands()
            .into_iter()
            .filter(|c| matches!(c.kind, CommandKind::Prompt { .. }))
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// TOML format for custom commands file.
#[derive(Debug, Deserialize)]
struct CustomCommands {
    #[serde(default)]
    commands: Vec<SlashCommand>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_not_a_command() {
        let registry = CommandRegistry::with_builtins();
        assert!(matches!(
            registry.parse("hello world"),
            CommandResult::NotACommand
        ));
    }

    #[test]
    fn test_unknown_command() {
        let registry = CommandRegistry::with_builtins();
        assert!(matches!(
            registry.parse("/nonexistent"),
            CommandResult::Unknown(ref name) if name == "nonexistent"
        ));
    }

    #[test]
    fn test_builtin_action() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/help") {
            CommandResult::BuiltinAction { action, args } => {
                assert_eq!(action, "help");
                assert!(args.is_empty());
            }
            other => panic!("expected BuiltinAction, got {other:?}"),
        }
    }

    #[test]
    fn test_prompt_command_no_args() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/commit") {
            CommandResult::ExpandedPrompt(prompt) => {
                assert!(prompt.contains("git diff"));
                assert!(prompt.contains("")); // {{args}} replaced with empty
            }
            other => panic!("expected ExpandedPrompt, got {other:?}"),
        }
    }

    #[test]
    fn test_prompt_command_with_args() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/search authentication bug in login flow") {
            CommandResult::ExpandedPrompt(prompt) => {
                assert!(prompt.contains("authentication bug in login flow"));
                assert!(prompt.contains("Search the codebase"));
            }
            other => panic!("expected ExpandedPrompt, got {other:?}"),
        }
    }

    #[test]
    fn test_fix_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/fix the compile error in main.rs") {
            CommandResult::ExpandedPrompt(prompt) => {
                assert!(prompt.contains("the compile error in main.rs"));
                assert!(prompt.contains("Diagnose and fix"));
            }
            other => panic!("expected ExpandedPrompt, got {other:?}"),
        }
    }

    #[test]
    fn test_quit_aliases() {
        let registry = CommandRegistry::with_builtins();
        for cmd in ["/quit", "/exit", "/q"] {
            match registry.parse(cmd) {
                CommandResult::BuiltinAction { action, .. } => {
                    assert_eq!(action, "quit");
                }
                other => panic!("expected quit action for {cmd}, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_all_commands_sorted() {
        let registry = CommandRegistry::with_builtins();
        let cmds = registry.all_commands();
        for i in 1..cmds.len() {
            assert!(cmds[i - 1].name <= cmds[i].name, "commands not sorted");
        }
    }

    #[test]
    fn test_prompt_commands_filtered() {
        let registry = CommandRegistry::with_builtins();
        let prompt_cmds = registry.prompt_commands();
        for cmd in &prompt_cmds {
            assert!(matches!(cmd.kind, CommandKind::Prompt { .. }));
        }
        // commit, review, search, fix, explain, refactor, test, pr, doc, security-review, add-dir
        assert!(prompt_cmds.len() >= 11);
    }

    #[test]
    fn test_custom_command_registration() {
        let mut registry = CommandRegistry::with_builtins();
        registry.register(SlashCommand {
            name: "deploy".into(),
            description: "Deploy to staging".into(),
            kind: CommandKind::Prompt {
                template: "Run the deployment pipeline for staging. {{args}}".into(),
            },
        });

        match registry.parse("/deploy v2.0") {
            CommandResult::ExpandedPrompt(prompt) => {
                assert!(prompt.contains("deployment pipeline"));
                assert!(prompt.contains("v2.0"));
            }
            other => panic!("expected ExpandedPrompt, got {other:?}"),
        }
    }

    #[test]
    fn test_compact_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/compact") {
            CommandResult::BuiltinAction { action, .. } => {
                assert_eq!(action, "compact");
            }
            other => panic!("expected compact action, got {other:?}"),
        }
    }

    #[test]
    fn test_model_command_with_args() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/model claude-opus-4-6") {
            CommandResult::BuiltinAction { action, args } => {
                assert_eq!(action, "model");
                assert_eq!(args, "claude-opus-4-6");
            }
            other => panic!("expected model action, got {other:?}"),
        }
    }

    #[test]
    fn test_cost_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/cost") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "cost"),
            other => panic!("expected cost action, got {other:?}"),
        }
    }

    #[test]
    fn test_status_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/status") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "status"),
            other => panic!("expected status action, got {other:?}"),
        }
    }

    #[test]
    fn test_diff_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/diff") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "diff"),
            other => panic!("expected diff action, got {other:?}"),
        }
    }

    #[test]
    fn test_doctor_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/doctor") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "doctor"),
            other => panic!("expected doctor action, got {other:?}"),
        }
    }

    #[test]
    fn test_version_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/version") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "version"),
            other => panic!("expected version action, got {other:?}"),
        }
    }

    #[test]
    fn test_effort_command_with_args() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/effort high") {
            CommandResult::BuiltinAction { action, args } => {
                assert_eq!(action, "effort");
                assert_eq!(args, "high");
            }
            other => panic!("expected effort action, got {other:?}"),
        }
    }

    #[test]
    fn test_plan_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/plan") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "plan"),
            other => panic!("expected plan action, got {other:?}"),
        }
    }

    #[test]
    fn test_fast_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/fast") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "fast"),
            other => panic!("expected fast action, got {other:?}"),
        }
    }

    #[test]
    fn test_security_review_prompt() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/security-review auth.rs") {
            CommandResult::ExpandedPrompt(prompt) => {
                assert!(prompt.contains("security review"));
                assert!(prompt.contains("auth.rs"));
            }
            other => panic!("expected ExpandedPrompt, got {other:?}"),
        }
    }

    #[test]
    fn test_login_logout_commands() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/login") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "login"),
            other => panic!("expected login action, got {other:?}"),
        }
        match registry.parse("/logout") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "logout"),
            other => panic!("expected logout action, got {other:?}"),
        }
    }

    #[test]
    fn test_mcp_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/mcp") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "mcp"),
            other => panic!("expected mcp action, got {other:?}"),
        }
    }

    #[test]
    fn test_memory_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/memory") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "memory"),
            other => panic!("expected memory action, got {other:?}"),
        }
    }

    #[test]
    fn test_permissions_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/permissions auto") {
            CommandResult::BuiltinAction { action, args } => {
                assert_eq!(action, "permissions");
                assert_eq!(args, "auto");
            }
            other => panic!("expected permissions action, got {other:?}"),
        }
    }

    #[test]
    fn test_all_builtin_count() {
        let registry = CommandRegistry::with_builtins();
        let all = registry.all_commands();
        // 11 prompt + 26 action commands = 37
        assert!(all.len() >= 35, "expected at least 35 commands, got {}", all.len());
    }

    #[test]
    fn test_export_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/export conversation.md") {
            CommandResult::BuiltinAction { action, args } => {
                assert_eq!(action, "export");
                assert_eq!(args, "conversation.md");
            }
            other => panic!("expected export action, got {other:?}"),
        }
    }

    #[test]
    fn test_context_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/context") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "context"),
            other => panic!("expected context action, got {other:?}"),
        }
    }

    #[test]
    fn test_resume_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/resume") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "resume"),
            other => panic!("expected resume action, got {other:?}"),
        }
    }

    #[test]
    fn test_vim_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/vim") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "vim"),
            other => panic!("expected vim action, got {other:?}"),
        }
    }

    #[test]
    fn test_tasks_command() {
        let registry = CommandRegistry::with_builtins();
        match registry.parse("/tasks") {
            CommandResult::BuiltinAction { action, .. } => assert_eq!(action, "tasks"),
            other => panic!("expected tasks action, got {other:?}"),
        }
    }
}
