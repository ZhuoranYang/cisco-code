//! Tool definitions and result types.
//!
//! Design insight from Claude Code: Tools are the central abstraction. A ToolResult
//! is not just output — it can carry side effects (new messages, context modifications).
//! This makes tools first-class participants in the conversation, not just functions.
//!
//! Design insight from Codex: Tools have a ToolKind (Function, Mcp, ToolSearch) that
//! determines how they're routed and sandboxed.

use serde::{Deserialize, Serialize};

/// Tool definition sent to the LLM in the API request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result of executing a tool.
///
/// Follows Claude Code's pattern where tool results can carry side effects:
/// - `output`: The direct output shown to the LLM
/// - `injected_messages`: Additional messages to insert into history
/// - `context_patch`: Modifications to the runtime context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// The tool's output (shown to LLM as tool_result content)
    pub output: String,

    /// Whether the tool execution failed
    pub is_error: bool,

    /// Additional messages to inject into conversation history.
    /// This is Claude Code's key pattern: tools can reshape the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub injected_messages: Option<Vec<crate::messages::Message>>,
}

impl ToolResult {
    /// Create a successful result with just output text.
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
            injected_messages: None,
        }
    }

    /// Create an error result.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            output: message.into(),
            is_error: true,
            injected_messages: None,
        }
    }
}

/// Permission level required to execute a tool.
///
/// Design insight: Claude Code has 6 permission modes; Codex has 3 sandbox policies.
/// We use 4 levels that map cleanly to both systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// No permission needed (read-only operations)
    ReadOnly,
    /// Write to workspace files (edit, create files)
    WorkspaceWrite,
    /// Execute commands, network access
    Execute,
    /// Destructive or irreversible operations
    Elevated,
}

/// Tool metadata used by the permission engine and registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub permission_level: PermissionLevel,
    pub is_read_only: bool,
    pub is_destructive: bool,
    pub is_concurrency_safe: bool,
    /// Tool source: built-in, plugin, or MCP
    pub source: ToolSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolSource {
    BuiltIn,
    Plugin { plugin_id: String },
    Mcp { server_name: String },
    Cisco { service: String },
}
