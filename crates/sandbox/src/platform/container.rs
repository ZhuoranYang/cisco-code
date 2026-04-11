//! Docker container sandbox.
//!
//! Wraps commands in `docker run` with volume mounts and network restrictions.

use anyhow::Result;

use crate::{CommandSpec, NetworkPolicy, SandboxMethod, SandboxedCommand};

/// Wrap a command with Docker container sandbox.
pub fn wrap_with_docker(
    cmd: CommandSpec,
    image: &str,
    writable_paths: &[String],
    network: &NetworkPolicy,
) -> Result<SandboxedCommand> {
    let mut args = vec![
        "run".into(),
        "--rm".into(),
        "-i".into(),
    ];

    // Network mode
    match network {
        NetworkPolicy::None => {
            args.push("--network".into());
            args.push("none".into());
        }
        NetworkPolicy::Full => {
            // Default Docker network (bridge)
        }
        NetworkPolicy::WorkspaceOnly | NetworkPolicy::Allowlist(_) => {
            // Docker can't do domain-level filtering easily;
            // use bridge network and rely on iptables/proxy
            args.push("--network".into());
            args.push("bridge".into());
        }
    }

    // Mount workspace
    args.push("-v".into());
    args.push(format!("{}:/workspace", cmd.cwd));
    args.push("-w".into());
    args.push("/workspace".into());

    // Mount additional writable paths
    for path in writable_paths {
        args.push("-v".into());
        args.push(format!("{path}:{path}"));
    }

    // Environment variables
    for (k, v) in &cmd.env {
        args.push("-e".into());
        args.push(format!("{k}={v}"));
    }

    // Image
    args.push(image.into());

    // Command to run inside container
    args.push(cmd.program.clone());
    args.extend(cmd.args.iter().cloned());

    Ok(SandboxedCommand {
        program: "docker".into(),
        args,
        cwd: cmd.cwd,
        env: std::collections::HashMap::new(), // env is passed via -e flags
        timeout_ms: cmd.timeout_ms,
        method: SandboxMethod::Container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_docker_wrap_basic() {
        let cmd = CommandSpec {
            program: "python".into(),
            args: vec!["script.py".into()],
            cwd: "/workspace".into(),
            env: HashMap::new(),
            timeout_ms: 30000,
        };

        let result =
            wrap_with_docker(cmd, "python:3.12-slim", &[], &NetworkPolicy::Full).unwrap();

        assert_eq!(result.program, "docker");
        assert_eq!(result.method, SandboxMethod::Container);
        assert!(result.args.contains(&"run".to_string()));
        assert!(result.args.contains(&"--rm".to_string()));
        assert!(result.args.contains(&"python:3.12-slim".to_string()));
        assert!(result.args.contains(&"python".to_string()));
        assert!(result.args.contains(&"script.py".to_string()));
    }

    #[test]
    fn test_docker_no_network() {
        let cmd = CommandSpec {
            program: "ls".into(),
            args: vec![],
            cwd: "/workspace".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };

        let result =
            wrap_with_docker(cmd, "alpine:latest", &[], &NetworkPolicy::None).unwrap();

        let net_idx = result
            .args
            .iter()
            .position(|a| a == "--network")
            .unwrap();
        assert_eq!(result.args[net_idx + 1], "none");
    }

    #[test]
    fn test_docker_with_env() {
        let mut env = HashMap::new();
        env.insert("API_KEY".into(), "secret".into());

        let cmd = CommandSpec {
            program: "curl".into(),
            args: vec!["https://api.example.com".into()],
            cwd: "/workspace".into(),
            env,
            timeout_ms: 10000,
        };

        let result =
            wrap_with_docker(cmd, "curlimages/curl", &[], &NetworkPolicy::Full).unwrap();

        assert!(result.args.contains(&"-e".to_string()));
        assert!(result.args.contains(&"API_KEY=secret".to_string()));
    }

    #[test]
    fn test_docker_workspace_mount() {
        let cmd = CommandSpec {
            program: "cat".into(),
            args: vec!["README.md".into()],
            cwd: "/home/user/project".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };

        let result = wrap_with_docker(cmd, "alpine", &[], &NetworkPolicy::Full).unwrap();

        assert!(result
            .args
            .contains(&"/home/user/project:/workspace".to_string()));
        assert!(result.args.contains(&"-w".to_string()));
    }

    #[test]
    fn test_docker_extra_mounts() {
        let cmd = CommandSpec {
            program: "ls".into(),
            args: vec![],
            cwd: "/workspace".into(),
            env: HashMap::new(),
            timeout_ms: 5000,
        };

        let result = wrap_with_docker(
            cmd,
            "alpine",
            &["/data/models".into(), "/cache".into()],
            &NetworkPolicy::Full,
        )
        .unwrap();

        assert!(result.args.contains(&"/data/models:/data/models".to_string()));
        assert!(result.args.contains(&"/cache:/cache".to_string()));
    }
}
