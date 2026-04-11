//! MCP tool — dynamic wrapper for Model Context Protocol server tools.
//!
//! Matches Claude Code's MCPTool: wraps tools discovered from MCP servers
//! with a standardized interface. The actual MCP client dispatches calls.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

/// A dynamic MCP tool that wraps a tool discovered from an MCP server.
pub struct McpTool {
    /// The prefixed name: `mcp__{server}__{tool}`.
    prefixed_name: String,
    /// The original tool name on the MCP server.
    tool_name: String,
    tool_description: String,
    tool_schema: serde_json::Value,
    server_name: String,
}

impl McpTool {
    /// Create a new MCP tool wrapper.
    pub fn new(
        tool_name: impl Into<String>,
        tool_description: impl Into<String>,
        tool_schema: serde_json::Value,
        server_name: impl Into<String>,
    ) -> Self {
        let tool_name = tool_name.into();
        let server_name = server_name.into();
        let prefixed_name = format!("mcp__{server_name}__{tool_name}");
        Self {
            prefixed_name,
            tool_name,
            tool_description: tool_description.into(),
            tool_schema,
            server_name,
        }
    }

    /// Create a dynamic MCP tool from server discovery results.
    ///
    /// This is the primary constructor used during runtime MCP tool registration.
    pub fn new_dynamic(
        server_name: &str,
        tool_name: &str,
        description: &str,
        schema: serde_json::Value,
    ) -> Self {
        Self::new(tool_name, description, schema, server_name)
    }

    /// The MCP server that provides this tool.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// The original tool name on the MCP server (without prefix).
    pub fn raw_tool_name(&self) -> &str {
        &self.tool_name
    }
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.tool_schema.clone()
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        // Package the MCP call request for the runtime to dispatch
        let request = json!({
            "type": "mcp_tool_call",
            "server": self.server_name,
            "tool": self.tool_name,
            "arguments": input,
        });

        Ok(ToolResult::success(
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|_| request.to_string()),
        ))
    }

    fn permission_level(&self) -> PermissionLevel {
        // MCP tools default to Execute; runtime can override per-server
        PermissionLevel::Execute
    }
}

/// Static MCP resource tools (ListMcpResources, ReadMcpResource).
pub struct ListMcpResourcesTool;

#[async_trait::async_trait]
impl Tool for ListMcpResourcesTool {
    fn name(&self) -> &str {
        "ListMcpResources"
    }

    fn description(&self) -> &str {
        "List available resources from connected MCP servers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP server name (optional, lists all if omitted)"
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let server = input.get("server").and_then(|v| v.as_str());
        let request = json!({
            "type": "mcp_list_resources",
            "server": server,
        });
        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

pub struct ReadMcpResourceTool;

#[async_trait::async_trait]
impl Tool for ReadMcpResourceTool {
    fn name(&self) -> &str {
        "ReadMcpResource"
    }

    fn description(&self) -> &str {
        "Read a specific resource from an MCP server by URI."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string",
                    "description": "MCP server name"
                },
                "uri": {
                    "type": "string",
                    "description": "Resource URI to read"
                }
            },
            "required": ["server", "uri"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let server = match input["server"].as_str() {
            Some(s) if !s.trim().is_empty() => s,
            _ => return Ok(ToolResult::error("missing required parameter 'server'")),
        };
        let uri = match input["uri"].as_str() {
            Some(u) if !u.trim().is_empty() => u,
            _ => return Ok(ToolResult::error("missing required parameter 'uri'")),
        };

        let request = json!({
            "type": "mcp_read_resource",
            "server": server,
            "uri": uri,
        });
        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        }
    }

    #[tokio::test]
    async fn test_mcp_tool_call() {
        let tool = McpTool::new(
            "weather",
            "Get weather data",
            json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
            "weather-server",
        );
        assert_eq!(tool.name(), "mcp__weather-server__weather");
        assert_eq!(tool.server_name(), "weather-server");
        assert_eq!(tool.raw_tool_name(), "weather");

        let result = tool
            .call(json!({"city": "San Jose"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "mcp_tool_call");
        assert_eq!(parsed["server"], "weather-server");
        assert_eq!(parsed["tool"], "weather");
        assert_eq!(parsed["arguments"]["city"], "San Jose");
    }

    #[tokio::test]
    async fn test_new_dynamic_constructor() {
        let tool = McpTool::new_dynamic(
            "github",
            "search_repos",
            "Search GitHub repositories",
            json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
        );
        assert_eq!(tool.name(), "mcp__github__search_repos");
        assert_eq!(tool.server_name(), "github");
        assert_eq!(tool.raw_tool_name(), "search_repos");
        assert_eq!(tool.description(), "Search GitHub repositories");
    }

    #[tokio::test]
    async fn test_mcp_tool_schema_passthrough() {
        let schema = json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        });
        let tool = McpTool::new("search", "Search", schema.clone(), "db-server");
        assert_eq!(tool.input_schema(), schema);
    }

    #[tokio::test]
    async fn test_list_mcp_resources() {
        let tool = ListMcpResourcesTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "mcp_list_resources");
    }

    #[tokio::test]
    async fn test_list_mcp_resources_with_server() {
        let tool = ListMcpResourcesTool;
        let result = tool
            .call(json!({"server": "my-server"}), &ctx())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["server"], "my-server");
    }

    #[tokio::test]
    async fn test_read_mcp_resource() {
        let tool = ReadMcpResourceTool;
        let result = tool
            .call(
                json!({"server": "db-server", "uri": "db://users/schema"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "mcp_read_resource");
        assert_eq!(parsed["uri"], "db://users/schema");
    }

    #[tokio::test]
    async fn test_read_mcp_resource_missing_server() {
        let tool = ReadMcpResourceTool;
        let result = tool
            .call(json!({"uri": "test://x"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_read_mcp_resource_missing_uri() {
        let tool = ReadMcpResourceTool;
        let result = tool
            .call(json!({"server": "srv"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_mcp_tool_permission() {
        let tool = McpTool::new("t", "d", json!({}), "s");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }

    #[test]
    fn test_list_resources_permission() {
        assert_eq!(ListMcpResourcesTool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn test_read_resource_permission() {
        assert_eq!(ReadMcpResourceTool.permission_level(), PermissionLevel::ReadOnly);
    }
}
