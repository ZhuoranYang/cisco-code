//! cisco-code-sandbox: OS-native sandboxing for tool execution.
//!
//! Design insight from Codex: Don't sandbox the tool — sandbox the *execution*.
//! The CommandSpec → SandboxTransformRequest → ExecRequest pipeline means tool
//! authors write normal code, and the sandbox layer wraps it transparently.
//!
//! Codex implements:
//! - macOS: Seatbelt (mandatory access control profiles)
//! - Linux: Bubblewrap (user namespaces) + Landlock (LSM)
//! - Windows: Restricted tokens
//!
//! cisco-code will implement all three plus Docker container sandboxing.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Sandbox policy determining the isolation level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxPolicy {
    /// No sandboxing (for trusted operations)
    None,
    /// OS-native sandbox (Seatbelt/Bubblewrap/restricted tokens)
    OsNative {
        /// Writable paths (workspace, temp)
        writable_paths: Vec<String>,
        /// Network access allowed
        network: NetworkPolicy,
    },
    /// Docker container sandbox
    Container {
        image: String,
        writable_paths: Vec<String>,
        network: NetworkPolicy,
    },
}

/// Network access policy within the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkPolicy {
    /// No network access
    None,
    /// Only workspace-related hosts (e.g., git remote)
    WorkspaceOnly,
    /// Allowlisted hosts only
    Allowlist(Vec<String>),
    /// Full network access
    Full,
}

/// A command to execute, before sandbox transformation.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: std::collections::HashMap<String, String>,
    pub timeout_ms: u64,
}

/// The result of sandbox-transforming a command.
/// This is what actually gets executed by the OS.
#[derive(Debug, Clone)]
pub struct SandboxedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: std::collections::HashMap<String, String>,
    pub timeout_ms: u64,
}

/// Transform a command through the sandbox.
///
/// On macOS: wraps with `sandbox-exec -f <profile>`
/// On Linux: wraps with `bwrap` + Landlock rules
/// On Windows: creates restricted token
pub fn sandbox_transform(
    cmd: CommandSpec,
    policy: &SandboxPolicy,
) -> Result<SandboxedCommand> {
    match policy {
        SandboxPolicy::None => Ok(SandboxedCommand {
            program: cmd.program,
            args: cmd.args,
            cwd: cmd.cwd,
            env: cmd.env,
            timeout_ms: cmd.timeout_ms,
        }),
        SandboxPolicy::OsNative { .. } => {
            // TODO: Phase 4 — implement platform-specific sandbox
            // For now, pass through
            Ok(SandboxedCommand {
                program: cmd.program,
                args: cmd.args,
                cwd: cmd.cwd,
                env: cmd.env,
                timeout_ms: cmd.timeout_ms,
            })
        }
        SandboxPolicy::Container { .. } => {
            // TODO: Phase 4 — implement Docker sandbox
            Ok(SandboxedCommand {
                program: cmd.program,
                args: cmd.args,
                cwd: cmd.cwd,
                env: cmd.env,
                timeout_ms: cmd.timeout_ms,
            })
        }
    }
}
