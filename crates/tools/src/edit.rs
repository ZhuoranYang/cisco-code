//! Edit tool — string replacement in files.
//!
//! Pattern from Claude Code's FileEditTool: find unique old_string in file,
//! replace with new_string. Supports replace_all for multiple occurrences.
//! Includes curly quote normalization for robustness.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Performs exact string replacements in files. The old_string must be unique in the file unless replace_all is true. Preserves exact indentation."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to modify"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default false)",
                    "default": false
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path'"))?;
        let old_string = input["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'old_string'"))?;
        let new_string = input["new_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'new_string'"))?;
        let replace_all = input["replace_all"].as_bool().unwrap_or(false);

        if old_string == new_string {
            return Ok(ToolResult::error(
                "old_string and new_string must be different",
            ));
        }

        let path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            Path::new(&ctx.cwd)
                .join(file_path)
                .to_string_lossy()
                .to_string()
        };

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read {path}: {e}"))),
        };

        // Try exact match first, then normalized quotes
        let actual_old = find_actual_string(&content, old_string);

        let search_str = match &actual_old {
            Some(s) => s.as_str(),
            None => {
                return Ok(ToolResult::error(format!(
                    "old_string not found in {path}. Make sure it matches exactly including whitespace."
                )));
            }
        };

        // Count occurrences
        let count = content.matches(search_str).count();

        if count > 1 && !replace_all {
            return Ok(ToolResult::error(format!(
                "Found {count} matches for old_string in {path}. Provide more context to make it unique, or set replace_all=true."
            )));
        }

        // Perform replacement
        let new_content = if replace_all {
            content.replace(search_str, new_string)
        } else {
            content.replacen(search_str, new_string, 1)
        };

        match tokio::fs::write(&path, &new_content).await {
            Ok(()) => {
                let msg = if replace_all && count > 1 {
                    format!("Replaced {count} occurrences in {path}")
                } else {
                    format!("Successfully edited {path}")
                };
                Ok(ToolResult::success(msg))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to write {path}: {e}"))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }
}

/// Find the actual string in file content, accounting for curly quote normalization.
/// Returns the actual substring from the file (preserving its quote style).
fn find_actual_string(content: &str, search: &str) -> Option<String> {
    // Exact match
    if content.contains(search) {
        return Some(search.to_string());
    }

    // Normalized match (curly quotes → straight quotes)
    let norm_search = normalize_quotes(search);
    let norm_content = normalize_quotes(content);

    if let Some(idx) = norm_content.find(&norm_search) {
        // Return the actual substring from the original content
        Some(content[idx..idx + search.len()].to_string())
    } else {
        None
    }
}

/// Normalize curly/smart quotes to straight quotes.
fn normalize_quotes(s: &str) -> String {
    s.replace('\u{2018}', "'") // '
        .replace('\u{2019}', "'") // '
        .replace('\u{201C}', "\"") // "
        .replace('\u{201D}', "\"") // "
}
