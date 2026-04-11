//! Plugin discovery: scanning directories for plugin manifests.
//!
//! Plugins live in directories that contain a `plugin.toml` or `plugin.json`.
//! Discovery walks well-known search paths and returns every valid manifest it
//! finds, annotated with its source (project-local, user-global, etc.).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::PluginManifest;

/// A discovered plugin: its location on disk, parsed manifest, and source.
#[derive(Debug, Clone)]
pub struct PluginLocation {
    /// Directory containing the plugin manifest.
    pub path: PathBuf,
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Where the plugin was found.
    pub source: PluginSource,
}

/// Where a plugin was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSource {
    /// Project-local plugin (`.cisco-code/plugins/` in the project).
    Local,
    /// Project-level (explicit project config).
    Project,
    /// User-global plugin (`~/.cisco-code/plugins/`).
    User,
    /// Shipped with the agent binary.
    Builtin,
}

/// Discover plugins by scanning the given search paths.
///
/// Each path is expected to be a directory whose immediate children are plugin
/// directories. A plugin directory must contain either `plugin.toml` or
/// `plugin.json`. Directories without a manifest are silently skipped.
pub fn discover_plugins(search_paths: &[PathBuf]) -> Vec<PluginLocation> {
    let mut results = Vec::new();

    for search_path in search_paths {
        if !search_path.is_dir() {
            tracing::debug!("plugin search path does not exist: {}", search_path.display());
            continue;
        }

        let source = source_for_path(search_path);

        let entries = match std::fs::read_dir(search_path) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(
                    "failed to read plugin search path {}: {e}",
                    search_path.display()
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }

            if let Some(location) = try_load_plugin_dir(&dir, &source) {
                results.push(location);
            }
        }
    }

    results
}

/// Return the standard plugin search paths for a given project directory.
///
/// Order (highest to lowest priority):
/// 1. `<project_dir>/.cisco-code/plugins/` (project-local)
/// 2. `~/.cisco-code/plugins/` (user-global)
pub fn plugin_search_paths(project_dir: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Project-local plugins.
    let project_plugins = PathBuf::from(project_dir).join(".cisco-code").join("plugins");
    paths.push(project_plugins);

    // User-global plugins.
    if let Some(home) = home_dir() {
        let user_plugins = home.join(".cisco-code").join("plugins");
        paths.push(user_plugins);
    }

    paths
}

/// Try to load a plugin from a directory. Returns `None` if the directory does
/// not contain a valid manifest.
fn try_load_plugin_dir(dir: &Path, source: &PluginSource) -> Option<PluginLocation> {
    // Prefer TOML over JSON.
    let toml_path = dir.join("plugin.toml");
    let json_path = dir.join("plugin.json");

    let manifest = if toml_path.is_file() {
        let content = std::fs::read_to_string(&toml_path).ok()?;
        PluginManifest::from_toml(&content).ok()?
    } else if json_path.is_file() {
        let content = std::fs::read_to_string(&json_path).ok()?;
        PluginManifest::from_json(&content).ok()?
    } else {
        return None;
    };

    Some(PluginLocation {
        path: dir.to_path_buf(),
        manifest,
        source: source.clone(),
    })
}

/// Heuristic: determine the plugin source from the search path.
fn source_for_path(path: &Path) -> PluginSource {
    let path_str = path.to_string_lossy();
    if path_str.contains(".cisco-code/plugins") || path_str.contains(".cisco-code\\plugins") {
        // Distinguish project-local vs user-global by checking if the path
        // starts with the user's home directory directly.
        if let Some(home) = home_dir() {
            let user_plugins = home.join(".cisco-code").join("plugins");
            if path == user_plugins {
                return PluginSource::User;
            }
        }
        PluginSource::Local
    } else {
        PluginSource::Project
    }
}

/// Cross-platform home directory.
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a plugin directory with a TOML manifest inside `parent`.
    fn create_toml_plugin(parent: &Path, name: &str) -> PathBuf {
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
template = "do something {{{{args}}}}"
"#
        );
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
        dir
    }

    /// Create a plugin directory with a JSON manifest inside `parent`.
    fn create_json_plugin(parent: &Path, name: &str) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let json = format!(
            r#"{{
  "name": "{name}",
  "version": "1.0.0",
  "description": "JSON plugin {name}",
  "commands": [],
  "hooks": [],
  "tools": [],
  "mcp_servers": [],
  "settings": []
}}"#
        );
        std::fs::write(dir.join("plugin.json"), json).unwrap();
        dir
    }

    #[test]
    fn test_discover_toml_plugin() {
        let tmp = TempDir::new().unwrap();
        create_toml_plugin(tmp.path(), "alpha");

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.name, "alpha");
    }

    #[test]
    fn test_discover_json_plugin() {
        let tmp = TempDir::new().unwrap();
        create_json_plugin(tmp.path(), "beta");

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.name, "beta");
    }

    #[test]
    fn test_discover_multiple_plugins() {
        let tmp = TempDir::new().unwrap();
        create_toml_plugin(tmp.path(), "one");
        create_toml_plugin(tmp.path(), "two");
        create_json_plugin(tmp.path(), "three");

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert_eq!(plugins.len(), 3);
        let names: Vec<&str> = plugins.iter().map(|p| p.manifest.name.as_str()).collect();
        assert!(names.contains(&"one"));
        assert!(names.contains(&"two"));
        assert!(names.contains(&"three"));
    }

    #[test]
    fn test_discover_skips_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        // Directory without any manifest file.
        std::fs::create_dir_all(tmp.path().join("no-manifest")).unwrap();
        create_toml_plugin(tmp.path(), "has-manifest");

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].manifest.name, "has-manifest");
    }

    #[test]
    fn test_discover_skips_invalid_toml() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plugin.toml"), "this is not valid toml [[[").unwrap();

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_discover_nonexistent_path() {
        let plugins = discover_plugins(&[PathBuf::from("/nonexistent/path/12345")]);
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_discover_multiple_search_paths() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        create_toml_plugin(tmp1.path(), "from-first");
        create_toml_plugin(tmp2.path(), "from-second");

        let plugins = discover_plugins(&[tmp1.path().to_path_buf(), tmp2.path().to_path_buf()]);
        assert_eq!(plugins.len(), 2);
    }

    #[test]
    fn test_plugin_search_paths_structure() {
        let paths = plugin_search_paths("/some/project");
        assert!(paths.len() >= 1);
        // First path should be project-local.
        assert!(paths[0].to_string_lossy().contains(".cisco-code/plugins"));
        assert!(paths[0].to_string_lossy().starts_with("/some/project"));
    }

    #[test]
    fn test_toml_preferred_over_json() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("dual");
        std::fs::create_dir_all(&dir).unwrap();

        // Write both TOML and JSON with different descriptions.
        std::fs::write(
            dir.join("plugin.toml"),
            r#"
name = "dual"
version = "1.0.0"
description = "from toml"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("plugin.json"),
            r#"{"name": "dual", "version": "1.0.0", "description": "from json"}"#,
        )
        .unwrap();

        let plugins = discover_plugins(&[tmp.path().to_path_buf()]);
        assert_eq!(plugins.len(), 1);
        // TOML takes precedence.
        assert_eq!(plugins[0].manifest.description, "from toml");
    }
}
