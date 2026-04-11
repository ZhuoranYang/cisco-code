//! cisco-code-sandbox: OS-native sandboxing for tool execution.
//!
//! Design insight from Codex: Don't sandbox the tool — sandbox the *execution*.
//! The CommandSpec → SandboxedCommand pipeline means tool authors write normal
//! code, and the sandbox layer wraps it transparently.
//!
//! Platforms:
//! - macOS: Seatbelt (sandbox-exec with custom .sb profile)
//! - Linux: Bubblewrap (bwrap user namespaces) + optional Landlock
//! - All: Filesystem deny/allow lists, network policy, timeout enforcement

pub mod platform;
pub mod profile;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Sandbox policy determining the isolation level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxPolicy {
    /// No sandboxing (for trusted operations).
    None,
    /// OS-native sandbox (Seatbelt/Bubblewrap).
    OsNative {
        /// Writable paths (workspace, temp).
        writable_paths: Vec<String>,
        /// Read-denied paths.
        deny_read: Vec<String>,
        /// Network access policy.
        network: NetworkPolicy,
    },
    /// Docker container sandbox.
    Container {
        image: String,
        writable_paths: Vec<String>,
        network: NetworkPolicy,
    },
}

/// Network access policy within the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkPolicy {
    /// No network access.
    None,
    /// Only workspace-related hosts (e.g., git remote).
    WorkspaceOnly,
    /// Allowlisted domains only.
    Allowlist(Vec<String>),
    /// Full network access.
    Full,
}

/// A command to execute, before sandbox transformation.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub timeout_ms: u64,
}

/// The result of sandbox-transforming a command.
/// This is what actually gets executed by the OS.
#[derive(Debug, Clone)]
pub struct SandboxedCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub timeout_ms: u64,
    /// The sandbox method applied.
    pub method: SandboxMethod,
}

/// How the command was sandboxed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxMethod {
    /// No sandboxing applied.
    None,
    /// macOS Seatbelt (sandbox-exec).
    Seatbelt,
    /// Linux Bubblewrap (bwrap).
    Bubblewrap,
    /// Docker container.
    Container,
}

/// Filesystem restrictions for the sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    /// Paths that are writable (workspace, temp).
    pub allow_write: Vec<String>,
    /// Paths that are explicitly denied for writing.
    pub deny_write: Vec<String>,
    /// Paths that are denied for reading.
    pub deny_read: Vec<String>,
    /// Additional read-only paths.
    pub allow_read: Vec<String>,
}

/// Check if sandbox dependencies are available on this platform.
pub fn check_dependencies() -> SandboxDependencies {
    let platform = std::env::consts::OS;

    let seatbelt_available = platform == "macos" && {
        std::process::Command::new("sandbox-exec")
            .arg("-n")
            .arg("no-network")
            .arg("true")
            .output()
            .is_ok()
    };

    let bwrap_available = (platform == "linux") && {
        std::process::Command::new("bwrap")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };

    let docker_available = std::process::Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    SandboxDependencies {
        platform: platform.into(),
        seatbelt: seatbelt_available,
        bubblewrap: bwrap_available,
        docker: docker_available,
    }
}

/// Status of sandbox dependencies on this system.
#[derive(Debug, Clone)]
pub struct SandboxDependencies {
    pub platform: String,
    pub seatbelt: bool,
    pub bubblewrap: bool,
    pub docker: bool,
}

impl SandboxDependencies {
    /// Whether any OS-native sandbox is available.
    pub fn has_os_native(&self) -> bool {
        self.seatbelt || self.bubblewrap
    }

    /// Best available sandbox method for this platform.
    pub fn best_method(&self) -> SandboxMethod {
        if self.seatbelt {
            SandboxMethod::Seatbelt
        } else if self.bubblewrap {
            SandboxMethod::Bubblewrap
        } else if self.docker {
            SandboxMethod::Container
        } else {
            SandboxMethod::None
        }
    }
}

/// Transform a command through the sandbox.
///
/// On macOS: wraps with `sandbox-exec -f <profile>`
/// On Linux: wraps with `bwrap` + filesystem/network rules
/// On other: passes through (no sandbox)
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
            method: SandboxMethod::None,
        }),

        SandboxPolicy::OsNative {
            writable_paths,
            deny_read,
            network,
        } => {
            let fs_policy = FilesystemPolicy {
                allow_write: writable_paths.clone(),
                deny_write: vec![],
                deny_read: deny_read.clone(),
                allow_read: vec![],
            };

            match std::env::consts::OS {
                "macos" => platform::macos::wrap_with_seatbelt(cmd, &fs_policy, network),
                "linux" => platform::linux::wrap_with_bwrap(cmd, &fs_policy, network),
                os => {
                    tracing::warn!("No OS-native sandbox for {os}, running unsandboxed");
                    Ok(SandboxedCommand {
                        program: cmd.program,
                        args: cmd.args,
                        cwd: cmd.cwd,
                        env: cmd.env,
                        timeout_ms: cmd.timeout_ms,
                        method: SandboxMethod::None,
                    })
                }
            }
        }

        SandboxPolicy::Container {
            image,
            writable_paths,
            network,
        } => platform::container::wrap_with_docker(cmd, image, writable_paths, network),
    }
}

/// Check if a path is safe to write (not in deny list).
pub fn is_path_allowed(path: &Path, policy: &FilesystemPolicy) -> bool {
    let path_str = path.to_string_lossy();

    // Check deny list first
    for denied in &policy.deny_write {
        if path_str.starts_with(denied) {
            return false;
        }
    }

    // Check allow list
    if policy.allow_write.is_empty() {
        return true; // No restrictions
    }

    for allowed in &policy.allow_write {
        if path_str.starts_with(allowed) {
            return true;
        }
    }

    false
}

/// Default deny-write paths (sensitive files that should never be modified by tools).
pub fn default_deny_write_paths() -> Vec<String> {
    vec![
        ".claude/settings.json".into(),
        ".cisco-code/settings.json".into(),
        ".git/config".into(),
        ".git/hooks".into(),
        ".ssh".into(),
        ".gnupg".into(),
        ".env".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_policy_none() {
        let cmd = CommandSpec {
            program: "echo".into(),
            args: vec!["hello".into()],
            cwd: ".".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };

        let result = sandbox_transform(cmd, &SandboxPolicy::None).unwrap();
        assert_eq!(result.program, "echo");
        assert_eq!(result.method, SandboxMethod::None);
    }

    #[test]
    fn test_is_path_allowed_no_restrictions() {
        let policy = FilesystemPolicy::default();
        assert!(is_path_allowed(Path::new("/tmp/test"), &policy));
    }

    #[test]
    fn test_is_path_allowed_in_allow_list() {
        let policy = FilesystemPolicy {
            allow_write: vec!["/workspace".into(), "/tmp".into()],
            ..Default::default()
        };
        assert!(is_path_allowed(Path::new("/workspace/src/main.rs"), &policy));
        assert!(is_path_allowed(Path::new("/tmp/scratch"), &policy));
        assert!(!is_path_allowed(Path::new("/etc/passwd"), &policy));
    }

    #[test]
    fn test_is_path_denied() {
        let policy = FilesystemPolicy {
            deny_write: vec![".git/hooks".into(), ".ssh".into()],
            ..Default::default()
        };
        assert!(!is_path_allowed(Path::new(".git/hooks/pre-commit"), &policy));
        assert!(!is_path_allowed(Path::new(".ssh/id_rsa"), &policy));
        assert!(is_path_allowed(Path::new("src/main.rs"), &policy));
    }

    #[test]
    fn test_deny_takes_precedence() {
        let policy = FilesystemPolicy {
            allow_write: vec![".git".into()],
            deny_write: vec![".git/hooks".into()],
            ..Default::default()
        };
        assert!(!is_path_allowed(Path::new(".git/hooks/pre-commit"), &policy));
        assert!(is_path_allowed(Path::new(".git/config"), &policy));
    }

    #[test]
    fn test_default_deny_paths() {
        let deny = default_deny_write_paths();
        assert!(deny.contains(&".git/config".into()));
        assert!(deny.contains(&".ssh".into()));
        assert!(deny.contains(&".env".into()));
    }

    #[test]
    fn test_sandbox_dependencies_best_method() {
        let deps = SandboxDependencies {
            platform: "macos".into(),
            seatbelt: true,
            bubblewrap: false,
            docker: true,
        };
        assert_eq!(deps.best_method(), SandboxMethod::Seatbelt);

        let deps = SandboxDependencies {
            platform: "linux".into(),
            seatbelt: false,
            bubblewrap: true,
            docker: false,
        };
        assert_eq!(deps.best_method(), SandboxMethod::Bubblewrap);

        let deps = SandboxDependencies {
            platform: "windows".into(),
            seatbelt: false,
            bubblewrap: false,
            docker: false,
        };
        assert_eq!(deps.best_method(), SandboxMethod::None);
    }

    #[test]
    fn test_sandbox_method_equality() {
        assert_eq!(SandboxMethod::Seatbelt, SandboxMethod::Seatbelt);
        assert_ne!(SandboxMethod::Seatbelt, SandboxMethod::Bubblewrap);
    }

    #[test]
    fn test_filesystem_policy_default() {
        let policy = FilesystemPolicy::default();
        assert!(policy.allow_write.is_empty());
        assert!(policy.deny_write.is_empty());
        assert!(policy.deny_read.is_empty());
    }
}
