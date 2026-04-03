//! Bash tool — execute shell commands.
//!
//! Pattern from Claude Code's BashTool: spawn process, capture stdout/stderr,
//! handle timeouts with SIGTERM → SIGKILL escalation.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::{Tool, ToolContext};

pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Executes a shell command and returns its output. The working directory persists between commands. Avoid using this for file reads (use Read), edits (use Edit), or searches (use Grep/Glob)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 120000, max 600000)"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

        let timeout_ms = input["timeout"].as_u64().unwrap_or(120_000).min(600_000);

        // Spawn shell process
        let mut child = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        // Read output with timeout
        let timeout = std::time::Duration::from_millis(timeout_ms);
        let result = tokio::time::timeout(timeout, async {
            let mut stdout = String::new();
            let mut stderr = String::new();

            if let Some(mut out) = child.stdout.take() {
                out.read_to_string(&mut stdout).await?;
            }
            if let Some(mut err) = child.stderr.take() {
                err.read_to_string(&mut stderr).await?;
            }

            let status = child.wait().await?;
            Ok::<_, anyhow::Error>((stdout, stderr, status.code().unwrap_or(-1)))
        })
        .await;

        match result {
            Ok(Ok((stdout, stderr, code))) => {
                let mut output = String::new();
                if !stdout.is_empty() {
                    output.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !output.is_empty() {
                        output.push('\n');
                    }
                    output.push_str(&stderr);
                }
                if output.is_empty() {
                    output = "(no output)".to_string();
                }

                if code != 0 {
                    Ok(ToolResult {
                        output: format!("Exit code {code}\n{output}"),
                        is_error: true,
                        injected_messages: None,
                    })
                } else {
                    Ok(ToolResult::success(output))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Command failed: {e}"))),
            Err(_) => {
                // Timeout — kill the process
                let _ = child.kill().await;
                Ok(ToolResult::error(format!(
                    "Command timed out after {timeout_ms}ms"
                )))
            }
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}
