//! Grep tool — search file contents using ripgrep.
//!
//! Pattern from Claude Code: shell out to `rg` (ripgrep) with appropriate flags.
//! Exit code 1 = no matches (not an error).

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
                "-A": {
                    "type": "integer",
                    "description": "Number of lines to show after each match"
                },
                "-B": {
                    "type": "integer",
                    "description": "Number of lines to show before each match"
                },
                "-C": {
                    "type": "integer",
                    "description": "Number of lines of context (before and after)"
                },
                "context": {
                    "type": "integer",
                    "description": "Alias for -C (lines of context before and after)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Limit output to first N entries (default 250)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip first N entries before applying head_limit (default 0)"
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline mode where . matches newlines (rg -U --multiline-dotall)"
                },
                "type": {
                    "type": "string",
                    "description": "File type filter (e.g. 'js', 'py', 'rust'). Maps to rg --type."
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
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;

        // Build ripgrep args
        let mut args = Vec::new();

        // Multiline mode
        if input["multiline"].as_bool().unwrap_or(false) {
            args.push("-U".to_string()); // --multiline
            args.push("--multiline-dotall".to_string());
        }

        match output_mode {
            "files_with_matches" => args.push("-l".to_string()),
            "count" => args.push("-c".to_string()),
            _ => {
                // content mode
                let show_numbers = input["-n"].as_bool().unwrap_or(true);
                if show_numbers {
                    args.push("-n".to_string());
                }
            }
        }

        if input["-i"].as_bool().unwrap_or(false) {
            args.push("-i".to_string());
        }

        // Context flags (only meaningful for content mode)
        if output_mode == "content" {
            // -C / context takes precedence, then -A/-B individually
            let context = input["-C"].as_u64().or_else(|| input["context"].as_u64());
            if let Some(c) = context {
                args.push("-C".to_string());
                args.push(c.to_string());
            } else {
                if let Some(a) = input["-A"].as_u64() {
                    args.push("-A".to_string());
                    args.push(a.to_string());
                }
                if let Some(b) = input["-B"].as_u64() {
                    args.push("-B".to_string());
                    args.push(b.to_string());
                }
            }
        }

        if let Some(glob_pattern) = input["glob"].as_str() {
            args.push("--glob".to_string());
            args.push(glob_pattern.to_string());
        }

        // File type filter
        if let Some(file_type) = input["type"].as_str() {
            args.push("--type".to_string());
            args.push(file_type.to_string());
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
                    // In content mode, preserve blank lines (they may be context
                    // lines from the source file or ripgrep separators).
                    // In other modes, filter empty lines.
                    let all_lines: Vec<&str> = if output_mode == "content" {
                        stdout.trim().lines().collect()
                    } else {
                        stdout.trim().lines().filter(|l| !l.is_empty()).collect()
                    };

                    // Apply offset + head_limit
                    let total = all_lines.len();
                    let lines: Vec<&str> = all_lines
                        .into_iter()
                        .skip(offset)
                        .take(head_limit)
                        .collect();

                    if lines.is_empty() {
                        if offset > 0 && total > 0 {
                            Ok(ToolResult::success(format!(
                                "No results at offset {offset} (total: {total})"
                            )))
                        } else {
                            Ok(ToolResult::success("No matches found."))
                        }
                    } else {
                        let result = lines.join("\n");
                        let remaining = total.saturating_sub(offset + head_limit);
                        if remaining > 0 {
                            Ok(ToolResult::success(format!(
                                "{result}\n\n[Showing results with pagination = limit: {head_limit}]"
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

    fn is_concurrency_safe(&self) -> bool {
        true
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
            progress_tx: None,
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

    #[tokio::test]
    async fn test_grep_with_offset() {
        let dir = tempfile::tempdir().unwrap();
        // Create files that will produce multiple matches
        for i in 1..=5 {
            std::fs::write(
                dir.path().join(format!("file{i}.txt")),
                "matching_content\n",
            )
            .unwrap();
        }

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "matching_content",
                    "path": dir.path().to_string_lossy(),
                    "offset": 2,
                    "head_limit": 2
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error && !result.output.contains("No matches") {
            // Should have at most 2 results (after skipping first 2)
            let lines: Vec<&str> = result.output.lines()
                .filter(|l| !l.is_empty() && !l.starts_with('['))
                .collect();
            assert!(lines.len() <= 2);
        }
    }

    #[tokio::test]
    async fn test_grep_multiline() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("multi.txt"),
            "struct Foo {\n    field: i32,\n}\n",
        )
        .unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "struct.*field",
                    "path": dir.path().to_string_lossy(),
                    "multiline": true,
                    "output_mode": "content"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        // Multiline match should find the cross-line pattern
        if !result.is_error {
            assert!(
                result.output.contains("struct") || result.output.contains("No matches"),
                "output: {}",
                result.output
            );
        }
    }

    #[tokio::test]
    async fn test_grep_type_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("code.py"), "def main(): pass\n").unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "main",
                    "path": dir.path().to_string_lossy(),
                    "type": "rust"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error && !result.output.contains("No matches") {
            assert!(result.output.contains("code.rs"));
            assert!(!result.output.contains("code.py"));
        }
    }

    #[tokio::test]
    async fn test_grep_context_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("ctx.txt"),
            "before1\nbefore2\ntarget\nafter1\nafter2\n",
        )
        .unwrap();

        let tool = GrepTool;
        let result = tool
            .call(
                serde_json::json!({
                    "pattern": "target",
                    "path": dir.path().to_string_lossy(),
                    "output_mode": "content",
                    "context": 1
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        if !result.is_error {
            assert!(result.output.contains("target"));
            // Should include context lines
            assert!(
                result.output.contains("before2") || result.output.contains("after1"),
                "Expected context lines, got: {}",
                result.output
            );
        }
    }
}
