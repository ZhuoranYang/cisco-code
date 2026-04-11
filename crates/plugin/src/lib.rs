//! cisco-code-plugin: Plugin system for extending cisco-code with custom
//! commands, hooks, tools, and MCP server configurations.
//!
//! Plugins are discovered from well-known directories, loaded from TOML or JSON
//! manifests, and registered into a central registry that the runtime queries.

pub mod manifest;
pub mod discovery;
pub mod registry;
pub mod loader;

pub use manifest::*;
pub use discovery::*;
pub use registry::*;
pub use loader::*;
