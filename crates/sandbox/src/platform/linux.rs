//! Linux sandbox via Bubblewrap (bwrap).
//!
//! Bubblewrap uses user namespaces to create lightweight containers.
//! We build the bwrap command with bind-mounts for allowed paths
//! and devirtualize the rest.

use anyhow::Result;
use std::collections::HashMap;

use crate::{CommandSpec, FilesystemPolicy, NetworkPolicy, SandboxMethod, SandboxedCommand};

/// Wrap a command with Linux Bubblewrap sandbox.
pub fn wrap_with_bwrap(
    cmd: CommandSpec,
    fs_policy: &FilesystemPolicy,
    network: &NetworkPolicy,
) -> Result<SandboxedCommand> {
    let mut args = Vec::new();

    // Create new namespaces
    args.push("--unshare-pid".into());
    args.push("--die-with-parent".into());

    // Network namespace (if restricted)
    match network {
        NetworkPolicy::None => {
            args.push("--unshare-net".into());
        }
        _ => {
            // Allow network access (don't unshare net namespace)
        }
    }

    // Mount /proc
    args.push("--proc".into());
    args.push("/proc".into());

    // Mount /dev
    args.push("--dev".into());
    args.push("/dev".into());

    // Read-only bind-mount standard system paths
    for path in &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc", "/opt", "/var", "/run"] {
        if std::path::Path::new(path).exists() {
            args.push("--ro-bind".into());
            args.push(path.to_string());
            args.push(path.to_string());
        }
    }

    // Writable /tmp
    args.push("--tmpfs".into());
    args.push("/tmp".into());

    // Read-write bind-mount the workspace
    args.push("--bind".into());
    args.push(cmd.cwd.clone());
    args.push(cmd.cwd.clone());

    // Additional writable paths
    for path in &fs_policy.allow_write {
        if std::path::Path::new(path).exists() {
            args.push("--bind".into());
            args.push(path.clone());
            args.push(path.clone());
        }
    }

    // Additional read-only paths
    for path in &fs_policy.allow_read {
        if std::path::Path::new(path).exists() {
            args.push("--ro-bind".into());
            args.push(path.clone());
            args.push(path.clone());
        }
    }

    // Home directory (read-only by default, unless in allow_write)
    if let Ok(home) = std::env::var("HOME") {
        let home_writable = fs_policy
            .allow_write
            .iter()
            .any(|p| p.starts_with(&home));

        if home_writable {
            args.push("--bind".into());
        } else {
            args.push("--ro-bind".into());
        }
        args.push(home.clone());
        args.push(home);
    }

    // Set working directory
    args.push("--chdir".into());
    args.push(cmd.cwd.clone());

    // The actual command to run
    args.push(cmd.program.clone());
    args.extend(cmd.args.iter().cloned());

    Ok(SandboxedCommand {
        program: "bwrap".into(),
        args,
        cwd: cmd.cwd,
        env: cmd.env,
        timeout_ms: cmd.timeout_ms,
        method: SandboxMethod::Bubblewrap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_basic() {
        let cmd = CommandSpec {
            program: "ls".into(),
            args: vec!["-la".into()],
            cwd: "/tmp/test".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };
        let fs_policy = FilesystemPolicy::default();
        let result = wrap_with_bwrap(cmd, &fs_policy, &NetworkPolicy::Full).unwrap();

        assert_eq!(result.program, "bwrap");
        assert_eq!(result.method, SandboxMethod::Bubblewrap);
        assert!(result.args.contains(&"--unshare-pid".to_string()));
        assert!(result.args.contains(&"--die-with-parent".to_string()));
        assert!(result.args.contains(&"ls".to_string()));
        assert!(result.args.contains(&"-la".to_string()));
    }

    #[test]
    fn test_wrap_no_network() {
        let cmd = CommandSpec {
            program: "curl".into(),
            args: vec!["https://example.com".into()],
            cwd: "/tmp".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };
        let fs_policy = FilesystemPolicy::default();
        let result = wrap_with_bwrap(cmd, &fs_policy, &NetworkPolicy::None).unwrap();

        assert!(result.args.contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn test_wrap_with_writable_paths() {
        let cmd = CommandSpec {
            program: "touch".into(),
            args: vec!["/tmp/test/file.txt".into()],
            cwd: "/tmp/test".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };
        let fs_policy = FilesystemPolicy {
            allow_write: vec!["/tmp/extra".into()],
            ..Default::default()
        };
        let result = wrap_with_bwrap(cmd, &fs_policy, &NetworkPolicy::Full).unwrap();

        // Workspace should be bind-mounted writable
        let bind_idx = result
            .args
            .iter()
            .position(|a| a == "--bind")
            .unwrap();
        assert_eq!(result.args[bind_idx + 1], "/tmp/test");
    }

    #[test]
    fn test_wrap_sets_chdir() {
        let cmd = CommandSpec {
            program: "pwd".into(),
            args: vec![],
            cwd: "/workspace/project".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };
        let fs_policy = FilesystemPolicy::default();
        let result = wrap_with_bwrap(cmd, &fs_policy, &NetworkPolicy::Full).unwrap();

        let chdir_idx = result
            .args
            .iter()
            .position(|a| a == "--chdir")
            .unwrap();
        assert_eq!(result.args[chdir_idx + 1], "/workspace/project");
    }
}
