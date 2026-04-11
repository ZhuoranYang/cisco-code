//! Subagent system — spawn child agents with different model classes.
//!
//! Design insight from Claude Code: The Agent tool spawns subagents for parallel
//! work, background research, or tasks requiring a different model class.
//!
//! Subagents are lightweight: they share the same tool registry but get their own
//! session, model config, and conversation loop. Results are returned as a single
//! message to the parent agent.

use cisco_code_api::{AssistantEvent, CompletionRequest, Provider};
use cisco_code_protocol::{StopReason, TokenUsage};
use uuid::Uuid;

use crate::hooks::{HookEvent, HookInput, HookRunner};

/// Configuration for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Human-readable description of the subagent's task.
    pub description: String,
    /// The prompt/task for the subagent to execute.
    pub prompt: String,
    /// Model to use (overrides parent's model).
    pub model: Option<String>,
    /// Maximum turns the subagent can take.
    pub max_turns: u32,
    /// Maximum output tokens per turn.
    pub max_tokens: u32,
    /// System prompt override (if None, uses a minimal subagent prompt).
    pub system_prompt: Option<String>,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            description: "subagent".into(),
            prompt: String::new(),
            model: None,
            max_turns: 10,
            max_tokens: 8192,
            system_prompt: None,
        }
    }
}

/// Result of a subagent execution.
#[derive(Debug, Clone)]
pub struct SubagentResult {
    /// The subagent's text output (concatenated from all turns).
    pub output: String,
    /// Total token usage across all subagent turns.
    pub usage: TokenUsage,
    /// Number of turns the subagent took.
    pub turns: u32,
    /// Whether the subagent finished successfully.
    pub success: bool,
    /// Error message if the subagent failed.
    pub error: Option<String>,
}

/// Run a subagent with the given provider and tools.
///
/// The subagent runs a simplified conversation loop with optional hook
/// integration. When `hooks` is provided, a `SubagentStop` event is fired
/// after the subagent completes (successfully or with error).
pub async fn run_subagent(
    config: &SubagentConfig,
    provider: &dyn Provider,
    tool_defs: Vec<cisco_code_protocol::ToolDefinition>,
    tools: &cisco_code_tools::ToolRegistry,
    hooks: Option<&HookRunner>,
) -> SubagentResult {
    let subagent_id = Uuid::new_v4().to_string();
    let model = config.model.clone().unwrap_or_else(|| "claude-sonnet-4-6".into());

    let system_prompt = config.system_prompt.clone().unwrap_or_else(|| {
        format!(
            "You are a subagent performing a specific task. Complete the task and report your findings concisely.\n\
             Task: {}\n\n\
             Guidelines:\n\
             - Focus only on the assigned task\n\
             - Be concise in your response\n\
             - Use tools as needed to complete the task\n\
             - Do not ask follow-up questions — just do the work",
            config.description,
        )
    });

    let mut messages = vec![cisco_code_api::ApiMessage {
        role: "user".into(),
        content: serde_json::json!(config.prompt),
    }];

    let mut total_output = String::new();
    let mut total_usage = TokenUsage::default();
    let mut turns = 0u32;
    let mut stop_reason = StopReason::ToolUse;

    let tool_ctx = cisco_code_tools::ToolContext {
        cwd: ".".into(),
        interactive: false,
        progress_tx: None,
    };

    while stop_reason == StopReason::ToolUse && turns < config.max_turns {
        turns += 1;

        let request = CompletionRequest {
            model: model.clone(),
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: tool_defs.clone(),
            max_tokens: config.max_tokens,
            temperature: Some(0.0),
            thinking: None,
            system_blocks: None,
        };

        let events = match provider.stream(request).await {
            Ok(events) => events,
            Err(e) => {
                let result = SubagentResult {
                    output: total_output,
                    usage: total_usage,
                    turns,
                    success: false,
                    error: Some(format!("Provider error: {e}")),
                };
                fire_subagent_stop(hooks, &subagent_id, &result).await;
                return result;
            }
        };

        let mut text_parts = Vec::new();
        let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();
        stop_reason = StopReason::EndTurn;

        for event in &events {
            match event {
                AssistantEvent::TextDelta(text) => {
                    text_parts.push(text.clone());
                }
                AssistantEvent::ThinkingDelta(_) => {
                    // Subagent thinking — not surfaced
                }
                AssistantEvent::ToolUse { id, name, input } => {
                    tool_uses.push((id.clone(), name.clone(), input.clone()));
                }
                AssistantEvent::Usage { input_tokens, output_tokens } => {
                    total_usage.input_tokens += input_tokens;
                    total_usage.output_tokens += output_tokens;
                }
                AssistantEvent::MessageStop { stop_reason: reason } => {
                    stop_reason = match reason.as_str() {
                        "tool_use" => StopReason::ToolUse,
                        "end_turn" => StopReason::EndTurn,
                        "max_tokens" => StopReason::MaxTokens,
                        _ => StopReason::EndTurn,
                    };
                }
            }
        }

        let full_text: String = text_parts.concat();
        if !full_text.is_empty() {
            total_output.push_str(&full_text);
        }

        // Build assistant message content for conversation history
        let mut assistant_content = Vec::new();
        if !full_text.is_empty() {
            assistant_content.push(serde_json::json!({"type": "text", "text": full_text}));
        }
        for (id, name, input) in &tool_uses {
            assistant_content.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input,
            }));
        }

        messages.push(cisco_code_api::ApiMessage {
            role: "assistant".into(),
            content: serde_json::json!(assistant_content),
        });

        // Execute tools
        if stop_reason == StopReason::ToolUse {
            let mut tool_results = Vec::new();

            for (tool_id, tool_name, tool_input) in &tool_uses {
                let result = match tools.execute(tool_name, tool_input.clone(), &tool_ctx).await {
                    Ok(r) => r,
                    Err(e) => cisco_code_protocol::ToolResult::error(format!("Tool error: {e}")),
                };

                tool_results.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": result.output,
                    "is_error": result.is_error,
                }));
            }

            messages.push(cisco_code_api::ApiMessage {
                role: "user".into(),
                content: serde_json::json!(tool_results),
            });
        }
    }

    let result = SubagentResult {
        output: total_output,
        usage: total_usage,
        turns,
        success: true,
        error: None,
    };
    fire_subagent_stop(hooks, &subagent_id, &result).await;
    result
}

/// Fire the `SubagentStop` hook event if a `HookRunner` is available.
async fn fire_subagent_stop(
    hooks: Option<&HookRunner>,
    subagent_id: &str,
    result: &SubagentResult,
) {
    if let Some(hooks) = hooks {
        let stop_input = HookInput {
            event: HookEvent::SubagentStop,
            session_id: String::new(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: Some(!result.success),
            subagent_id: Some(subagent_id.to_string()),
            stop_reason: Some(
                if result.success { "completed" } else { "error" }.to_string(),
            ),
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: None,
            summary_tokens: None,
        };
        let _ = hooks.run(&stop_input).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_config_default() {
        let config = SubagentConfig::default();
        assert_eq!(config.max_turns, 10);
        assert_eq!(config.max_tokens, 8192);
        assert!(config.model.is_none());
        assert!(config.system_prompt.is_none());
    }

    #[test]
    fn test_subagent_result_success() {
        let result = SubagentResult {
            output: "found 3 files".into(),
            usage: TokenUsage::default(),
            turns: 2,
            success: true,
            error: None,
        };
        assert!(result.success);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_subagent_result_failure() {
        let result = SubagentResult {
            output: String::new(),
            usage: TokenUsage::default(),
            turns: 1,
            success: false,
            error: Some("API timeout".into()),
        };
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("timeout"));
    }
}
