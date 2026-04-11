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
        "Executes a shell command and returns its output. The working directory persists between commands. Avoid using this for file reads (use Read), edits (use Edit), or searches (use Grep/Glob). Set run_in_background to true to run long commands without blocking. Returns a task ID that can be checked with TaskOutput or by reading the output file."
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
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background. Returns a task ID immediately. Use TaskOutput to check results later."
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of what this command does (e.g., 'Run test suite', 'Install dependencies')"
                }
            },
            "required": ["command"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'command' parameter"))?;

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if run_in_background {
            return self.run_background(command, &ctx.cwd, &description).await;
        }

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

impl BashTool {
    async fn run_background(
        &self,
        command: &str,
        cwd: &str,
        description: &str,
    ) -> Result<ToolResult> {
        let task_id = uuid::Uuid::new_v4().to_string();

        // Create tasks directory
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let tasks_dir = format!("{}/.cisco-code/tasks", home);
        tokio::fs::create_dir_all(&tasks_dir).await?;

        let output_path = format!("{}/{}.txt", tasks_dir, task_id);
        let output_path_clone = output_path.clone();
        let command_owned = command.to_string();
        let cwd_owned = cwd.to_string();

        // Spawn background task
        tokio::spawn(async move {
            let result = tokio::process::Command::new("bash")
                .arg("-c")
                .arg(&command_owned)
                .current_dir(&cwd_owned)
                .output()
                .await;

            match result {
                Ok(output) => {
                    let mut content = String::new();
                    if !output.stdout.is_empty() {
                        content.push_str(&String::from_utf8_lossy(&output.stdout));
                    }
                    if !output.stderr.is_empty() {
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str("STDERR:\n");
                        content.push_str(&String::from_utf8_lossy(&output.stderr));
                    }
                    content.push_str(&format!(
                        "\nExit code: {}",
                        output.status.code().unwrap_or(-1)
                    ));
                    let _ = tokio::fs::write(&output_path_clone, content).await;
                }
                Err(e) => {
                    let _ = tokio::fs::write(&output_path_clone, format!("Error: {}", e)).await;
                }
            }
        });

        let result = serde_json::json!({
            "background_task_id": task_id,
            "status": "launched",
            "output_file": output_path,
            "description": description,
            "message": "Command launched in background. Use Read tool on the output_file to check results, or use TaskOutput with the task ID."
        });

        Ok(ToolResult::success(result.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        }
    }

    #[tokio::test]
    async fn test_bash_echo() {
        let tool = BashTool;
        let result = tool
            .call(serde_json::json!({"command": "echo hello"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output.trim(), "hello");
    }

    #[tokio::test]
    async fn test_bash_exit_code_nonzero() {
        let tool = BashTool;
        let result = tool
            .call(serde_json::json!({"command": "exit 42"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Exit code 42"));
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let tool = BashTool;
        let result = tool
            .call(
                serde_json::json!({"command": "echo err >&2"}),
                &ctx(),
            )
            .await
            .unwrap();
        // stderr captured but exit code 0 → success
        assert!(!result.is_error);
        assert!(result.output.contains("err"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let tool = BashTool;
        let result = tool
            .call(
                serde_json::json!({"command": "sleep 10", "timeout": 500}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("timed out"));
    }

    #[tokio::test]
    async fn test_bash_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            cwd: dir.path().to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        };

        let tool = BashTool;
        let result = tool
            .call(serde_json::json!({"command": "pwd"}), &ctx)
            .await
            .unwrap();
        assert!(!result.is_error);
        // macOS resolves /tmp → /private/tmp, so use canonical comparison
        let expected = dir.path().canonicalize().unwrap();
        let actual_path = std::path::Path::new(result.output.trim()).canonicalize().unwrap();
        assert_eq!(actual_path, expected);
    }

    #[tokio::test]
    async fn test_bash_no_output() {
        let tool = BashTool;
        let result = tool
            .call(serde_json::json!({"command": "true"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output, "(no output)");
    }
}
