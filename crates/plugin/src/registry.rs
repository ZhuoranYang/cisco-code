//! Plugin registry: the central store of registered plugins.
//!
//! The registry tracks plugin state (enabled/disabled/error), and provides
//! aggregation methods to collect all commands, hooks, tools, and MCP servers
//! contributed by enabled plugins.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::discovery::{PluginLocation, PluginSource};
use crate::manifest::{PluginCommand, PluginHook, PluginMcpServer, PluginTool};

/// Runtime state of a registered plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    /// Plugin is active and its capabilities are loaded.
    Enabled,
    /// Plugin is installed but not active.
    Disabled,
    /// Plugin failed to load or validate.
    Error(String),
}

/// A single plugin entry in the registry.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: crate::manifest::PluginManifest,
    pub location: PathBuf,
    pub state: PluginState,
    pub source: PluginSource,
}

/// Central plugin registry.
#[derive(Debug)]
pub struct PluginRegistry {
    plugins: HashMap<String, PluginEntry>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a discovered plugin. Validates the manifest on register; if
    /// validation fails the plugin is still added but in the `Error` state.
    /// Returns an error only if a plugin with the same name is already present.
    pub fn register(&mut self, location: PluginLocation) -> anyhow::Result<()> {
        let name = location.manifest.name.clone();
        if self.plugins.contains_key(&name) {
            anyhow::bail!("plugin '{}' is already registered", name);
        }

        let state = match location.manifest.validate() {
            Ok(()) => PluginState::Enabled,
            Err(e) => PluginState::Error(e.to_string()),
        };

        self.plugins.insert(
            name,
            PluginEntry {
                manifest: location.manifest,
                location: location.path,
                state,
                source: location.source,
            },
        );
        Ok(())
    }

    /// Remove a plugin from the registry, returning its entry if it existed.
    pub fn unregister(&mut self, name: &str) -> Option<PluginEntry> {
        self.plugins.remove(name)
    }

    /// Enable a registered plugin.
    pub fn enable(&mut self, name: &str) -> anyhow::Result<()> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("plugin '{}' not found", name))?;
        entry.state = PluginState::Enabled;
        Ok(())
    }

    /// Disable a registered plugin.
    pub fn disable(&mut self, name: &str) -> anyhow::Result<()> {
        let entry = self
            .plugins
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("plugin '{}' not found", name))?;
        entry.state = PluginState::Disabled;
        Ok(())
    }

    /// Look up a plugin by name.
    pub fn get(&self, name: &str) -> Option<&PluginEntry> {
        self.plugins.get(name)
    }

    /// Return all plugins that are currently enabled.
    pub fn enabled_plugins(&self) -> Vec<&PluginEntry> {
        self.plugins
            .values()
            .filter(|e| e.state == PluginState::Enabled)
            .collect()
    }

    /// Collect all slash commands from enabled plugins, paired with their
    /// plugin name.
    pub fn all_commands(&self) -> Vec<(String, &PluginCommand)> {
        self.plugins
            .iter()
            .filter(|(_, e)| e.state == PluginState::Enabled)
            .flat_map(|(name, entry)| {
                entry
                    .manifest
                    .commands
                    .iter()
                    .map(move |cmd| (name.clone(), cmd))
            })
            .collect()
    }

    /// Collect all hooks from enabled plugins.
    pub fn all_hooks(&self) -> Vec<(String, &PluginHook)> {
        self.plugins
            .iter()
            .filter(|(_, e)| e.state == PluginState::Enabled)
            .flat_map(|(name, entry)| {
                entry
                    .manifest
                    .hooks
                    .iter()
                    .map(move |hook| (name.clone(), hook))
            })
            .collect()
    }

    /// Collect all tools from enabled plugins.
    pub fn all_tools(&self) -> Vec<(String, &PluginTool)> {
        self.plugins
            .iter()
            .filter(|(_, e)| e.state == PluginState::Enabled)
            .flat_map(|(name, entry)| {
                entry
                    .manifest
                    .tools
                    .iter()
                    .map(move |tool| (name.clone(), tool))
            })
            .collect()
    }

    /// Collect all MCP server configs from enabled plugins.
    pub fn all_mcp_servers(&self) -> Vec<(String, &PluginMcpServer)> {
        self.plugins
            .iter()
            .filter(|(_, e)| e.state == PluginState::Enabled)
            .flat_map(|(name, entry)| {
                entry
                    .manifest
                    .mcp_servers
                    .iter()
                    .map(move |srv| (name.clone(), srv))
            })
            .collect()
    }

    /// List all registered plugins (name and entry), regardless of state.
    /// Sorted by name for deterministic output.
    pub fn list(&self) -> Vec<(&str, &PluginEntry)> {
        let mut entries: Vec<_> = self.plugins.iter().map(|(k, v)| (k.as_str(), v)).collect();
        entries.sort_by_key(|(name, _)| *name);
        entries
    }

    /// Total number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::PluginLocation;
    use crate::manifest::PluginManifest;
    use std::path::PathBuf;

    fn make_location(name: &str) -> PluginLocation {
        let toml = format!(
            r#"
name = "{name}"
version = "1.0.0"
description = "Test plugin"

[[commands]]
name = "cmd-{name}"
description = "A command"
template = "do {{{{args}}}}"

[[hooks]]
event = "pre_tool_use"
command = "echo hook"

[[tools]]
name = "tool-{name}"
description = "A tool"
input_schema = {{ type = "object" }}
command = "run-tool"

[[mcp_servers]]
name = "mcp-{name}"
command = "npx"
"#
        );
        PluginLocation {
            path: PathBuf::from(format!("/plugins/{name}")),
            manifest: PluginManifest::from_toml(&toml).unwrap(),
            source: PluginSource::User,
        }
    }

    fn make_minimal_location(name: &str) -> PluginLocation {
        let toml = format!(
            r#"
name = "{name}"
version = "1.0.0"
description = "Minimal plugin"
"#
        );
        PluginLocation {
            path: PathBuf::from(format!("/plugins/{name}")),
            manifest: PluginManifest::from_toml(&toml).unwrap(),
            source: PluginSource::Local,
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();

        let entry = reg.get("alpha").unwrap();
        assert_eq!(entry.manifest.name, "alpha");
        assert_eq!(entry.state, PluginState::Enabled);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        let err = reg.register(make_location("alpha")).unwrap_err();
        assert!(err.to_string().contains("already registered"));
    }

    #[test]
    fn test_unregister() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        let removed = reg.unregister("alpha");
        assert!(removed.is_some());
        assert!(reg.get("alpha").is_none());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut reg = PluginRegistry::new();
        assert!(reg.unregister("nope").is_none());
    }

    #[test]
    fn test_enable_disable() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();

        reg.disable("alpha").unwrap();
        assert_eq!(reg.get("alpha").unwrap().state, PluginState::Disabled);
        assert!(reg.enabled_plugins().is_empty());

        reg.enable("alpha").unwrap();
        assert_eq!(reg.get("alpha").unwrap().state, PluginState::Enabled);
        assert_eq!(reg.enabled_plugins().len(), 1);
    }

    #[test]
    fn test_enable_nonexistent_fails() {
        let mut reg = PluginRegistry::new();
        assert!(reg.enable("nope").is_err());
    }

    #[test]
    fn test_disable_nonexistent_fails() {
        let mut reg = PluginRegistry::new();
        assert!(reg.disable("nope").is_err());
    }

    #[test]
    fn test_enabled_plugins_filters() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        reg.register(make_location("beta")).unwrap();
        reg.disable("beta").unwrap();

        let enabled = reg.enabled_plugins();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].manifest.name, "alpha");
    }

    #[test]
    fn test_all_commands() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        reg.register(make_location("beta")).unwrap();

        let cmds = reg.all_commands();
        assert_eq!(cmds.len(), 2);
        let names: Vec<&str> = cmds.iter().map(|(_, c)| c.name.as_str()).collect();
        assert!(names.contains(&"cmd-alpha"));
        assert!(names.contains(&"cmd-beta"));
    }

    #[test]
    fn test_all_commands_skips_disabled() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        reg.register(make_location("beta")).unwrap();
        reg.disable("alpha").unwrap();

        let cmds = reg.all_commands();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].1.name, "cmd-beta");
    }

    #[test]
    fn test_all_hooks() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        let hooks = reg.all_hooks();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].0, "alpha");
    }

    #[test]
    fn test_all_tools() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        reg.register(make_location("beta")).unwrap();
        let tools = reg.all_tools();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_all_mcp_servers() {
        let mut reg = PluginRegistry::new();
        reg.register(make_location("alpha")).unwrap();
        let servers = reg.all_mcp_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].1.name, "mcp-alpha");
    }

    #[test]
    fn test_list_sorted() {
        let mut reg = PluginRegistry::new();
        reg.register(make_minimal_location("charlie")).unwrap();
        reg.register(make_minimal_location("alpha")).unwrap();
        reg.register(make_minimal_location("bravo")).unwrap();

        let list = reg.list();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].0, "alpha");
        assert_eq!(list[1].0, "bravo");
        assert_eq!(list[2].0, "charlie");
    }

    #[test]
    fn test_empty_registry() {
        let reg = PluginRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.all_commands().is_empty());
        assert!(reg.all_hooks().is_empty());
        assert!(reg.all_tools().is_empty());
        assert!(reg.all_mcp_servers().is_empty());
        assert!(reg.list().is_empty());
    }
}
