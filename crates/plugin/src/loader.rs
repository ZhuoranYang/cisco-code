//! Plugin loader: orchestrates discovery, validation, and registration.
//!
//! The loader is the main entry point for the plugin system. It discovers
//! plugins from search paths, registers them in the registry, applies a
//! disabled-list, and exposes a summary of the resulting state.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::discovery::{discover_plugins, PluginLocation, PluginSource};
use crate::manifest::PluginManifest;
use crate::registry::{PluginRegistry, PluginState};

/// Result of a plugin loading operation.
#[derive(Debug, Clone)]
pub struct LoadResult {
    /// Successfully loaded plugin names.
    pub loaded: Vec<String>,
    /// Plugins that failed to load: (name, error message).
    pub failed: Vec<(String, String)>,
    /// Plugins that were skipped because they are in the disabled list.
    pub skipped: Vec<String>,
}

/// Summary statistics of the current plugin state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSummary {
    pub total: usize,
    pub enabled: usize,
    pub disabled: usize,
    pub errored: usize,
    pub command_count: usize,
    pub hook_count: usize,
    pub tool_count: usize,
}

/// Orchestrates plugin discovery, loading, and lifecycle management.
pub struct PluginLoader {
    /// The underlying registry holding all plugin entries.
    pub registry: PluginRegistry,
    /// Plugin names that should be skipped during loading.
    disabled_plugins: HashSet<String>,
}

impl PluginLoader {
    /// Create a new loader with an empty registry.
    pub fn new() -> Self {
        Self {
            registry: PluginRegistry::new(),
            disabled_plugins: HashSet::new(),
        }
    }

    /// Discover and load plugins from the given search paths.
    ///
    /// Plugins on the disabled list are skipped. Duplicate names or validation
    /// failures are recorded in the `LoadResult` rather than causing a panic.
    pub fn load_from_paths(&mut self, paths: &[PathBuf]) -> LoadResult {
        let discovered = discover_plugins(paths);
        let mut result = LoadResult {
            loaded: Vec::new(),
            failed: Vec::new(),
            skipped: Vec::new(),
        };

        for location in discovered {
            let name = location.manifest.name.clone();

            if self.disabled_plugins.contains(&name) {
                result.skipped.push(name);
                continue;
            }

            match self.registry.register(location) {
                Ok(()) => {
                    // Check if it ended up in error state (validation failure).
                    if let Some(entry) = self.registry.get(&name) {
                        if matches!(entry.state, PluginState::Error(_)) {
                            if let PluginState::Error(ref msg) = entry.state {
                                result.failed.push((name.clone(), msg.clone()));
                            }
                            // Still count as registered but note the failure.
                            continue;
                        }
                    }
                    result.loaded.push(name);
                }
                Err(e) => {
                    result.failed.push((name, e.to_string()));
                }
            }
        }

        result
    }

    /// Load a single plugin from a manifest file path.
    ///
    /// The file is parsed (TOML or JSON based on extension), validated, and
    /// registered.
    pub fn load_from_manifest(&mut self, path: &Path) -> anyhow::Result<PluginLocation> {
        let content = std::fs::read_to_string(path)?;
        let manifest = if path.extension().is_some_and(|ext| ext == "json") {
            PluginManifest::from_json(&content)?
        } else {
            PluginManifest::from_toml(&content)?
        };

        manifest.validate()?;

        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        let location = PluginLocation {
            path: dir,
            manifest,
            source: PluginSource::Local,
        };

        self.registry.register(location.clone())?;
        Ok(location)
    }

    /// Mark the given plugin names as disabled.
    ///
    /// Already-registered plugins with these names are moved to the `Disabled`
    /// state. Future `load_from_paths` calls will skip plugins with these names.
    pub fn apply_disabled_list(&mut self, names: &[String]) {
        for name in names {
            self.disabled_plugins.insert(name.clone());
            // Best-effort: disable if already registered.
            let _ = self.registry.disable(name);
        }
    }

    /// Compute a summary of the current plugin state.
    pub fn summary(&self) -> PluginSummary {
        let all = self.registry.list();
        let enabled = all
            .iter()
            .filter(|(_, e)| e.state == PluginState::Enabled)
            .count();
        let disabled = all
            .iter()
            .filter(|(_, e)| e.state == PluginState::Disabled)
            .count();
        let errored = all
            .iter()
            .filter(|(_, e)| matches!(e.state, PluginState::Error(_)))
            .count();

        PluginSummary {
            total: all.len(),
            enabled,
            disabled,
            errored,
            command_count: self.registry.all_commands().len(),
            hook_count: self.registry.all_hooks().len(),
            tool_count: self.registry.all_tools().len(),
        }
    }
}

impl Default for PluginLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Helper: create a plugin directory with a TOML manifest.
    fn create_plugin(parent: &Path, name: &str) {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"
name = "{name}"
version = "1.0.0"
description = "Test plugin {name}"

[[commands]]
name = "cmd-{name}"
description = "A command"
template = "do it"

[[hooks]]
event = "pre_tool_use"
command = "echo"

[[tools]]
name = "tool-{name}"
description = "A tool"
input_schema = {{ type = "object" }}
command = "run"
"#
        );
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
    }

    #[test]
    fn test_load_from_paths() {
        let tmp = TempDir::new().unwrap();
        create_plugin(tmp.path(), "alpha");
        create_plugin(tmp.path(), "beta");

        let mut loader = PluginLoader::new();
        let result = loader.load_from_paths(&[tmp.path().to_path_buf()]);

        assert_eq!(result.loaded.len(), 2);
        assert!(result.failed.is_empty());
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn test_load_with_disabled_list() {
        let tmp = TempDir::new().unwrap();
        create_plugin(tmp.path(), "alpha");
        create_plugin(tmp.path(), "beta");

        let mut loader = PluginLoader::new();
        loader.apply_disabled_list(&["alpha".into()]);
        let result = loader.load_from_paths(&[tmp.path().to_path_buf()]);

        assert_eq!(result.loaded.len(), 1);
        assert!(result.loaded.contains(&"beta".to_string()));
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0], "alpha");
    }

    #[test]
    fn test_load_from_manifest_toml() {
        let tmp = TempDir::new().unwrap();
        let toml = r#"
name = "single"
version = "1.0.0"
description = "A single plugin"
"#;
        let path = tmp.path().join("plugin.toml");
        std::fs::write(&path, toml).unwrap();

        let mut loader = PluginLoader::new();
        let location = loader.load_from_manifest(&path).unwrap();
        assert_eq!(location.manifest.name, "single");
        assert!(loader.registry.get("single").is_some());
    }

    #[test]
    fn test_load_from_manifest_json() {
        let tmp = TempDir::new().unwrap();
        let json = r#"{
  "name": "json-single",
  "version": "1.0.0",
  "description": "A JSON plugin"
}"#;
        let path = tmp.path().join("plugin.json");
        std::fs::write(&path, json).unwrap();

        let mut loader = PluginLoader::new();
        let location = loader.load_from_manifest(&path).unwrap();
        assert_eq!(location.manifest.name, "json-single");
    }

    #[test]
    fn test_load_from_manifest_invalid_fails() {
        let tmp = TempDir::new().unwrap();
        // Missing version field = invalid semver.
        let toml = r#"
name = "bad"
version = "nope"
description = "bad version"
"#;
        let path = tmp.path().join("plugin.toml");
        std::fs::write(&path, toml).unwrap();

        let mut loader = PluginLoader::new();
        let err = loader.load_from_manifest(&path).unwrap_err();
        assert!(err.to_string().contains("not valid semver"));
    }

    #[test]
    fn test_summary_counts() {
        let tmp = TempDir::new().unwrap();
        create_plugin(tmp.path(), "alpha");
        create_plugin(tmp.path(), "beta");

        let mut loader = PluginLoader::new();
        loader.load_from_paths(&[tmp.path().to_path_buf()]);

        let summary = loader.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.enabled, 2);
        assert_eq!(summary.disabled, 0);
        assert_eq!(summary.errored, 0);
        assert_eq!(summary.command_count, 2);
        assert_eq!(summary.hook_count, 2);
        assert_eq!(summary.tool_count, 2);
    }

    #[test]
    fn test_summary_after_disable() {
        let tmp = TempDir::new().unwrap();
        create_plugin(tmp.path(), "alpha");
        create_plugin(tmp.path(), "beta");

        let mut loader = PluginLoader::new();
        loader.load_from_paths(&[tmp.path().to_path_buf()]);
        loader.apply_disabled_list(&["alpha".into()]);

        let summary = loader.summary();
        assert_eq!(summary.total, 2);
        assert_eq!(summary.enabled, 1);
        assert_eq!(summary.disabled, 1);
        assert_eq!(summary.command_count, 1);
        assert_eq!(summary.hook_count, 1);
        assert_eq!(summary.tool_count, 1);
    }

    #[test]
    fn test_empty_loader_summary() {
        let loader = PluginLoader::new();
        let summary = loader.summary();
        assert_eq!(
            summary,
            PluginSummary {
                total: 0,
                enabled: 0,
                disabled: 0,
                errored: 0,
                command_count: 0,
                hook_count: 0,
                tool_count: 0,
            }
        );
    }
}
