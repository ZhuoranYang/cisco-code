//! Sandbox profile presets for common tool execution scenarios.
//!
//! Profiles combine filesystem, network, and resource policies into
//! reusable configurations. Tool authors pick a profile rather than
//! hand-building policies.

use crate::{NetworkPolicy, SandboxPolicy};

/// A named sandbox profile preset.
#[derive(Debug, Clone)]
pub struct SandboxProfile {
    /// Profile name (e.g., "read-only", "shell", "network").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// The sandbox policy this profile expands to.
    pub policy: SandboxPolicy,
}

impl SandboxProfile {
    /// Read-only profile: tools can read but not write (except /tmp).
    pub fn read_only(_cwd: &str) -> Self {
        Self {
            name: "read-only".into(),
            description: "Read-only access to workspace, no network".into(),
            policy: SandboxPolicy::OsNative {
                writable_paths: vec!["/tmp".into()],
                deny_read: vec![],
                network: NetworkPolicy::None,
            },
        }
    }

    /// Shell profile: read/write workspace, no network by default.
    pub fn shell(cwd: &str) -> Self {
        Self {
            name: "shell".into(),
            description: "Read/write workspace access, no network".into(),
            policy: SandboxPolicy::OsNative {
                writable_paths: vec![cwd.to_string(), "/tmp".into()],
                deny_read: vec![],
                network: NetworkPolicy::None,
            },
        }
    }

    /// Network profile: read/write workspace + full network.
    pub fn network(cwd: &str) -> Self {
        Self {
            name: "network".into(),
            description: "Read/write workspace access with full network".into(),
            policy: SandboxPolicy::OsNative {
                writable_paths: vec![cwd.to_string(), "/tmp".into()],
                deny_read: vec![],
                network: NetworkPolicy::Full,
            },
        }
    }

    /// Restricted profile: read-only workspace, no network, deny sensitive paths.
    pub fn restricted(_cwd: &str) -> Self {
        Self {
            name: "restricted".into(),
            description: "Minimal access — read-only, no network, deny secrets".into(),
            policy: SandboxPolicy::OsNative {
                writable_paths: vec!["/tmp".into()],
                deny_read: vec![
                    ".ssh".into(),
                    ".gnupg".into(),
                    ".env".into(),
                    ".aws".into(),
                ],
                network: NetworkPolicy::None,
            },
        }
    }

    /// Unrestricted profile: no sandbox.
    pub fn unrestricted() -> Self {
        Self {
            name: "unrestricted".into(),
            description: "No sandboxing — full system access".into(),
            policy: SandboxPolicy::None,
        }
    }

    /// Container profile: run in a Docker container.
    pub fn container(cwd: &str, image: &str) -> Self {
        Self {
            name: "container".into(),
            description: format!("Docker container ({image})"),
            policy: SandboxPolicy::Container {
                image: image.to_string(),
                writable_paths: vec![cwd.to_string()],
                network: NetworkPolicy::Full,
            },
        }
    }
}

/// Look up a profile by name.
pub fn profile_by_name(name: &str, cwd: &str) -> Option<SandboxProfile> {
    match name {
        "read-only" => Some(SandboxProfile::read_only(cwd)),
        "shell" => Some(SandboxProfile::shell(cwd)),
        "network" => Some(SandboxProfile::network(cwd)),
        "restricted" => Some(SandboxProfile::restricted(cwd)),
        "unrestricted" => Some(SandboxProfile::unrestricted()),
        _ => None,
    }
}

/// List all available built-in profiles.
pub fn builtin_profiles(cwd: &str) -> Vec<SandboxProfile> {
    vec![
        SandboxProfile::read_only(cwd),
        SandboxProfile::shell(cwd),
        SandboxProfile::network(cwd),
        SandboxProfile::restricted(cwd),
        SandboxProfile::unrestricted(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_only_profile() {
        let profile = SandboxProfile::read_only("/workspace");
        assert_eq!(profile.name, "read-only");
        match &profile.policy {
            SandboxPolicy::OsNative {
                writable_paths,
                network,
                ..
            } => {
                assert!(writable_paths.contains(&"/tmp".to_string()));
                assert!(matches!(network, NetworkPolicy::None));
            }
            _ => panic!("Expected OsNative policy"),
        }
    }

    #[test]
    fn test_shell_profile() {
        let profile = SandboxProfile::shell("/workspace");
        assert_eq!(profile.name, "shell");
        match &profile.policy {
            SandboxPolicy::OsNative {
                writable_paths,
                network,
                ..
            } => {
                assert!(writable_paths.contains(&"/workspace".to_string()));
                assert!(matches!(network, NetworkPolicy::None));
            }
            _ => panic!("Expected OsNative policy"),
        }
    }

    #[test]
    fn test_network_profile() {
        let profile = SandboxProfile::network("/workspace");
        match &profile.policy {
            SandboxPolicy::OsNative { network, .. } => {
                assert!(matches!(network, NetworkPolicy::Full));
            }
            _ => panic!("Expected OsNative policy"),
        }
    }

    #[test]
    fn test_restricted_profile() {
        let profile = SandboxProfile::restricted("/workspace");
        match &profile.policy {
            SandboxPolicy::OsNative { deny_read, .. } => {
                assert!(deny_read.contains(&".ssh".to_string()));
                assert!(deny_read.contains(&".env".to_string()));
            }
            _ => panic!("Expected OsNative policy"),
        }
    }

    #[test]
    fn test_unrestricted_profile() {
        let profile = SandboxProfile::unrestricted();
        assert!(matches!(profile.policy, SandboxPolicy::None));
    }

    #[test]
    fn test_container_profile() {
        let profile = SandboxProfile::container("/workspace", "python:3.12");
        match &profile.policy {
            SandboxPolicy::Container { image, .. } => {
                assert_eq!(image, "python:3.12");
            }
            _ => panic!("Expected Container policy"),
        }
    }

    #[test]
    fn test_profile_by_name() {
        assert!(profile_by_name("shell", "/workspace").is_some());
        assert!(profile_by_name("read-only", "/workspace").is_some());
        assert!(profile_by_name("nonexistent", "/workspace").is_none());
    }

    #[test]
    fn test_builtin_profiles() {
        let profiles = builtin_profiles("/workspace");
        assert_eq!(profiles.len(), 5);
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"read-only"));
        assert!(names.contains(&"shell"));
        assert!(names.contains(&"network"));
        assert!(names.contains(&"restricted"));
        assert!(names.contains(&"unrestricted"));
    }
}
