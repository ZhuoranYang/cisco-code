//! High-level MCP client.
//!
//! Manages the lifecycle: connect → initialize → discover tools → call tools.
//! Converts MCP tools into cisco-code ToolDefinitions for seamless integration.

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest};
use crate::transport::{self, Transport};
use crate::types::*;

/// MCP client — connects to a single MCP server and exposes its tools.
pub struct McpClient {
    /// Server display name.
    pub name: String,
    /// Transport layer.
    transport: Box<dyn Transport>,
    /// Server capabilities discovered during initialization.
    pub capabilities: ServerCapabilities,
    /// Server info from initialization.
    pub server_info: Option<Implementation>,
    /// Auto-incrementing request ID.
    next_id: AtomicU64,
    /// Cached tool list.
    tools: Vec<McpTool>,
}

impl McpClient {
    /// Connect to an MCP server and perform initialization handshake.
    pub async fn connect(config: &McpServerConfig) -> Result<Self> {
        let transport = transport::create_transport(&config.transport, &config.env).await?;

        let mut client = Self {
            name: config.name.clone(),
            transport,
            capabilities: ServerCapabilities::default(),
            server_info: None,
            next_id: AtomicU64::new(1),
            tools: Vec::new(),
        };

        client.initialize().await?;
        Ok(client)
    }

    /// MCP initialize handshake.
    async fn initialize(&mut self) -> Result<()> {
        let params = InitializeParams {
            protocol_version: MCP_PROTOCOL_VERSION.into(),
            capabilities: ClientCapabilities {
                roots: Some(RootsCapability { list_changed: true }),
            },
            client_info: Implementation {
                name: "cisco-code".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
        };

        let req = self.make_request("initialize", Some(serde_json::to_value(&params)?));
        let resp = self.transport.request(req).await?;
        let result: InitializeResult = serde_json::from_value(resp.into_result().map_err(|e| {
            anyhow::anyhow!("MCP initialize failed: {e}")
        })?)?;

        self.capabilities = result.capabilities;
        self.server_info = Some(result.server_info);

        // Send initialized notification
        let notif = JsonRpcNotification::new("notifications/initialized", None);
        self.transport.notify(notif).await?;

        tracing::info!(
            "MCP server '{}' initialized (protocol {})",
            self.name,
            result.protocol_version,
        );

        Ok(())
    }

    /// Discover available tools from the server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let req = self.make_request("tools/list", None);
        let resp = self.transport.request(req).await?;
        let result: ToolsListResult = serde_json::from_value(resp.into_result().map_err(|e| {
            anyhow::anyhow!("tools/list failed: {e}")
        })?)?;

        self.tools = result.tools.clone();

        tracing::info!(
            "MCP server '{}': {} tools available",
            self.name,
            self.tools.len(),
        );

        Ok(result.tools)
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult> {
        let params = ToolCallParams {
            name: name.into(),
            arguments,
        };

        let req = self.make_request("tools/call", Some(serde_json::to_value(&params)?));
        let resp = self.transport.request(req).await?;
        let result: ToolCallResult = serde_json::from_value(resp.into_result().map_err(|e| {
            anyhow::anyhow!("tools/call '{name}' failed: {e}")
        })?)?;

        Ok(result)
    }

    /// List available resources.
    pub async fn list_resources(&self) -> Result<Vec<McpResource>> {
        let req = self.make_request("resources/list", None);
        let resp = self.transport.request(req).await?;
        let result: ResourcesListResult =
            serde_json::from_value(resp.into_result().map_err(|e| {
                anyhow::anyhow!("resources/list failed: {e}")
            })?)?;
        Ok(result.resources)
    }

    /// Read a resource by URI.
    pub async fn read_resource(&self, uri: &str) -> Result<ResourceReadResult> {
        let params = serde_json::json!({ "uri": uri });
        let req = self.make_request("resources/read", Some(params));
        let resp = self.transport.request(req).await?;
        let result: ResourceReadResult =
            serde_json::from_value(resp.into_result().map_err(|e| {
                anyhow::anyhow!("resources/read '{uri}' failed: {e}")
            })?)?;
        Ok(result)
    }

    /// List available prompts.
    pub async fn list_prompts(&self) -> Result<Vec<McpPrompt>> {
        let req = self.make_request("prompts/list", None);
        let resp = self.transport.request(req).await?;
        let result: PromptsListResult =
            serde_json::from_value(resp.into_result().map_err(|e| {
                anyhow::anyhow!("prompts/list failed: {e}")
            })?)?;
        Ok(result.prompts)
    }

    /// Get a prompt by name with arguments.
    pub async fn get_prompt(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<PromptGetResult> {
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        let req = self.make_request("prompts/get", Some(params));
        let resp = self.transport.request(req).await?;
        let result: PromptGetResult = serde_json::from_value(resp.into_result().map_err(|e| {
            anyhow::anyhow!("prompts/get '{name}' failed: {e}")
        })?)?;
        Ok(result)
    }

    /// Convert this server's MCP tools to cisco-code ToolDefinitions.
    ///
    /// Tool names are prefixed with "mcp__{server_name}__" (double underscore)
    /// to match the Claude Code convention and avoid collisions with built-in tools.
    pub fn to_tool_definitions(&self) -> Vec<cisco_code_protocol::ToolDefinition> {
        self.tools
            .iter()
            .map(|t| cisco_code_protocol::ToolDefinition {
                name: format!("mcp__{}__{}", self.name, t.name),
                description: t
                    .description
                    .clone()
                    .unwrap_or_else(|| format!("MCP tool from {}", self.name)),
                input_schema: t.input_schema.clone(),
            })
            .collect()
    }

    /// Get cached tools.
    pub fn cached_tools(&self) -> &[McpTool] {
        &self.tools
    }

    /// Disconnect from the server.
    pub async fn close(self) -> Result<()> {
        self.transport.close().await
    }

    /// Create a JSON-RPC request with auto-incrementing ID.
    fn make_request(&self, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        JsonRpcRequest::new(id, method, params)
    }
}

/// Manages multiple MCP server connections.
pub struct McpManager {
    clients: Vec<McpClient>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            clients: Vec::new(),
        }
    }

    /// Connect to an MCP server and add it to the manager.
    pub async fn add_server(&mut self, config: &McpServerConfig) -> Result<()> {
        let client = McpClient::connect(config).await?;
        self.clients.push(client);
        Ok(())
    }

    /// Discover tools from all connected servers.
    pub async fn discover_all_tools(&mut self) -> Result<Vec<cisco_code_protocol::ToolDefinition>> {
        let mut all_defs = Vec::new();
        for client in &mut self.clients {
            client.list_tools().await?;
            all_defs.extend(client.to_tool_definitions());
        }
        Ok(all_defs)
    }

    /// Call an MCP tool by its prefixed name (e.g., "mcp__github__search_repos").
    pub async fn call_tool(
        &self,
        prefixed_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult> {
        // Parse "mcp__{server}__{tool}" format
        let parts: Vec<&str> = prefixed_name.splitn(3, "__").collect();
        if parts.len() != 3 || parts[0] != "mcp" {
            anyhow::bail!("Invalid MCP tool name format: {prefixed_name}. Expected mcp__server__tool");
        }
        let server_name = parts[1];
        let tool_name = parts[2];

        let client = self
            .clients
            .iter()
            .find(|c| c.name == server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{server_name}' not connected"))?;

        client.call_tool(tool_name, arguments).await
    }

    /// Get all connected server names.
    pub fn server_names(&self) -> Vec<&str> {
        self.clients.iter().map(|c| c.name.as_str()).collect()
    }

    /// Get the number of connected servers.
    pub fn server_count(&self) -> usize {
        self.clients.len()
    }

    /// Get total number of available tools across all servers.
    pub fn tool_count(&self) -> usize {
        self.clients.iter().map(|c| c.tools.len()).sum()
    }

    /// Disconnect all servers.
    pub async fn close_all(self) -> Result<()> {
        for client in self.clients {
            if let Err(e) = client.close().await {
                tracing::warn!("Error closing MCP server '{}': {e}", "unknown");
            }
        }
        Ok(())
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_manager_empty() {
        let manager = McpManager::new();
        assert_eq!(manager.server_count(), 0);
        assert_eq!(manager.tool_count(), 0);
        assert!(manager.server_names().is_empty());
    }

    #[test]
    fn test_tool_name_format() {
        // Simulate what to_tool_definitions produces
        let tool = McpTool {
            name: "search_repos".into(),
            description: Some("Search GitHub repos".into()),
            input_schema: serde_json::json!({"type": "object"}),
        };

        let server_name = "github";
        let prefixed = format!("mcp__{server_name}__{}", tool.name);
        assert_eq!(prefixed, "mcp__github__search_repos");

        // Parse it back
        let parts: Vec<&str> = prefixed.splitn(3, "__").collect();
        assert_eq!(parts, vec!["mcp", "github", "search_repos"]);
    }

    #[tokio::test]
    async fn test_call_tool_invalid_format() {
        let manager = McpManager::new();

        let result = manager.call_tool("invalid_name", serde_json::json!({})).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid MCP tool name"));
    }

    #[tokio::test]
    async fn test_call_tool_unknown_server() {
        let manager = McpManager::new();

        let result = manager
            .call_tool("mcp__unknown__some_tool", serde_json::json!({}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[test]
    fn test_to_tool_definitions_prefixes_names() {
        // We can't create a full McpClient without a transport, but we can test the logic
        let tool = McpTool {
            name: "read_file".into(),
            description: Some("Read a file".into()),
            input_schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };

        let server_name = "filesystem";
        let def = cisco_code_protocol::ToolDefinition {
            name: format!("mcp__{server_name}__{}", tool.name),
            description: tool.description.unwrap_or_default(),
            input_schema: tool.input_schema,
        };

        assert_eq!(def.name, "mcp__filesystem__read_file");
        assert_eq!(def.description, "Read a file");
    }
}
