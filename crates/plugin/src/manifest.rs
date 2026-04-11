//! Plugin manifest parsing and validation.
//!
//! A plugin manifest declares the plugin's metadata and the capabilities it
//! provides: slash commands, hooks, tools, MCP server configs, and settings.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The top-level plugin manifest, deserialized from `plugin.toml` or `plugin.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin name (e.g. "cisco-security-scanner").
    pub name: String,
    /// SemVer version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// Plugin author.
    #[serde(default)]
    pub author: Option<String>,
    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Minimum cisco-code version required.
    #[serde(default)]
    pub min_agent_version: Option<String>,
    /// High-level capability categories this plugin provides.
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
    /// Slash commands the plugin provides.
    #[serde(default)]
    pub commands: Vec<PluginCommand>,
    /// Hook definitions.
    #[serde(default)]
    pub hooks: Vec<PluginHook>,
    /// Custom tool definitions.
    #[serde(default)]
    pub tools: Vec<PluginTool>,
    /// MCP server configurations.
    #[serde(default)]
    pub mcp_servers: Vec<PluginMcpServer>,
    /// Configurable settings exposed to the user.
    #[serde(default)]
    pub settings: Vec<PluginSetting>,
}

/// Broad capability categories a plugin can declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    Commands,
    Hooks,
    Tools,
    McpServers,
    Agents,
}

/// A slash command provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCommand {
    /// Command name (without the "/" prefix).
    pub name: String,
    /// Short description shown in /help.
    pub description: String,
    /// Prompt template. `{{args}}` is replaced with user arguments.
    pub template: String,
    /// Alternative names that also invoke this command.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A hook definition provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginHook {
    /// The lifecycle event (e.g. "pre_tool_use", "session_start").
    pub event: String,
    /// Shell command to execute when the hook fires.
    pub command: String,
    /// Optional tool-name filter (supports trailing wildcard).
    #[serde(default)]
    pub tool_filter: Option<String>,
    /// Timeout in milliseconds; defaults to 5000 if absent.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// A custom tool definition provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginTool {
    /// Tool name as seen by the LLM.
    pub name: String,
    /// Tool description for the LLM.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub input_schema: serde_json::Value,
    /// Shell command to execute. Receives tool input as JSON on stdin.
    pub command: String,
}

/// An MCP server configuration provided by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMcpServer {
    /// Logical server name.
    pub name: String,
    /// Command to launch the server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A configurable setting that a plugin exposes to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSetting {
    /// Setting key (e.g. "scanner.severity_threshold").
    pub key: String,
    /// Human-readable description.
    pub description: String,
    /// The type of the setting value.
    pub setting_type: SettingType,
    /// Default value (optional).
    #[serde(default)]
    pub default_value: Option<serde_json::Value>,
}

/// Setting value type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    Number,
    Boolean,
    Choice(Vec<std::string::String>),
}

impl PluginManifest {
    /// Parse a manifest from TOML content.
    pub fn from_toml(content: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(content)?;
        Ok(manifest)
    }

    /// Parse a manifest from JSON content.
    pub fn from_json(content: &str) -> anyhow::Result<Self> {
        let manifest: Self = serde_json::from_str(content)?;
        Ok(manifest)
    }

    /// Validate the manifest for correctness.
    ///
    /// Checks:
    /// - name is non-empty
    /// - version looks like a valid semver (major.minor.patch)
    /// - no duplicate command names (including aliases)
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.trim().is_empty() {
            anyhow::bail!("plugin name must not be empty");
        }

        // Basic semver check: must have at least major.minor.patch with numeric parts.
        let parts: Vec<&str> = self.version.split('.').collect();
        if parts.len() < 3 {
            anyhow::bail!(
                "plugin version '{}' is not valid semver (expected major.minor.patch)",
                self.version
            );
        }
        for (i, part) in parts.iter().take(3).enumerate() {
            if part.parse::<u64>().is_err() {
                let label = ["major", "minor", "patch"][i];
                anyhow::bail!(
                    "plugin version '{}': {} component '{}' is not a valid number",
                    self.version,
                    label,
                    part
                );
            }
        }

        // Check for duplicate command names (including aliases).
        let mut seen = std::collections::HashSet::new();
        for cmd in &self.commands {
            if !seen.insert(&cmd.name) {
                anyhow::bail!("duplicate command name: '{}'", cmd.name);
            }
            for alias in &cmd.aliases {
                if !seen.insert(alias) {
                    anyhow::bail!("duplicate command name/alias: '{}'", alias);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_toml() -> &'static str {
        r#"
name = "test-plugin"
version = "1.0.0"
description = "A test plugin"
author = "Test Author"
homepage = "https://example.com"
license = "MIT"
min_agent_version = "0.1.0"
capabilities = ["commands", "hooks", "tools"]

[[commands]]
name = "deploy"
description = "Deploy to staging"
template = "Run deployment for {{args}}"
aliases = ["d"]

[[commands]]
name = "lint"
description = "Run linter"
template = "Lint the codebase: {{args}}"

[[hooks]]
event = "pre_tool_use"
command = "echo checking"
tool_filter = "Bash"
timeout_ms = 3000

[[tools]]
name = "security-scan"
description = "Run a security scan"
input_schema = { type = "object", properties = { target = { type = "string" } } }
command = "security-scanner --json"

[[mcp_servers]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]

[mcp_servers.env]
GITHUB_TOKEN = "placeholder"

[[settings]]
key = "deploy.environment"
description = "Target deployment environment"
setting_type = { choice = ["staging", "production"] }
default_value = "staging"
"#
    }

    fn sample_json() -> &'static str {
        r#"{
  "name": "json-plugin",
  "version": "2.1.0",
  "description": "A JSON plugin",
  "capabilities": ["tools"],
  "commands": [],
  "hooks": [],
  "tools": [
    {
      "name": "formatter",
      "description": "Format code",
      "input_schema": {"type": "object"},
      "command": "fmt --stdin"
    }
  ],
  "mcp_servers": [],
  "settings": []
}"#
    }

    #[test]
    fn test_parse_toml_full() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description, "A test plugin");
        assert_eq!(manifest.author.as_deref(), Some("Test Author"));
        assert_eq!(manifest.homepage.as_deref(), Some("https://example.com"));
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
        assert_eq!(manifest.min_agent_version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.capabilities.len(), 3);
        assert_eq!(manifest.commands.len(), 2);
        assert_eq!(manifest.hooks.len(), 1);
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.mcp_servers.len(), 1);
        assert_eq!(manifest.settings.len(), 1);
    }

    #[test]
    fn test_parse_toml_commands() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let deploy = &manifest.commands[0];
        assert_eq!(deploy.name, "deploy");
        assert_eq!(deploy.description, "Deploy to staging");
        assert!(deploy.template.contains("{{args}}"));
        assert_eq!(deploy.aliases, vec!["d"]);
    }

    #[test]
    fn test_parse_toml_hooks() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let hook = &manifest.hooks[0];
        assert_eq!(hook.event, "pre_tool_use");
        assert_eq!(hook.command, "echo checking");
        assert_eq!(hook.tool_filter.as_deref(), Some("Bash"));
        assert_eq!(hook.timeout_ms, Some(3000));
    }

    #[test]
    fn test_parse_toml_tools() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let tool = &manifest.tools[0];
        assert_eq!(tool.name, "security-scan");
        assert_eq!(tool.command, "security-scanner --json");
        assert!(tool.input_schema.is_object());
    }

    #[test]
    fn test_parse_toml_mcp_servers() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let server = &manifest.mcp_servers[0];
        assert_eq!(server.name, "github");
        assert_eq!(server.command, "npx");
        assert_eq!(server.args, vec!["-y", "@modelcontextprotocol/server-github"]);
        assert_eq!(
            server.env.get("GITHUB_TOKEN").map(String::as_str),
            Some("placeholder")
        );
    }

    #[test]
    fn test_parse_toml_settings() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let setting = &manifest.settings[0];
        assert_eq!(setting.key, "deploy.environment");
        assert_eq!(
            setting.setting_type,
            SettingType::Choice(vec!["staging".into(), "production".into()])
        );
        assert_eq!(
            setting.default_value,
            Some(serde_json::Value::String("staging".into()))
        );
    }

    #[test]
    fn test_parse_json() {
        let manifest = PluginManifest::from_json(sample_json()).unwrap();
        assert_eq!(manifest.name, "json-plugin");
        assert_eq!(manifest.version, "2.1.0");
        assert_eq!(manifest.capabilities, vec![PluginCapability::Tools]);
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "formatter");
    }

    #[test]
    fn test_validate_success() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let toml = r#"
name = ""
version = "1.0.0"
description = "bad"
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("name must not be empty"));
    }

    #[test]
    fn test_validate_bad_version() {
        let toml = r#"
name = "bad-version"
version = "1.0"
description = "bad"
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("not valid semver"));
    }

    #[test]
    fn test_validate_non_numeric_version() {
        let toml = r#"
name = "bad-version"
version = "1.abc.0"
description = "bad"
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("minor"));
        assert!(err.to_string().contains("not a valid number"));
    }

    #[test]
    fn test_validate_duplicate_command_names() {
        let toml = r#"
name = "dup-cmds"
version = "1.0.0"
description = "duplicate"

[[commands]]
name = "deploy"
description = "first"
template = "one"

[[commands]]
name = "deploy"
description = "second"
template = "two"
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate command name"));
    }

    #[test]
    fn test_validate_duplicate_alias() {
        let toml = r#"
name = "dup-alias"
version = "1.0.0"
description = "duplicate alias"

[[commands]]
name = "deploy"
description = "first"
template = "one"
aliases = ["d"]

[[commands]]
name = "download"
description = "second"
template = "two"
aliases = ["d"]
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate command name/alias"));
    }

    #[test]
    fn test_defaults_for_optional_fields() {
        let toml = r#"
name = "minimal"
version = "0.1.0"
description = "minimal plugin"
"#;
        let manifest = PluginManifest::from_toml(toml).unwrap();
        assert!(manifest.author.is_none());
        assert!(manifest.homepage.is_none());
        assert!(manifest.license.is_none());
        assert!(manifest.min_agent_version.is_none());
        assert!(manifest.capabilities.is_empty());
        assert!(manifest.commands.is_empty());
        assert!(manifest.hooks.is_empty());
        assert!(manifest.tools.is_empty());
        assert!(manifest.mcp_servers.is_empty());
        assert!(manifest.settings.is_empty());
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_roundtrip_json() {
        let manifest = PluginManifest::from_toml(sample_toml()).unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let roundtripped = PluginManifest::from_json(&json).unwrap();
        assert_eq!(roundtripped.name, manifest.name);
        assert_eq!(roundtripped.version, manifest.version);
        assert_eq!(roundtripped.commands.len(), manifest.commands.len());
        assert_eq!(roundtripped.tools.len(), manifest.tools.len());
    }
}
