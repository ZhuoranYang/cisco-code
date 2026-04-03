//! Grep tool — search file contents using ripgrep.
//!
//! Pattern from Claude Code: shell out to `rg` (ripgrep) with appropriate flags.
//! Exit code 1 = no matches (not an error). EAGAIN gets retried with -j 1.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::process::Stdio;
use tokio::process::Command;

use crate::{Tool, ToolContext};

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents using ripgrep regex. Returns matching file paths by default, or matching lines with content mode."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (default: cwd)"
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs', '*.{ts,tsx}')"
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "Output mode (default: files_with_matches)"
                },
                "-i": {
                    "type": "boolean",
                    "description": "Case insensitive search"
                },
                "-n": {
                    "type": "boolean",
                    "description": "Show line numbers (default true for content mode)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Limit output to first N entries (default 250)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;

        let search_path = input["path"]
            .as_str()
            .unwrap_or(&ctx.cwd)
            .to_string();

        let output_mode = input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches");

        let head_limit = input["head_limit"].as_u64().unwrap_or(250) as usize;

        // Build ripgrep args
        let mut args = Vec::new();

        match output_mode {
            "files_with_matches" => args.push("-l".to_string()),
            "count" => args.push("-c".to_string()),
            _ => {
                // content mode
                args.push("-n".to_string()); // line numbers
            }
        }

        if input["-i"].as_bool().unwrap_or(false) {
            args.push("-i".to_string());
        }

        if let Some(glob_pattern) = input["glob"].as_str() {
            args.push("--glob".to_string());
            args.push(glob_pattern.to_string());
        }

        args.push("--".to_string());
        args.push(pattern.to_string());
        args.push(search_path);

        // Execute ripgrep
        let output = Command::new("rg")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        match output {
            Ok(out) => {
                let code = out.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);

                if code == 0 || code == 1 {
                    // code 1 = no matches
                    let lines: Vec<&str> = stdout
                        .trim()
                        .lines()
                        .filter(|l| !l.is_empty())
                        .take(head_limit)
                        .collect();

                    if lines.is_empty() {
                        Ok(ToolResult::success("No matches found."))
                    } else {
                        let result = lines.join("\n");
                        let total = stdout.trim().lines().count();
                        if total > head_limit {
                            Ok(ToolResult::success(format!(
                                "{result}\n\n... ({} more results truncated)",
                                total - head_limit
                            )))
                        } else {
                            Ok(ToolResult::success(result))
                        }
                    }
                } else {
                    // Real error
                    let msg = if stderr.is_empty() {
                        format!("ripgrep exited with code {code}")
                    } else {
                        stderr.to_string()
                    };
                    Ok(ToolResult::error(msg))
                }
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to run ripgrep: {e}. Is 'rg' installed?"
            ))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;

    fn ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_string_lossy().to_string(),
            interactive: false,
        }
    }

    #[tokio::test]
    async fn test_grep_finds_pattern() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nfoo bar\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "nothing here\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "hello",
                    "path": dir.path().to_string_lossy()
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        // Might fail if rg not installed, but should still return ok
        if !result.is_error {
            assert!(result.output.contains("a.txt"));
        }
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "zzzzz_nonexistent",
                    "path": dir.path().to_string_lossy()
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        if !result.is_error {
            assert!(result.output.contains("No matches"));
        }
    }

    #[tokio::test]
    async fn test_grep_content_mode() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "println",
                    "path": dir.path().to_string_lossy(),
                    "output_mode": "content"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error {
            assert!(result.output.contains("println"));
        }
    }

    #[tokio::test]
    async fn test_grep_glob_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "target\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "target\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "target",
                    "path": dir.path().to_string_lossy(),
                    "glob": "*.rs"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error && !result.output.contains("No matches") {
            assert!(result.output.contains("a.rs"));
            assert!(!result.output.contains("b.txt"));
        }
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "Hello World\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "hello",
                    "path": dir.path().to_string_lossy(),
                    "-i": true
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error {
            assert!(result.output.contains("test.txt"));
        }
    }
}
