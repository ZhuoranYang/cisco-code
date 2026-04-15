//! Configuration system with hierarchical merging.
//!
//! Design insight from Claw-Code-Parity: Config hierarchy with
//! user < project < local < env < cli precedence.
//!
//! Design insight from Claude Code: CLAUDE.md files in project root for
//! project-specific instructions (we use cisco-code.md or CLAUDE.md).
//!
//! Format: TOML (cleaner than JSON for human editing).

use std::path::{Path, PathBuf};

use anyhow::Result;
use cisco_code_api::ThinkingConfig;
use serde::{Deserialize, Serialize};

/// Runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub model: String,
    /// Model class: "small", "medium", "large" — resolved by CLI to a specific model.
    pub model_class: Option<String>,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub max_budget_usd: Option<f64>,
    pub temperature: Option<f64>,
    pub permission_mode: PermissionMode,
    pub sandbox_mode: SandboxMode,
    /// Extended thinking config (Anthropic models only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Ask for every tool use
    Default,
    /// Allow read-only tools, ask for writes
    AcceptReads,
    /// Allow all tools without asking
    BypassPermissions,
    /// Deny all tool use
    DenyAll,
    /// Plan mode — read-only, no code changes allowed.
    /// Matches Claude Code's plan mode: the agent focuses on research,
    /// analysis, and planning. Only read-only tools are permitted.
    Plan,
}

impl PermissionMode {
    /// Convert to a string identifier (used for state transitions).
    pub fn as_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::AcceptReads => "accept_reads",
            Self::BypassPermissions => "bypass",
            Self::DenyAll => "deny_all",
            Self::Plan => "plan",
        }
    }

    /// Parse from a string identifier.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "default" => Self::Default,
            "accept_reads" | "accept-reads" => Self::AcceptReads,
            "bypass" => Self::BypassPermissions,
            "deny_all" | "deny-all" => Self::DenyAll,
            "plan" => Self::Plan,
            _ => Self::Default,
        }
    }
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
            model_class: None,
            max_tokens: 16384,
            max_turns: 50,
            max_budget_usd: None,
            temperature: None,
            permission_mode: PermissionMode::Default,
            sandbox_mode: SandboxMode::None,
            thinking: None,
        }
    }
}

/// Partial config used for TOML deserialization and merging.
/// All fields are optional so partial configs can be layered.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PartialConfig {
    pub general: Option<GeneralSection>,
    pub permissions: Option<PermissionsSection>,
    pub sandbox: Option<SandboxSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeneralSection {
    pub default_model: Option<String>,
    pub model_class: Option<String>,
    pub max_tokens: Option<u32>,
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionsSection {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SandboxSection {
    pub mode: Option<String>,
}

impl RuntimeConfig {
    /// Load configuration with hierarchical merging.
    ///
    /// Precedence (lowest → highest):
    /// 1. Defaults
    /// 2. User config (~/.cisco-code/config.toml)
    /// 3. Project config (.cisco-code/config.toml)
    /// 4. Environment variables
    pub fn load() -> Result<Self> {
        let mut config = Self::default();

        // Layer 1: User config
        if let Some(home) = dirs_home() {
            let user_config = home.join(".cisco-code").join("config.toml");
            if user_config.exists() {
                if let Ok(partial) = load_toml(&user_config) {
                    config.apply_partial(&partial);
                }
            }
        }

        // Layer 2: Project config
        let project_config = Path::new(".cisco-code").join("config.toml");
        if project_config.exists() {
            if let Ok(partial) = load_toml(&project_config) {
                config.apply_partial(&partial);
            }
        }

        // Layer 3: Environment variables
        config.apply_env();

        Ok(config)
    }

    /// Apply a partial config on top of this config.
    fn apply_partial(&mut self, partial: &PartialConfig) {
        if let Some(ref general) = partial.general {
            if let Some(ref model) = general.default_model {
                self.model = model.clone();
            }
            if let Some(ref class) = general.model_class {
                self.model_class = Some(class.clone());
            }
            if let Some(max_tokens) = general.max_tokens {
                self.max_tokens = max_tokens;
            }
            if let Some(max_turns) = general.max_turns {
                self.max_turns = max_turns;
            }
            if let Some(budget) = general.max_budget_usd {
                self.max_budget_usd = Some(budget);
            }
            if let Some(temp) = general.temperature {
                self.temperature = Some(temp);
            }
        }

        if let Some(ref perms) = partial.permissions {
            if let Some(ref mode) = perms.mode {
                self.permission_mode = PermissionMode::from_str_lossy(mode);
            }
        }

        if let Some(ref sandbox) = partial.sandbox {
            if let Some(ref mode) = sandbox.mode {
                self.sandbox_mode = match mode.as_str() {
                    "os-native" => SandboxMode::OsNative,
                    "container" => SandboxMode::Container,
                    _ => SandboxMode::None,
                };
            }
        }
    }

    /// Apply environment variable overrides.
    fn apply_env(&mut self) {
        if let Ok(model) = std::env::var("CISCO_CODE_MODEL") {
            self.model = model;
        }
        if let Ok(class) = std::env::var("CISCO_CODE_MODEL_CLASS") {
            self.model_class = Some(class);
        }
        if let Ok(tokens) = std::env::var("CISCO_CODE_MAX_TOKENS") {
            if let Ok(n) = tokens.parse() {
                self.max_tokens = n;
            }
        }
        if let Ok(turns) = std::env::var("CISCO_CODE_MAX_TURNS") {
            if let Ok(n) = turns.parse() {
                self.max_turns = n;
            }
        }
    }
}

fn load_toml(path: &Path) -> Result<PartialConfig> {
    let content = std::fs::read_to_string(path)?;
    let partial: PartialConfig = toml::from_str(&content)?;
    Ok(partial)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RuntimeConfig::default();
        assert_eq!(config.model, "claude-sonnet-4-6");
        assert!(config.model_class.is_none());
        assert_eq!(config.max_tokens, 16384);
        assert_eq!(config.max_turns, 50);
        assert!(config.max_budget_usd.is_none());
        assert!(config.temperature.is_none());
    }

    #[test]
    fn test_load_toml_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[general]
default_model = "claude-opus-4-6"
max_tokens = 8192
max_turns = 10
temperature = 0.5

[permissions]
mode = "bypass"

[sandbox]
mode = "os-native"
"#,
        )
        .unwrap();

        let partial = load_toml(&path).unwrap();
        let general = partial.general.unwrap();
        assert_eq!(general.default_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(general.max_tokens, Some(8192));
        assert_eq!(general.max_turns, Some(10));
        assert_eq!(general.temperature, Some(0.5));
        assert_eq!(partial.permissions.unwrap().mode.as_deref(), Some("bypass"));
        assert_eq!(partial.sandbox.unwrap().mode.as_deref(), Some("os-native"));
    }

    #[test]
    fn test_apply_partial() {
        let mut config = RuntimeConfig::default();
        let partial = PartialConfig {
            general: Some(GeneralSection {
                default_model: Some("gpt-5".into()),
                model_class: Some("large".into()),
                max_tokens: Some(2048),
                max_turns: None,
                max_budget_usd: Some(5.0),
                temperature: None,
            }),
            permissions: Some(PermissionsSection {
                mode: Some("accept-reads".into()),
            }),
            sandbox: None,
        };

        config.apply_partial(&partial);
        assert_eq!(config.model, "gpt-5");
        assert_eq!(config.model_class.as_deref(), Some("large"));
        assert_eq!(config.max_tokens, 2048);
        assert_eq!(config.max_turns, 50); // unchanged
        assert_eq!(config.max_budget_usd, Some(5.0));
        assert_eq!(config.permission_mode, PermissionMode::AcceptReads);
    }

    #[test]
    fn test_apply_empty_partial_is_noop() {
        let original = RuntimeConfig::default();
        let mut config = RuntimeConfig::default();
        config.apply_partial(&PartialConfig::default());

        assert_eq!(config.model, original.model);
        assert_eq!(config.max_tokens, original.max_tokens);
        assert_eq!(config.max_turns, original.max_turns);
    }

    #[test]
    fn test_partial_toml_missing_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("minimal.toml");
        std::fs::write(&path, "[general]\ndefault_model = \"test\"\n").unwrap();

        let partial = load_toml(&path).unwrap();
        assert!(partial.general.is_some());
        assert!(partial.permissions.is_none());
        assert!(partial.sandbox.is_none());
    }

    #[test]
    fn test_hierarchical_override() {
        // Simulate: user config sets model, project config overrides max_tokens
        let mut config = RuntimeConfig::default();

        let user_partial = PartialConfig {
            general: Some(GeneralSection {
                default_model: Some("user-model".into()),
                model_class: None,
                max_tokens: Some(4096),
                max_turns: None,
                max_budget_usd: None,
                temperature: None,
            }),
            permissions: None,
            sandbox: None,
        };
        config.apply_partial(&user_partial);
        assert_eq!(config.model, "user-model");
        assert_eq!(config.max_tokens, 4096);

        let project_partial = PartialConfig {
            general: Some(GeneralSection {
                default_model: None,
                model_class: None,
                max_tokens: Some(8192),
                max_turns: None,
                max_budget_usd: None,
                temperature: None,
            }),
            permissions: None,
            sandbox: None,
        };
        config.apply_partial(&project_partial);
        assert_eq!(config.model, "user-model"); // not overridden
        assert_eq!(config.max_tokens, 8192); // overridden
    }

    #[test]
    fn test_permission_mode_as_str() {
        assert_eq!(PermissionMode::Default.as_str(), "default");
        assert_eq!(PermissionMode::AcceptReads.as_str(), "accept_reads");
        assert_eq!(PermissionMode::BypassPermissions.as_str(), "bypass");
        assert_eq!(PermissionMode::DenyAll.as_str(), "deny_all");
        assert_eq!(PermissionMode::Plan.as_str(), "plan");
    }

    #[test]
    fn test_permission_mode_from_str_lossy() {
        assert_eq!(PermissionMode::from_str_lossy("default"), PermissionMode::Default);
        assert_eq!(PermissionMode::from_str_lossy("accept_reads"), PermissionMode::AcceptReads);
        assert_eq!(PermissionMode::from_str_lossy("accept-reads"), PermissionMode::AcceptReads);
        assert_eq!(PermissionMode::from_str_lossy("bypass"), PermissionMode::BypassPermissions);
        assert_eq!(PermissionMode::from_str_lossy("deny_all"), PermissionMode::DenyAll);
        assert_eq!(PermissionMode::from_str_lossy("deny-all"), PermissionMode::DenyAll);
        assert_eq!(PermissionMode::from_str_lossy("plan"), PermissionMode::Plan);
        assert_eq!(PermissionMode::from_str_lossy("unknown"), PermissionMode::Default);
    }

    #[test]
    fn test_permission_mode_plan_in_config() {
        let mut config = RuntimeConfig::default();
        let partial = PartialConfig {
            general: None,
            permissions: Some(PermissionsSection {
                mode: Some("plan".into()),
            }),
            sandbox: None,
        };
        config.apply_partial(&partial);
        assert_eq!(config.permission_mode, PermissionMode::Plan);
    }
}
