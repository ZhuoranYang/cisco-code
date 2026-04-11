//! ApplyPatch tool — apply unified diff patches to files.
//!
//! Inspired by Codex's grammar-validated approach. Models naturally produce
//! unified diffs for multi-file changes; forcing them through Edit one replacement
//! at a time wastes tokens and is error-prone.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct ApplyPatchTool;

#[async_trait::async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "ApplyPatch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to one or more files. Parses standard unified diff format with ---/+++ headers and @@ hunk markers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "The unified diff patch content"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for resolving relative paths (default: session cwd)"
                }
            },
            "required": ["patch"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let patch_text = input["patch"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'patch'"))?;

        let cwd = input["cwd"]
            .as_str()
            .unwrap_or(&ctx.cwd)
            .to_string();

        let files = match parse_patch(patch_text) {
            Ok(f) => f,
            Err(e) => return Ok(ToolResult::error(format!("Failed to parse patch: {e}"))),
        };

        if files.is_empty() {
            return Ok(ToolResult::error("No file changes found in patch"));
        }

        let mut results = Vec::new();
        let mut success_count = 0;
        let mut fail_count = 0;

        for file_patch in &files {
            let path = resolve_path(&file_patch.path, &cwd);

            match apply_file_patch(&path, &file_patch.hunks).await {
                Ok(applied) => {
                    results.push(format!("  {} — {} hunk(s) applied", file_patch.path, applied));
                    success_count += 1;
                }
                Err(e) => {
                    results.push(format!("  {} — FAILED: {}", file_patch.path, e));
                    fail_count += 1;
                }
            }
        }

        let summary = if fail_count == 0 {
            format!("Patch applied successfully ({} file(s)):\n{}", success_count, results.join("\n"))
        } else {
            format!(
                "Patch partially applied ({} succeeded, {} failed):\n{}",
                success_count, fail_count, results.join("\n")
            )
        };

        if fail_count > 0 {
            Ok(ToolResult::error(summary))
        } else {
            Ok(ToolResult::success(summary))
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }
}

/// A parsed file-level patch (one ---/+++ pair).
#[derive(Debug)]
struct FilePatch {
    path: String,
    hunks: Vec<Hunk>,
}

/// A single hunk from a unified diff.
#[derive(Debug)]
#[allow(dead_code)] // fields parsed for correctness validation; used in tests
struct Hunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

/// Parse unified diff text into structured file patches.
fn parse_patch(text: &str) -> Result<Vec<FilePatch>> {
    let mut files = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Find --- / +++ header pair
        if lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ") {
            let old_path = parse_file_header(lines[i], "--- ");
            let new_path = parse_file_header(lines[i + 1], "+++ ");
            i += 2;

            // Use new_path (the target), falling back to old_path for deletions
            let path = if new_path == "/dev/null" {
                old_path
            } else {
                new_path
            };

            // Parse hunks for this file
            let mut hunks = Vec::new();
            while i < lines.len() && lines[i].starts_with("@@ ") {
                let (hunk, next_i) = parse_hunk(&lines, i)?;
                hunks.push(hunk);
                i = next_i;
            }

            if !hunks.is_empty() {
                files.push(FilePatch { path, hunks });
            }
        } else {
            i += 1;
        }
    }

    Ok(files)
}

/// Extract file path from a ---/+++ header line.
fn parse_file_header(line: &str, prefix: &str) -> String {
    let rest = &line[prefix.len()..];
    // Handle "a/path" or "b/path" prefixes from git diff
    let path = if rest.starts_with("a/") || rest.starts_with("b/") {
        &rest[2..]
    } else {
        rest
    };
    // Strip trailing tab + timestamp (some diff formats)
    path.split('\t').next().unwrap_or(path).to_string()
}

/// Parse a single hunk starting at the @@ line.
fn parse_hunk(lines: &[&str], start: usize) -> Result<(Hunk, usize)> {
    let header = lines[start];
    let (old_start, old_count, new_start, new_count) = parse_hunk_header(header)?;

    let mut hunk_lines = Vec::new();
    let mut i = start + 1;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("@@ ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            break;
        }
        if line.starts_with('+') {
            hunk_lines.push(HunkLine::Add(line[1..].to_string()));
        } else if line.starts_with('-') {
            hunk_lines.push(HunkLine::Remove(line[1..].to_string()));
        } else if line.starts_with(' ') || line.is_empty() {
            let content = if line.is_empty() { "" } else { &line[1..] };
            hunk_lines.push(HunkLine::Context(content.to_string()));
        } else if line == "\\ No newline at end of file" {
            // Skip this directive
        } else {
            // Treat unknown lines as context
            hunk_lines.push(HunkLine::Context(line.to_string()));
        }
        i += 1;
    }

    Ok((
        Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: hunk_lines,
        },
        i,
    ))
}

/// Parse @@ -start,count +start,count @@ header.
fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize, usize)> {
    // Format: @@ -old_start[,old_count] +new_start[,new_count] @@
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() < 4 || parts[0] != "@@" {
        anyhow::bail!("Invalid hunk header: {header}");
    }

    let (old_start, old_count) = parse_range(parts[1].trim_start_matches('-'))?;
    let (new_start, new_count) = parse_range(parts[2].trim_start_matches('+'))?;

    Ok((old_start, old_count, new_start, new_count))
}

/// Parse "start,count" or "start" into (start, count).
fn parse_range(s: &str) -> Result<(usize, usize)> {
    if let Some((start, count)) = s.split_once(',') {
        Ok((start.parse()?, count.parse()?))
    } else {
        Ok((s.parse()?, 1))
    }
}

/// Resolve a path relative to the working directory.
fn resolve_path(file_path: &str, cwd: &str) -> String {
    if Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        Path::new(cwd)
            .join(file_path)
            .to_string_lossy()
            .to_string()
    }
}

/// Apply all hunks to a single file.
async fn apply_file_patch(path: &str, hunks: &[Hunk]) -> Result<usize> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // New file — only additions allowed
            String::new()
        }
        Err(e) => anyhow::bail!("Failed to read {path}: {e}"),
    };

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    // Track if file ended with newline
    let had_trailing_newline = content.ends_with('\n');

    // Sort hunks by position, then apply in reverse order to preserve line numbers
    let mut sorted_hunks: Vec<&Hunk> = hunks.iter().collect();
    sorted_hunks.sort_by_key(|h| h.old_start);
    let mut applied = 0;
    for hunk in sorted_hunks.iter().rev() {
        apply_hunk(&mut lines, hunk)?;
        applied += 1;
    }

    // Reconstruct file content
    let mut output = lines.join("\n");
    if had_trailing_newline || content.is_empty() {
        output.push('\n');
    }

    tokio::fs::write(path, &output).await?;
    Ok(applied)
}

/// Apply a single hunk to the line buffer.
fn apply_hunk(lines: &mut Vec<String>, hunk: &Hunk) -> Result<()> {
    // Convert 1-indexed to 0-indexed
    let start = if hunk.old_start == 0 { 0 } else { hunk.old_start - 1 };

    // Verify context lines match
    let mut file_idx = start;
    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(expected) => {
                if file_idx >= lines.len() {
                    anyhow::bail!(
                        "Context mismatch at line {}: expected '{}' but file has fewer lines",
                        file_idx + 1,
                        expected
                    );
                }
                if lines[file_idx].trim_end() != expected.trim_end() {
                    anyhow::bail!(
                        "Context mismatch at line {}: expected '{}', got '{}'",
                        file_idx + 1,
                        expected,
                        lines[file_idx]
                    );
                }
                file_idx += 1;
            }
            HunkLine::Remove(expected) => {
                if file_idx >= lines.len() {
                    anyhow::bail!(
                        "Remove mismatch at line {}: expected '{}' but file has fewer lines",
                        file_idx + 1,
                        expected
                    );
                }
                if lines[file_idx].trim_end() != expected.trim_end() {
                    anyhow::bail!(
                        "Remove mismatch at line {}: expected '{}', got '{}'",
                        file_idx + 1,
                        expected,
                        lines[file_idx]
                    );
                }
                file_idx += 1;
            }
            HunkLine::Add(_) => {
                // Additions don't consume file lines during verification
            }
        }
    }

    // Apply the hunk: remove old lines, insert new lines
    let mut new_lines = Vec::new();
    let mut remove_count = 0;
    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(s) => {
                new_lines.push(s.clone());
                remove_count += 1;
            }
            HunkLine::Remove(_) => {
                remove_count += 1;
            }
            HunkLine::Add(s) => {
                new_lines.push(s.clone());
            }
        }
    }

    // Splice: remove old range, insert new lines
    let end = (start + remove_count).min(lines.len());
    lines.splice(start..end, new_lines);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -10,5 +12,7 @@ fn main()").unwrap();
        assert_eq!((os, oc, ns, nc), (10, 5, 12, 7));
    }

    #[test]
    fn test_parse_hunk_header_single_line() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -1 +1 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (1, 1, 1, 1));
    }

    #[test]
    fn test_parse_file_header() {
        assert_eq!(parse_file_header("--- a/src/main.rs", "--- "), "src/main.rs");
        assert_eq!(parse_file_header("+++ b/src/main.rs", "+++ "), "src/main.rs");
        assert_eq!(parse_file_header("--- /dev/null", "--- "), "/dev/null");
    }

    #[test]
    fn test_parse_patch_basic() {
        let patch = "\
--- a/hello.txt
+++ b/hello.txt
@@ -1,3 +1,3 @@
 line one
-line two
+line TWO
 line three
";
        let files = parse_patch(patch).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "hello.txt");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0].old_start, 1);
    }

    #[test]
    fn test_parse_patch_multi_file() {
        let patch = "\
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,2 @@
-old
+new
 keep
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-foo
+bar
";
        let files = parse_patch(patch).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[1].path, "b.txt");
    }

    #[test]
    fn test_apply_hunk_replace() {
        let mut lines = vec![
            "line one".to_string(),
            "line two".to_string(),
            "line three".to_string(),
        ];
        let hunk = Hunk {
            old_start: 1,
            old_count: 3,
            new_start: 1,
            new_count: 3,
            lines: vec![
                HunkLine::Context("line one".into()),
                HunkLine::Remove("line two".into()),
                HunkLine::Add("line TWO".into()),
                HunkLine::Context("line three".into()),
            ],
        };
        apply_hunk(&mut lines, &hunk).unwrap();
        assert_eq!(lines, vec!["line one", "line TWO", "line three"]);
    }

    #[test]
    fn test_apply_hunk_add_lines() {
        let mut lines = vec![
            "one".to_string(),
            "three".to_string(),
        ];
        let hunk = Hunk {
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 3,
            lines: vec![
                HunkLine::Context("one".into()),
                HunkLine::Add("two".into()),
                HunkLine::Context("three".into()),
            ],
        };
        apply_hunk(&mut lines, &hunk).unwrap();
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    #[test]
    fn test_apply_hunk_context_mismatch() {
        let mut lines = vec![
            "actual content".to_string(),
        ];
        let hunk = Hunk {
            old_start: 1,
            old_count: 1,
            new_start: 1,
            new_count: 1,
            lines: vec![
                HunkLine::Context("expected content".into()),
            ],
        };
        let result = apply_hunk(&mut lines, &hunk);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_apply_patch_tool_basic() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello\nworld\n").unwrap();

        let tool = ApplyPatchTool;
        let ctx = crate::ToolContext {
            cwd: dir.path().to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        };

        let patch = format!(
            "--- a/test.txt\n+++ b/test.txt\n@@ -1,2 +1,2 @@\n hello\n-world\n+universe\n"
        );

        let result = tool.call(json!({"patch": patch}), &ctx).await.unwrap();
        assert!(!result.is_error, "Error: {}", result.output);

        let content = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "hello\nuniverse\n");
    }

    #[tokio::test]
    async fn test_apply_patch_tool_invalid() {
        let tool = ApplyPatchTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        };

        let result = tool.call(json!({"patch": "not a valid patch"}), &ctx).await.unwrap();
        assert!(result.is_error);
    }
}
