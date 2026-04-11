//! cisco-code-tools: Tool registry and built-in tool implementations.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolDefinition, ToolResult};

pub mod agent;
pub mod apply_patch;
pub mod bash;
pub mod config_tool;
pub mod cron_tools;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp_tool;
pub mod notebook;
pub mod plan;
pub mod read;
pub mod send_message;
pub mod skill;
pub mod sleep;
pub mod task_tools;
pub mod todo_tool;
pub mod tool_search;
pub mod web_fetch;
pub mod web_search;
pub mod worktree_tool;
pub mod write;

/// Context passed to every tool execution.
#[derive(Clone)]
pub struct ToolContext {
    pub cwd: String,
    pub interactive: bool,
    /// Optional channel for emitting real-time progress events during tool execution.
    /// When `Some`, tools can send `StreamEvent::ToolProgress` for live UI updates.
    pub progress_tx: Option<tokio::sync::mpsc::Sender<cisco_code_protocol::StreamEvent>>,
}

impl ToolContext {
    /// Emit a progress event for the current tool execution.
    ///
    /// Best-effort: silently ignores send failures (receiver dropped, channel full).
    /// This allows tools to report live progress without error handling overhead.
    pub fn emit_progress(&self, tool_use_id: &str, data: cisco_code_protocol::ToolProgressData) {
        if let Some(ref tx) = self.progress_tx {
            let _ = tx.try_send(cisco_code_protocol::StreamEvent::ToolProgress {
                tool_use_id: tool_use_id.to_string(),
                progress: data,
            });
        }
    }
}

/// The core tool trait.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult>;
    fn permission_level(&self) -> PermissionLevel;

    /// Whether this tool is safe to execute concurrently with other safe tools.
    ///
    /// Returns `true` for read-only tools with no side effects (Read, Grep, Glob, etc.).
    /// Returns `false` (default) for tools that modify state (Bash, Write, Edit, etc.).
    ///
    /// The `StreamingToolExecutor` uses this to run consecutive safe tools in parallel
    /// while executing unsafe tools one at a time with exclusive access.
    fn is_concurrency_safe(&self) -> bool {
        false
    }
}

/// Global tool registry.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            anyhow::bail!("Tool already registered: {name}");
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Register a batch of MCP tools discovered at runtime.
    ///
    /// This is used after MCP server discovery to dynamically add tools
    /// with `mcp__{server}__{tool}` naming convention.
    pub fn register_mcp_tools(&mut self, tools: Vec<mcp_tool::McpTool>) -> Result<()> {
        for tool in tools {
            self.register(Arc::new(tool))?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<_> = self
            .tools
            .values()
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }

    /// Execute a tool by name.
    pub async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult> {
        let tool = self
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
        tool.call(input, ctx).await
    }

    /// Create registry with all built-in tools.
    pub fn with_builtins() -> Result<Self> {
        let mut registry = Self::new();
        // Core file/shell tools
        registry.register(Arc::new(bash::BashTool))?;
        registry.register(Arc::new(read::ReadTool))?;
        registry.register(Arc::new(write::WriteTool))?;
        registry.register(Arc::new(edit::EditTool))?;
        registry.register(Arc::new(apply_patch::ApplyPatchTool))?;
        registry.register(Arc::new(grep::GrepTool))?;
        registry.register(Arc::new(glob::GlobTool))?;
        // Agent and subagent tools
        registry.register(Arc::new(agent::AgentTool))?;
        // Web tools
        registry.register(Arc::new(web_fetch::WebFetchTool))?;
        registry.register(Arc::new(web_search::WebSearchTool))?;
        // Notebook
        registry.register(Arc::new(notebook::NotebookEditTool))?;
        // Code intelligence
        registry.register(Arc::new(lsp::LspTool))?;
        // Skills and search
        registry.register(Arc::new(skill::SkillTool))?;
        registry.register(Arc::new(tool_search::ToolSearchTool))?;
        // Plan mode
        registry.register(Arc::new(plan::EnterPlanModeTool))?;
        registry.register(Arc::new(plan::ExitPlanModeTool))?;
        // Task management
        registry.register(Arc::new(task_tools::TaskCreateTool))?;
        registry.register(Arc::new(task_tools::TaskUpdateTool))?;
        registry.register(Arc::new(task_tools::TaskListTool))?;
        registry.register(Arc::new(task_tools::TaskGetTool))?;
        registry.register(Arc::new(task_tools::TaskOutputTool))?;
        registry.register(Arc::new(task_tools::TaskStopTool))?;
        // Cron scheduling
        registry.register(Arc::new(cron_tools::CronCreateTool))?;
        registry.register(Arc::new(cron_tools::CronListTool))?;
        registry.register(Arc::new(cron_tools::CronDeleteTool))?;
        // Worktree isolation
        registry.register(Arc::new(worktree_tool::EnterWorktreeTool))?;
        registry.register(Arc::new(worktree_tool::ExitWorktreeTool))?;
        // User interaction
        registry.register(Arc::new(worktree_tool::AskUserQuestionTool))?;
        // Messaging
        registry.register(Arc::new(send_message::SendMessageTool))?;
        // Configuration
        registry.register(Arc::new(config_tool::ConfigTool))?;
        // Todo management
        registry.register(Arc::new(todo_tool::TodoWriteTool))?;
        // Sleep
        registry.register(Arc::new(sleep::SleepTool))?;
        // MCP resource tools (dynamic McpTool instances are added at runtime)
        registry.register(Arc::new(mcp_tool::ListMcpResourcesTool))?;
        registry.register(Arc::new(mcp_tool::ReadMcpResourceTool))?;
        Ok(registry)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_is_empty() {
        let reg = ToolRegistry::new();
        assert!(reg.definitions().is_empty());
    }

    #[test]
    fn test_with_builtins_has_all_tools() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let defs = reg.definitions();
        // 6 core + 1 apply_patch + 1 agent + 2 web + 1 notebook + 1 lsp + 1 skill
        // + 1 tool_search + 2 plan + 6 task + 3 cron + 2 worktree + 1 ask
        // + 1 send_message + 1 config + 1 todo + 1 sleep + 2 mcp_resources = 34
        assert_eq!(defs.len(), 34);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        // Core tools
        assert!(names.contains(&"Bash"));
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Write"));
        assert!(names.contains(&"Edit"));
        assert!(names.contains(&"Grep"));
        assert!(names.contains(&"Glob"));
        // Agent & web tools
        assert!(names.contains(&"Agent"));
        assert!(names.contains(&"WebFetch"));
        assert!(names.contains(&"WebSearch"));
        assert!(names.contains(&"NotebookEdit"));
        assert!(names.contains(&"LSP"));
        assert!(names.contains(&"Skill"));
        assert!(names.contains(&"ToolSearch"));
        assert!(names.contains(&"EnterPlanMode"));
        assert!(names.contains(&"ExitPlanMode"));
        // Task tools
        assert!(names.contains(&"TaskCreate"));
        assert!(names.contains(&"TaskUpdate"));
        assert!(names.contains(&"TaskList"));
        assert!(names.contains(&"TaskGet"));
        assert!(names.contains(&"TaskOutput"));
        assert!(names.contains(&"TaskStop"));
        // Cron & worktree tools
        assert!(names.contains(&"CronCreate"));
        assert!(names.contains(&"CronList"));
        assert!(names.contains(&"CronDelete"));
        assert!(names.contains(&"EnterWorktree"));
        assert!(names.contains(&"ExitWorktree"));
        assert!(names.contains(&"AskUserQuestion"));
        // New tools: messaging, config, sleep, MCP resources, apply_patch, todo
        assert!(names.contains(&"SendMessage"));
        assert!(names.contains(&"Config"));
        assert!(names.contains(&"Sleep"));
        assert!(names.contains(&"ListMcpResources"));
        assert!(names.contains(&"ReadMcpResource"));
        assert!(names.contains(&"ApplyPatch"));
        assert!(names.contains(&"TodoWrite"));
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(read::ReadTool)).unwrap();
        assert!(reg.get("Read").is_some());
        assert!(reg.get("NonExistent").is_none());
    }

    #[test]
    fn test_duplicate_registration_fails() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(read::ReadTool)).unwrap();
        let result = reg.register(Arc::new(read::ReadTool));
        assert!(result.is_err());
    }

    #[test]
    fn test_definitions_sorted_by_name() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let defs = reg.definitions();
        for i in 1..defs.len() {
            assert!(defs[i - 1].name <= defs[i].name, "definitions not sorted");
        }
    }

    #[test]
    fn test_tool_schemas_are_valid_json_objects() {
        let reg = ToolRegistry::with_builtins().unwrap();
        for def in reg.definitions() {
            assert!(
                def.input_schema.is_object(),
                "{} schema is not an object",
                def.name
            );
            assert_eq!(
                def.input_schema["type"], "object",
                "{} schema type is not 'object'",
                def.name
            );
            assert!(
                def.input_schema.get("properties").is_some(),
                "{} schema has no properties",
                def.name
            );
        }
    }

    #[tokio::test]
    async fn test_execute_unknown_tool_returns_error() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let ctx = ToolContext {
            cwd: ".".into(),
            interactive: false,
            progress_tx: None,
        };
        let result = reg.execute("NonExistent", serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        let reg = ToolRegistry::with_builtins().unwrap();
        for def in reg.definitions() {
            assert!(
                !def.description.is_empty(),
                "{} has no description",
                def.name
            );
            assert!(
                def.description.len() > 10,
                "{} description is too short",
                def.name
            );
        }
    }

    #[test]
    fn test_all_tools_have_unique_names() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let defs = reg.definitions();
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!(
                seen.insert(def.name.as_str()),
                "Duplicate tool name: {}",
                def.name
            );
        }
    }

    #[test]
    fn test_all_schemas_have_required_field() {
        let reg = ToolRegistry::with_builtins().unwrap();
        for def in reg.definitions() {
            // Every schema should have a "required" field (even if empty array)
            assert!(
                def.input_schema.get("required").is_some(),
                "{} schema has no 'required' field",
                def.name
            );
        }
    }

    #[test]
    fn test_permission_levels_are_set() {
        let reg = ToolRegistry::with_builtins().unwrap();
        // Verify some known permission levels
        let bash = reg.get("Bash").unwrap();
        assert_eq!(bash.permission_level(), PermissionLevel::Execute);

        let read = reg.get("Read").unwrap();
        assert_eq!(read.permission_level(), PermissionLevel::ReadOnly);

        let write = reg.get("Write").unwrap();
        assert_eq!(write.permission_level(), PermissionLevel::WorkspaceWrite);

        let agent = reg.get("Agent").unwrap();
        assert_eq!(agent.permission_level(), PermissionLevel::Execute);
    }

    #[test]
    fn test_register_mcp_tools_batch() {
        let mut reg = ToolRegistry::new();
        let tools = vec![
            mcp_tool::McpTool::new_dynamic(
                "github",
                "search_repos",
                "Search GitHub repositories",
                serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
            ),
            mcp_tool::McpTool::new_dynamic(
                "github",
                "create_issue",
                "Create a GitHub issue",
                serde_json::json!({"type": "object", "properties": {"title": {"type": "string"}}, "required": ["title"]}),
            ),
        ];
        reg.register_mcp_tools(tools).unwrap();

        assert!(reg.get("mcp__github__search_repos").is_some());
        assert!(reg.get("mcp__github__create_issue").is_some());
        assert_eq!(reg.definitions().len(), 2);
    }

    #[test]
    fn test_register_mcp_tools_duplicate_fails() {
        let mut reg = ToolRegistry::new();
        let tools = vec![
            mcp_tool::McpTool::new_dynamic("srv", "tool_a", "desc", serde_json::json!({"type": "object", "properties": {}, "required": []})),
        ];
        reg.register_mcp_tools(tools).unwrap();

        let dup = vec![
            mcp_tool::McpTool::new_dynamic("srv", "tool_a", "desc", serde_json::json!({"type": "object", "properties": {}, "required": []})),
        ];
        assert!(reg.register_mcp_tools(dup).is_err());
    }

    #[test]
    fn test_default_registry() {
        let reg = ToolRegistry::default();
        assert!(reg.definitions().is_empty());
    }

    #[tokio::test]
    async fn test_execute_via_registry() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let ctx = ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        };
        // Execute TaskList — should succeed with a request descriptor
        let result = reg
            .execute("TaskList", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("task_list"));
    }

    #[tokio::test]
    async fn test_execute_plan_mode_tools_via_registry() {
        let reg = ToolRegistry::with_builtins().unwrap();
        let ctx = ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        };
        let enter = reg
            .execute("EnterPlanMode", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert!(enter.output.contains("enter_plan_mode"));

        let exit = reg
            .execute("ExitPlanMode", serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert!(exit.output.contains("exit_plan_mode"));
    }
}
