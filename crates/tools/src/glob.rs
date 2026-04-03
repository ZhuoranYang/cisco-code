//! Glob tool — find files by pattern.
//!
//! Pattern from Claude Code: uses ripgrep --files --glob for fast matching,
//! sorted by modification time.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::process::Stdio;
use tokio::process::Command;

use crate::{Tool, ToolContext};

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Fast file pattern matching. Supports glob patterns like '**/*.rs' or 'src/**/*.ts'. Returns matching file paths sorted by modification time."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files (e.g. '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: cwd)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;

        let search_dir = input["path"]
            .as_str()
            .unwrap_or(&ctx.cwd)
            .to_string();

        // Use ripgrep --files --glob for fast matching
        let output = Command::new("rg")
            .args([
                "--files",
                "--glob",
                pattern,
                "--sort=modified",
                "--hidden",
                &search_dir,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) => {
                let code = out.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&out.stdout);

                if code == 0 || code == 1 {
                    let files: Vec<&str> = stdout
                        .trim()
                        .lines()
                        .filter(|l| !l.is_empty())
                        .take(500) // limit
                        .collect();

                    if files.is_empty() {
                        Ok(ToolResult::success("No files matched."))
                    } else {
                        let count = files.len();
                        Ok(ToolResult::success(format!(
                            "{}\n\n{count} file(s) matched",
                            files.join("\n")
                        )))
                    }
                } else {
                    // Fallback to native glob if rg not available
                    glob_native(pattern, &search_dir).await
                }
            }
            Err(_) => {
                // rg not available, use native glob
                glob_native(pattern, &search_dir).await
            }
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

/// Fallback glob implementation using the `glob` crate.
async fn glob_native(pattern: &str, base_dir: &str) -> Result<ToolResult> {
    let full_pattern = if std::path::Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        format!("{base_dir}/{pattern}")
    };

    let paths: Vec<String> = glob::glob(&full_pattern)?
        .filter_map(|entry| entry.ok())
        .take(500)
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    if paths.is_empty() {
        Ok(ToolResult::success("No files matched."))
    } else {
        let count = paths.len();
        Ok(ToolResult::success(format!(
            "{}\n\n{count} file(s) matched",
            paths.join("\n")
        )))
    }
}
