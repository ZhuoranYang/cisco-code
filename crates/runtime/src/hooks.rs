//! Hook system for lifecycle event handling.
//!
//! Design insight from Claude Code: Hooks fire at pre/post tool use,
//! session start/end, and can modify tool input or suppress execution.
//!
//! Hooks are shell commands that run in response to lifecycle events.
//! They receive event data as JSON on stdin and can:
//! - Pre-tool: modify tool input (stdout JSON) or suppress execution (exit code 1)
//! - Post-tool: observe tool results (informational only)
//! - Session: run setup/teardown commands

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A lifecycle event that can trigger hooks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before a tool is executed. Hook can modify input or block execution.
    PreToolUse,
    /// After a tool has executed. Informational only.
    PostToolUse,
    /// When a new session starts.
    SessionStart,
    /// When a session ends.
    SessionEnd,
    /// Before sending a message to the LLM.
    PreMessage,
    /// After receiving a response from the LLM.
    PostMessage,
    /// Fired when the agent is stopped (Ctrl+C, cancel, max turns, completed).
    Stop,
    /// Fired when a subagent completes or is cancelled.
    SubagentStop,
    /// Fired when a notification is about to be sent to the user.
    Notification,
    /// Fired when the user submits a prompt, before it's processed.
    /// Hook can modify the prompt (stdout JSON with "prompt" key) or suppress it (exit 1).
    UserPromptSubmit,
    /// Fired after a file is created or modified by a tool (Write, Edit, ApplyPatch).
    FileChanged,
    /// Fired before context compaction begins.
    CompactionStart,
    /// Fired after context compaction completes.
    CompactionEnd,
}

/// A configured hook: an event trigger + shell command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Which event triggers this hook.
    pub event: HookEvent,
    /// Shell command to execute.
    pub command: String,
    /// Optional: only trigger for specific tool names (for tool events).
    #[serde(default)]
    pub tool_filter: Option<String>,
    /// Timeout for the hook command.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Whether hook failure should abort the operation.
    #[serde(default)]
    pub required: bool,
}

fn default_timeout_ms() -> u64 {
    5000
}

/// Data passed to a hook via stdin as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInput {
    /// The event type.
    pub event: HookEvent,
    /// Session ID.
    pub session_id: String,
    /// Tool name (for tool events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tool input (for pre_tool_use).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    /// Tool result (for post_tool_use).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<String>,
    /// Whether the tool result was an error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Subagent identifier (for SubagentStop events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_id: Option<String>,
    /// Reason the agent stopped (for Stop events), e.g. "user_cancel", "max_turns", "completed".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Notification payload (for Notification events), typically {title, body, level}.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<serde_json::Value>,
    /// File path affected (for FileChanged events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// File operation type (for FileChanged events): "write", "edit", "patch", "delete".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_operation: Option<String>,
    /// User prompt text (for UserPromptSubmit events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Compaction summary tokens (for CompactionEnd events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_tokens: Option<u64>,
}

/// Result of running a hook.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Hook completed successfully, proceed normally.
    Continue,
    /// Hook modified the tool input (pre_tool_use only).
    ModifiedInput(serde_json::Value),
    /// Hook requested suppression of the tool execution.
    Suppress {
        /// Message to return as the tool result instead.
        message: String,
    },
    /// Hook explicitly approved tool execution, bypassing the permission engine.
    /// This enables enterprise policy hooks that programmatically control tool access.
    Approve,
    /// Hook approved AND provided modified input (pre_tool_use only).
    /// This happens when a hook outputs JSON with both `"decision": "approve"`
    /// and additional fields that modify the tool input.
    ApproveWithModifiedInput(serde_json::Value),
    /// Hook explicitly denied tool execution, bypassing the permission engine.
    Deny {
        /// Reason for denial.
        reason: String,
    },
    /// Hook failed.
    Error {
        command: String,
        message: String,
    },
}

/// The hook runner manages and executes lifecycle hooks.
pub struct HookRunner {
    hooks: Vec<HookConfig>,
    /// Working directory for hook execution.
    cwd: PathBuf,
}

impl HookRunner {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            hooks: Vec::new(),
            cwd: cwd.into(),
        }
    }

    /// Load hooks from a configuration map (e.g., from TOML).
    pub fn with_hooks(mut self, hooks: Vec<HookConfig>) -> Self {
        self.hooks = hooks;
        self
    }

    /// Add a single hook.
    pub fn add_hook(&mut self, hook: HookConfig) {
        self.hooks.push(hook);
    }

    /// Get hooks registered for a specific event.
    pub fn hooks_for_event(&self, event: &HookEvent) -> Vec<&HookConfig> {
        self.hooks
            .iter()
            .filter(|h| h.event == *event)
            .collect()
    }

    /// Run all hooks for a given event with the provided input data.
    ///
    /// For PreToolUse: returns ModifiedInput if any hook modifies input,
    /// or Suppress if any hook exits with code 1.
    ///
    /// For other events: returns Continue unless a required hook fails.
    pub async fn run(&self, input: &HookInput) -> HookResult {
        let hooks = self.hooks_for_event(&input.event);

        if hooks.is_empty() {
            return HookResult::Continue;
        }

        for hook in hooks {
            // Check tool filter — skip this hook if filter is set but doesn't match.
            // A tool-filtered hook should NOT run when there's no tool_name to match.
            if let Some(ref filter) = hook.tool_filter {
                match &input.tool_name {
                    Some(tool_name) if matches_hook_filter(filter, tool_name) => {}
                    _ => continue, // no tool_name or no match → skip
                }
            }

            match self.execute_hook(hook, input).await {
                Ok(result) => match result {
                    HookResult::Continue => continue,
                    HookResult::ModifiedInput(_)
                    | HookResult::Suppress { .. }
                    | HookResult::Approve
                    | HookResult::ApproveWithModifiedInput(_)
                    | HookResult::Deny { .. } => {
                        return result;
                    }
                    HookResult::Error { .. } => {
                        if hook.required {
                            return result;
                        }
                        // Non-required hook failure: log and continue
                        tracing::warn!(
                            "Non-required hook '{}' failed, continuing",
                            hook.command
                        );
                        continue;
                    }
                },
                Err(e) => {
                    let error_result = HookResult::Error {
                        command: hook.command.clone(),
                        message: e.to_string(),
                    };
                    if hook.required {
                        return error_result;
                    }
                    tracing::warn!(
                        "Non-required hook '{}' failed: {e}, continuing",
                        hook.command
                    );
                    continue;
                }
            }
        }

        HookResult::Continue
    }

    /// Execute a single hook command.
    async fn execute_hook(
        &self,
        hook: &HookConfig,
        input: &HookInput,
    ) -> Result<HookResult> {
        let input_json = serde_json::to_string(input)?;
        let timeout = Duration::from_millis(hook.timeout_ms);

        let result = tokio::time::timeout(timeout, async {
            let mut child = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&hook.command)
                .current_dir(&self.cwd)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            // Write input JSON to stdin
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(input_json.as_bytes()).await?;
                drop(stdin);
            }

            let output = child.wait_with_output().await?;
            Ok::<_, anyhow::Error>(output)
        })
        .await;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                match exit_code {
                    0 => {
                        // Check if stdout contains JSON with an action or modified input
                        if !stdout.trim().is_empty() {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(
                                stdout.trim(),
                            ) {
                                // Check for explicit permission decisions
                                match json.get("decision").and_then(|d| d.as_str()) {
                                    Some("approve") => {
                                        // If this is a pre_tool_use hook and the JSON has
                                        // fields beyond "decision" (and optional "reason"),
                                        // treat it as approve + modified input.
                                        if input.event == HookEvent::PreToolUse {
                                            let has_extra_fields = json.as_object()
                                                .map(|obj| obj.keys().any(|k| k != "decision" && k != "reason"))
                                                .unwrap_or(false);
                                            if has_extra_fields {
                                                return Ok(HookResult::ApproveWithModifiedInput(json));
                                            }
                                        }
                                        return Ok(HookResult::Approve);
                                    }
                                    Some("deny") => {
                                        let reason = json["reason"]
                                            .as_str()
                                            .unwrap_or("denied by hook")
                                            .to_string();
                                        return Ok(HookResult::Deny { reason });
                                    }
                                    _ => {}
                                }
                                // For pre_tool_use: treat JSON as modified input
                                if input.event == HookEvent::PreToolUse {
                                    return Ok(HookResult::ModifiedInput(json));
                                }
                            }
                        }
                        Ok(HookResult::Continue)
                    }
                    1 => {
                        // Exit code 1 = suppress tool execution
                        let message = if !stderr.trim().is_empty() {
                            stderr.trim().to_string()
                        } else if !stdout.trim().is_empty() {
                            stdout.trim().to_string()
                        } else {
                            format!("Hook '{}' blocked execution", hook.command)
                        };
                        Ok(HookResult::Suppress { message })
                    }
                    code => Ok(HookResult::Error {
                        command: hook.command.clone(),
                        message: format!(
                            "Hook exited with code {code}. stderr: {stderr}"
                        ),
                    }),
                }
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(HookResult::Error {
                command: hook.command.clone(),
                message: format!(
                    "Hook timed out after {}ms",
                    hook.timeout_ms
                ),
            }),
        }
    }

    /// Return the number of registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }
}

/// Check if a tool name matches a hook's tool filter.
/// Supports exact match and wildcard suffix.
fn matches_hook_filter(filter: &str, tool_name: &str) -> bool {
    if filter == "*" {
        return true;
    }
    if let Some(prefix) = filter.strip_suffix('*') {
        return tool_name.starts_with(prefix);
    }
    filter == tool_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_runner_empty() {
        let runner = HookRunner::new(".");
        assert_eq!(runner.hook_count(), 0);
        assert!(runner.hooks_for_event(&HookEvent::PreToolUse).is_empty());
    }

    #[test]
    fn test_add_hook() {
        let mut runner = HookRunner::new(".");
        runner.add_hook(HookConfig {
            event: HookEvent::PreToolUse,
            command: "echo ok".into(),
            tool_filter: None,
            timeout_ms: 5000,
            required: false,
        });
        assert_eq!(runner.hook_count(), 1);
        assert_eq!(runner.hooks_for_event(&HookEvent::PreToolUse).len(), 1);
        assert!(runner.hooks_for_event(&HookEvent::PostToolUse).is_empty());
    }

    #[test]
    fn test_hooks_for_event_filters_correctly() {
        let runner = HookRunner::new(".").with_hooks(vec![
            HookConfig {
                event: HookEvent::PreToolUse,
                command: "pre1".into(),
                tool_filter: None,
                timeout_ms: 5000,
                required: false,
            },
            HookConfig {
                event: HookEvent::PostToolUse,
                command: "post1".into(),
                tool_filter: None,
                timeout_ms: 5000,
                required: false,
            },
            HookConfig {
                event: HookEvent::PreToolUse,
                command: "pre2".into(),
                tool_filter: Some("Bash".into()),
                timeout_ms: 5000,
                required: false,
            },
        ]);

        assert_eq!(runner.hooks_for_event(&HookEvent::PreToolUse).len(), 2);
        assert_eq!(runner.hooks_for_event(&HookEvent::PostToolUse).len(), 1);
        assert_eq!(runner.hooks_for_event(&HookEvent::SessionStart).len(), 0);
    }

    #[test]
    fn test_matches_hook_filter_exact() {
        assert!(matches_hook_filter("Bash", "Bash"));
        assert!(!matches_hook_filter("Bash", "Read"));
    }

    #[test]
    fn test_matches_hook_filter_wildcard() {
        assert!(matches_hook_filter("*", "anything"));
        assert!(matches_hook_filter("mcp:*", "mcp:github"));
        assert!(!matches_hook_filter("mcp:*", "Bash"));
    }

    #[test]
    fn test_hook_input_serialization() {
        let input = HookInput {
            event: HookEvent::PreToolUse,
            session_id: "sess-123".into(),
            tool_name: Some("Bash".into()),
            tool_input: Some(serde_json::json!({"command": "ls"})),
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("pre_tool_use"));
        assert!(json.contains("Bash"));
        assert!(json.contains("sess-123"));
        assert!(!json.contains("tool_result")); // skip_serializing_if
    }

    #[test]
    fn test_hook_event_serialization() {
        let event = HookEvent::PreToolUse;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, "\"pre_tool_use\"");

        let event = HookEvent::SessionStart;
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(json, "\"session_start\"");
    }

    #[tokio::test]
    async fn test_run_no_hooks_returns_continue() {
        let runner = HookRunner::new(".");
        let input = HookInput {
            event: HookEvent::PreToolUse,
            session_id: "test".into(),
            tool_name: Some("Bash".into()),
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_run_simple_hook_continue() {
        let runner = HookRunner::new(".").with_hooks(vec![HookConfig {
            event: HookEvent::PostToolUse,
            command: "true".into(), // exits 0
            tool_filter: None,
            timeout_ms: 5000,
            required: false,
        }]);

        let input = HookInput {
            event: HookEvent::PostToolUse,
            session_id: "test".into(),
            tool_name: Some("Bash".into()),
            tool_input: None,
            tool_result: Some("ok".into()),
            is_error: Some(false),
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_run_hook_suppress() {
        let runner = HookRunner::new(".").with_hooks(vec![HookConfig {
            event: HookEvent::PreToolUse,
            command: "echo 'blocked by policy' >&2; exit 1".into(),
            tool_filter: None,
            timeout_ms: 5000,
            required: false,
        }]);

        let input = HookInput {
            event: HookEvent::PreToolUse,
            session_id: "test".into(),
            tool_name: Some("Bash".into()),
            tool_input: Some(serde_json::json!({"command": "rm -rf /"})),
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Suppress { .. }));
        if let HookResult::Suppress { message } = result {
            assert!(message.contains("blocked by policy"));
        }
    }

    #[tokio::test]
    async fn test_run_hook_with_tool_filter_skips_non_matching() {
        let runner = HookRunner::new(".").with_hooks(vec![HookConfig {
            event: HookEvent::PreToolUse,
            command: "exit 1".into(), // would suppress if it ran
            tool_filter: Some("Write".into()),
            timeout_ms: 5000,
            required: false,
        }]);

        let input = HookInput {
            event: HookEvent::PreToolUse,
            session_id: "test".into(),
            tool_name: Some("Read".into()), // doesn't match "Write" filter
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_hook_timeout() {
        let runner = HookRunner::new(".").with_hooks(vec![HookConfig {
            event: HookEvent::PreToolUse,
            command: "sleep 10".into(),
            tool_filter: None,
            timeout_ms: 100, // 100ms timeout
            required: true,
        }]);

        let input = HookInput {
            event: HookEvent::PreToolUse,
            session_id: "test".into(),
            tool_name: Some("Bash".into()),
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Error { .. }));
        if let HookResult::Error { message, .. } = result {
            assert!(message.contains("timed out"));
        }
    }

    #[test]
    fn test_hook_config_defaults() {
        let config: HookConfig = serde_json::from_str(
            r#"{"event": "pre_tool_use", "command": "echo ok"}"#,
        )
        .unwrap();
        assert_eq!(config.timeout_ms, 5000);
        assert!(!config.required);
        assert!(config.tool_filter.is_none());
    }

    #[test]
    fn test_default_timeout_ms() {
        assert_eq!(default_timeout_ms(), 5000);
    }

    #[test]
    fn test_new_event_serialization() {
        let stop = HookEvent::Stop;
        assert_eq!(serde_json::to_string(&stop).unwrap(), "\"stop\"");

        let subagent_stop = HookEvent::SubagentStop;
        assert_eq!(
            serde_json::to_string(&subagent_stop).unwrap(),
            "\"subagent_stop\""
        );

        let notification = HookEvent::Notification;
        assert_eq!(
            serde_json::to_string(&notification).unwrap(),
            "\"notification\""
        );
    }

    #[test]
    fn test_new_event_deserialization() {
        let stop: HookEvent = serde_json::from_str("\"stop\"").unwrap();
        assert_eq!(stop, HookEvent::Stop);

        let subagent_stop: HookEvent =
            serde_json::from_str("\"subagent_stop\"").unwrap();
        assert_eq!(subagent_stop, HookEvent::SubagentStop);

        let notification: HookEvent =
            serde_json::from_str("\"notification\"").unwrap();
        assert_eq!(notification, HookEvent::Notification);
    }

    #[test]
    fn test_stop_input_serialization() {
        let input = HookInput {
            event: HookEvent::Stop,
            session_id: "sess-456".into(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: Some("user_cancel".into()),
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"stop\""));
        assert!(json.contains("\"user_cancel\""));
        assert!(!json.contains("subagent_id")); // skip_serializing_if
        assert!(!json.contains("notification")); // skip_serializing_if
    }

    #[test]
    fn test_subagent_stop_input_serialization() {
        let input = HookInput {
            event: HookEvent::SubagentStop,
            session_id: "sess-789".into(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: Some("sub-001".into()),
            stop_reason: Some("completed".into()),
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"subagent_stop\""));
        assert!(json.contains("\"sub-001\""));
        assert!(json.contains("\"completed\""));
    }

    #[test]
    fn test_notification_input_serialization() {
        let input = HookInput {
            event: HookEvent::Notification,
            session_id: "sess-abc".into(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: Some(serde_json::json!({
                "title": "Task complete",
                "body": "Build succeeded",
                "level": "info"
            })),
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };

        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"notification\""));
        assert!(json.contains("Task complete"));
        assert!(json.contains("Build succeeded"));
        assert!(json.contains("info"));
        assert!(!json.contains("stop_reason")); // skip_serializing_if
    }

    #[test]
    fn test_hooks_for_new_events_filter_correctly() {
        let runner = HookRunner::new(".").with_hooks(vec![
            HookConfig {
                event: HookEvent::Stop,
                command: "on-stop.sh".into(),
                tool_filter: None,
                timeout_ms: 5000,
                required: false,
            },
            HookConfig {
                event: HookEvent::SubagentStop,
                command: "on-subagent-stop.sh".into(),
                tool_filter: None,
                timeout_ms: 5000,
                required: false,
            },
            HookConfig {
                event: HookEvent::Notification,
                command: "on-notify.sh".into(),
                tool_filter: None,
                timeout_ms: 5000,
                required: false,
            },
        ]);

        assert_eq!(runner.hooks_for_event(&HookEvent::Stop).len(), 1);
        assert_eq!(runner.hooks_for_event(&HookEvent::SubagentStop).len(), 1);
        assert_eq!(runner.hooks_for_event(&HookEvent::Notification).len(), 1);
        assert_eq!(runner.hooks_for_event(&HookEvent::PreToolUse).len(), 0);
    }

    #[test]
    fn test_hook_config_deserialize_new_events() {
        let config: HookConfig = serde_json::from_str(
            r#"{"event": "stop", "command": "cleanup.sh"}"#,
        )
        .unwrap();
        assert_eq!(config.event, HookEvent::Stop);

        let config: HookConfig = serde_json::from_str(
            r#"{"event": "subagent_stop", "command": "log-subagent.sh"}"#,
        )
        .unwrap();
        assert_eq!(config.event, HookEvent::SubagentStop);

        let config: HookConfig = serde_json::from_str(
            r#"{"event": "notification", "command": "notify.sh"}"#,
        )
        .unwrap();
        assert_eq!(config.event, HookEvent::Notification);
    }

    #[tokio::test]
    async fn test_run_stop_hook() {
        let runner = HookRunner::new(".").with_hooks(vec![HookConfig {
            event: HookEvent::Stop,
            command: "true".into(),
            tool_filter: None,
            timeout_ms: 5000,
            required: false,
        }]);

        let input = HookInput {
            event: HookEvent::Stop,
            session_id: "test".into(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: Some("max_turns".into()),
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let result = runner.run(&input).await;
        assert!(matches!(result, HookResult::Continue));
    }
}
