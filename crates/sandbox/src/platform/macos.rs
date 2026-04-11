//! macOS sandbox via Seatbelt (sandbox-exec).
//!
//! Seatbelt uses Apple's mandatory access control framework.
//! We generate a .sb profile dynamically based on the filesystem
//! and network policies, then wrap the command with `sandbox-exec -f`.

use anyhow::Result;

use crate::{CommandSpec, FilesystemPolicy, NetworkPolicy, SandboxMethod, SandboxedCommand};

/// Wrap a command with macOS Seatbelt sandbox.
pub fn wrap_with_seatbelt(
    cmd: CommandSpec,
    fs_policy: &FilesystemPolicy,
    network: &NetworkPolicy,
) -> Result<SandboxedCommand> {
    let profile = generate_seatbelt_profile(fs_policy, network, &cmd.cwd);

    // Write profile to a temp file
    let profile_path = write_temp_profile(&profile)?;

    // Build: sandbox-exec -f <profile> <program> <args...>
    let mut args = vec![
        "-f".to_string(),
        profile_path,
        cmd.program.clone(),
    ];
    args.extend(cmd.args.iter().cloned());

    Ok(SandboxedCommand {
        program: "sandbox-exec".into(),
        args,
        cwd: cmd.cwd,
        env: cmd.env,
        timeout_ms: cmd.timeout_ms,
        method: SandboxMethod::Seatbelt,
    })
}

/// Generate a Seatbelt .sb profile from policies.
fn generate_seatbelt_profile(
    fs_policy: &FilesystemPolicy,
    network: &NetworkPolicy,
    cwd: &str,
) -> String {
    let mut profile = String::new();
    profile.push_str("(version 1)\n");
    profile.push_str("(deny default)\n\n");

    // Allow basic process execution
    profile.push_str("(allow process-exec)\n");
    profile.push_str("(allow process-fork)\n");
    profile.push_str("(allow signal)\n\n");

    // Allow reading system libraries and executables
    profile.push_str("(allow file-read*\n");
    profile.push_str("  (subpath \"/usr\")\n");
    profile.push_str("  (subpath \"/bin\")\n");
    profile.push_str("  (subpath \"/sbin\")\n");
    profile.push_str("  (subpath \"/Library\")\n");
    profile.push_str("  (subpath \"/System\")\n");
    profile.push_str("  (subpath \"/private/var\")\n");
    profile.push_str("  (subpath \"/private/tmp\")\n");
    profile.push_str("  (subpath \"/var\")\n");
    profile.push_str("  (subpath \"/tmp\")\n");
    profile.push_str("  (subpath \"/dev\")\n");
    profile.push_str("  (subpath \"/etc\")\n");
    profile.push_str("  (subpath \"/opt\")\n");

    // Allow reading the workspace
    profile.push_str(&format!("  (subpath \"{}\")\n", cwd));

    for path in &fs_policy.allow_read {
        profile.push_str(&format!("  (subpath \"{}\")\n", path));
    }

    profile.push_str(")\n\n");

    // Deny reading specific paths
    for path in &fs_policy.deny_read {
        profile.push_str(&format!(
            "(deny file-read* (subpath \"{}\"))\n",
            path
        ));
    }

    // Allow writing to workspace and allowed paths
    profile.push_str("(allow file-write*\n");
    profile.push_str(&format!("  (subpath \"{}\")\n", cwd));
    profile.push_str("  (subpath \"/tmp\")\n");
    profile.push_str("  (subpath \"/private/tmp\")\n");
    profile.push_str("  (subpath \"/dev\")\n");

    for path in &fs_policy.allow_write {
        profile.push_str(&format!("  (subpath \"{}\")\n", path));
    }

    profile.push_str(")\n\n");

    // Deny writing to specific paths (overrides allow)
    for path in &fs_policy.deny_write {
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            path
        ));
    }

    // Network policy
    match network {
        NetworkPolicy::None => {
            profile.push_str("\n(deny network*)\n");
        }
        NetworkPolicy::Full => {
            profile.push_str("\n(allow network*)\n");
        }
        NetworkPolicy::WorkspaceOnly | NetworkPolicy::Allowlist(_) => {
            // Seatbelt can't do domain-level filtering easily,
            // so we allow network but rely on proxy/firewall for fine-grained control
            profile.push_str("\n(allow network*)\n");
        }
    }

    // Allow sysctl, mach, and IPC (needed for most programs)
    profile.push_str("\n(allow sysctl-read)\n");
    profile.push_str("(allow mach-lookup)\n");
    profile.push_str("(allow ipc-posix-shm-read-data)\n");
    profile.push_str("(allow ipc-posix-shm-write-data)\n");

    profile
}

/// Write a seatbelt profile to a temp file and return the path.
fn write_temp_profile(profile: &str) -> Result<String> {
    let dir = std::env::temp_dir().join("cisco-code-sandbox");
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(format!("profile-{}.sb", std::process::id()));
    std::fs::write(&path, profile)?;

    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_profile_basic() {
        let fs_policy = FilesystemPolicy::default();
        let profile = generate_seatbelt_profile(&fs_policy, &NetworkPolicy::Full, "/workspace");

        assert!(profile.contains("(version 1)"));
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(allow process-exec)"));
        assert!(profile.contains("/workspace"));
        assert!(profile.contains("(allow network*)"));
    }

    #[test]
    fn test_generate_profile_no_network() {
        let fs_policy = FilesystemPolicy::default();
        let profile = generate_seatbelt_profile(&fs_policy, &NetworkPolicy::None, "/tmp");

        assert!(profile.contains("(deny network*)"));
    }

    #[test]
    fn test_generate_profile_deny_read() {
        let fs_policy = FilesystemPolicy {
            deny_read: vec!["/secret".into()],
            ..Default::default()
        };
        let profile = generate_seatbelt_profile(&fs_policy, &NetworkPolicy::Full, "/workspace");

        assert!(profile.contains("(deny file-read* (subpath \"/secret\"))"));
    }

    #[test]
    fn test_generate_profile_deny_write() {
        let fs_policy = FilesystemPolicy {
            deny_write: vec![".git/hooks".into()],
            ..Default::default()
        };
        let profile = generate_seatbelt_profile(&fs_policy, &NetworkPolicy::Full, "/workspace");

        assert!(profile.contains("(deny file-write* (subpath \".git/hooks\"))"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_wrap_with_seatbelt() {
        use std::collections::HashMap;
        let cmd = CommandSpec {
            program: "echo".into(),
            args: vec!["hello".into()],
            cwd: "/tmp".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };
        let fs_policy = FilesystemPolicy::default();
        let result = wrap_with_seatbelt(cmd, &fs_policy, &NetworkPolicy::Full).unwrap();

        assert_eq!(result.program, "sandbox-exec");
        assert_eq!(result.method, SandboxMethod::Seatbelt);
        assert!(result.args.contains(&"-f".to_string()));
        assert!(result.args.contains(&"echo".to_string()));
    }
}
