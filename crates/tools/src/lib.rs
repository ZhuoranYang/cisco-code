//! cisco-code-tools: Tool registry and built-in tool implementations.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolDefinition, ToolMetadata, ToolResult, ToolSource};

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;

/// Context passed to every tool execution.
#[derive(Clone)]
pub struct ToolContext {
    pub cwd: String,
    pub interactive: bool,
}

/// The core tool trait.
#[allow(async_fn_in_trait)]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult>;
    fn permission_level(&self) -> PermissionLevel;
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
        registry.register(Arc::new(bash::BashTool))?;
        registry.register(Arc::new(read::ReadTool))?;
        registry.register(Arc::new(write::WriteTool))?;
        registry.register(Arc::new(edit::EditTool))?;
        registry.register(Arc::new(grep::GrepTool))?;
        registry.register(Arc::new(glob::GlobTool))?;
        Ok(registry)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
