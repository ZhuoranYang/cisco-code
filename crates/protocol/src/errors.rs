//! Error types shared across crates.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CiscoCodeError {
    #[error("API error: {message}")]
    Api { message: String, status: Option<u16> },

    #[error("Tool error in {tool_name}: {message}")]
    Tool { tool_name: String, message: String },

    #[error("Permission denied for {tool_name}: {reason}")]
    PermissionDenied { tool_name: String, reason: String },

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),

    #[error("Provider error: {provider}: {message}")]
    Provider { provider: String, message: String },

    #[error("IPC error: {0}")]
    Ipc(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),
}
