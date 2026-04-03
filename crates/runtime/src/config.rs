//! Configuration system with hierarchical merging.
//!
//! Design insight from Claw-Code-Parity: Config hierarchy with
//! user < project < local < env < cli precedence.
//!
//! Format: TOML (cleaner than JSON for human editing).

use serde::{Deserialize, Serialize};

/// Runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub max_budget_usd: Option<f64>,
    pub temperature: Option<f64>,
    pub permission_mode: PermissionMode,
    pub sandbox_mode: SandboxMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Ask for every tool use
    Default,
    /// Allow read-only tools, ask for writes
    AcceptReads,
    /// Allow all tools without asking
    BypassPermissions,
    /// Deny all tool use
    DenyAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxMode {
    None,
    OsNative,
    Container,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 16384,
            max_turns: 50,
            max_budget_usd: None,
            temperature: None,
            permission_mode: PermissionMode::Default,
            sandbox_mode: SandboxMode::None,
        }
    }
}
