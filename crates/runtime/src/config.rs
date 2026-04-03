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
                self.permission_mode = match mode.as_str() {
                    "accept-reads" => PermissionMode::AcceptReads,
                    "bypass" => PermissionMode::BypassPermissions,
                    "deny-all" => PermissionMode::DenyAll,
                    _ => PermissionMode::Default,
                };
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
