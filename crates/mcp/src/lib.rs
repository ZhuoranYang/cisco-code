//! cisco-code-mcp: Model Context Protocol client and server.
//!
//! MCP enables cisco-code to:
//! - Connect to external MCP servers and use their tools
//! - Expose its own tools to other agents (as an MCP server)
//!
//! Transport modes:
//! - stdio: Spawn a child process, communicate via JSON-RPC over stdin/stdout
//! - HTTP+SSE: Connect to a remote MCP server (streamable HTTP)
//!
//! Protocol: JSON-RPC 2.0 with MCP-specific methods:
//! - initialize / initialized
//! - tools/list, tools/call
//! - resources/list, resources/read
//! - prompts/list, prompts/get

pub mod client;
pub mod jsonrpc;
pub mod transport;
pub mod types;

pub use client::McpClient;
pub use types::*;
