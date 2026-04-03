//! cisco-code-tools: Tool registry and built-in tool implementations.
//!
//! Design insight from Claude Code: Tools are the central abstraction. Everything
//! revolves around the tool system — the LLM is a tool-calling engine, the runtime
//! is a tool-execution engine.
//!
//! Design insight from Codex: The registry pattern with async trait handlers enables
//! clean separation between tool definition and execution.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use cisco_code_protocol::{ToolDefinition, ToolResult, ToolMetadata, PermissionLevel};

/// Context passed to every tool execution.
///
/// Design insight from Claude Code: ToolUseContext carries ~20+ fields including
/// app state, file cache, MCP clients, abort controller, etc. Tools are first-class
/// citizens that can influence the entire session.
#[derive(Clone)]
pub struct ToolContext {
    /// Current working directory
    pub cwd: String,
    /// Whether the session is interactive (can prompt user)
    pub interactive: bool,
    /// Abort signal
    pub abort: tokio::sync::watch::Receiver<bool>,
}

/// The core tool trait. Every tool implements this.
///
/// Follows Codex's async trait pattern with Claw-Code-Parity's simplicity.
#[allow(async_fn_in_trait)]
pub trait Tool: Send + Sync {
    /// Tool name (must be unique in registry)
    fn name(&self) -> &str;

    /// Tool description (sent to LLM)
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with given input
    async fn call(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult>;

    /// Tool metadata for permission engine
    fn metadata(&self) -> ToolMetadata;
}

/// Global tool registry.
///
/// Design pattern from Codex: HashMap<name, Arc<dyn Tool>> with type-erased handlers.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Returns error if name already registered.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<()> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            anyhow::bail!("Tool already registered: {name}");
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Look up a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Generate tool definitions for the LLM API request.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    /// Create registry with all built-in tools.
    pub fn with_builtins() -> Result<Self> {
        let mut registry = Self::new();
        // Tools will be registered here as they're implemented
        // Phase 2: bash, read_file, write_file, edit_file, glob, grep, web_fetch, agent, memory
        let _ = &registry; // suppress unused warning during scaffolding
        Ok(registry)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
