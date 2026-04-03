//! The core conversation runtime — the heart of cisco-code.
//!
//! Design insight from Claw-Code-Parity: `ConversationRuntime<P>` is generic
//! over Provider, making it testable and composable.
//!
//! Design insight from Claude Code: The turn loop yields StreamEvents for
//! real-time TUI rendering (generator pattern). Tool results feed back into
//! the conversation for multi-turn tool use chains.
//!
//! The agent loop:
//! 1. Build CompletionRequest from session + config + tools
//! 2. Call provider.stream(request) → Vec<AssistantEvent>
//! 3. Collect text deltas, emit them as StreamEvents
//! 4. For each ToolUse: check permissions → execute → collect result
//! 5. Add assistant message + tool results to session
//! 6. If stop_reason == tool_use, loop back to step 1
//! 7. If stop_reason == end_turn or budget exhausted, return

use anyhow::Result;
use cisco_code_api::{ApiMessage, AssistantEvent, CompletionRequest, Provider};
use cisco_code_protocol::{
    ContentBlock, Message, StopReason, StreamEvent, TokenUsage,
    AssistantMessage, ToolResultMessage, ToolUseMessage,
};
use cisco_code_tools::{ToolContext, ToolRegistry};
use uuid::Uuid;

use crate::config::RuntimeConfig;
use crate::prompt::{load_project_instructions, PromptBuilder};
use crate::session::Session;

/// The core conversation runtime.
///
/// Generic over `P: Provider` (LLM backend) for testability and swappability.
pub struct ConversationRuntime<P: Provider> {
    pub provider: P,
    pub tools: ToolRegistry,
    pub session: Session,
    pub config: RuntimeConfig,
    turn_count: u32,
    total_usage: TokenUsage,
}

impl<P: Provider> ConversationRuntime<P> {
    pub fn new(provider: P, tools: ToolRegistry, config: RuntimeConfig) -> Self {
        Self {
            provider,
            tools,
            session: Session::new(),
            config,
            turn_count: 0,
            total_usage: TokenUsage::default(),
        }
    }

    /// Get the current working directory for tool execution.
    fn cwd(&self) -> &str {
        // TODO: Track cwd changes from Bash cd commands
        "."
    }

    /// Build the system prompt for this turn.
    fn build_system_prompt(&self) -> String {
        let cwd = self.cwd().to_string();
        let mut builder = PromptBuilder::new(&cwd);

        if let Some(instructions) = load_project_instructions(&cwd) {
            builder = builder.with_instructions(instructions);
        }

        builder.build()
    }

    /// Convert session messages to API format.
    ///
    /// The Anthropic Messages API expects alternating user/assistant messages.
    /// Tool results must be sent as user messages with tool_result content blocks.
    fn build_api_messages(&self) -> Vec<ApiMessage> {
        let mut api_messages = Vec::new();

        for msg in &self.session.messages {
            match msg {
                Message::User(user_msg) => {
                    let content: Vec<serde_json::Value> = user_msg
                        .content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            }
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        })
                        .collect();

                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content: serde_json::json!(content),
                    });
                }

                Message::Assistant(asst_msg) => {
                    let content: Vec<serde_json::Value> = asst_msg
                        .content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input,
                                })
                            }
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        })
                        .collect();

                    api_messages.push(ApiMessage {
                        role: "assistant".into(),
                        content: serde_json::json!(content),
                    });
                }

                Message::ToolResult(result_msg) => {
                    // Tool results go as user messages with tool_result content blocks
                    // (Anthropic API requirement)
                    let content = serde_json::json!([{
                        "type": "tool_result",
                        "tool_use_id": result_msg.tool_use_id,
                        "content": result_msg.content,
                        "is_error": result_msg.is_error,
                    }]);

                    // Check if last api_message is also a user role (batch tool results)
                    if let Some(last) = api_messages.last_mut() {
                        if last.role == "user" {
                            // Append to existing user message's content array
                            if let Some(arr) = last.content.as_array_mut() {
                                if let Some(new_block) = content.as_array() {
                                    arr.extend(new_block.iter().cloned());
                                    continue;
                                }
                            }
                        }
                    }

                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content,
                    });
                }

                // System messages and ToolUse messages are handled separately
                // (system goes in system_prompt, tool_use is part of assistant messages)
                _ => {}
            }
        }

        api_messages
    }

    /// Submit a user message and execute the agent loop.
    ///
    /// Returns a stream of events for real-time rendering.
    /// The loop continues until the model stops using tools (end_turn)
    /// or a budget/turn limit is reached.
    pub async fn submit_message(&mut self, user_input: &str) -> Result<Vec<StreamEvent>> {
        // Add user message to session
        let user_msg = Message::User(cisco_code_protocol::UserMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: user_input.to_string(),
            }],
            attachments: None,
        });
        self.session.add_message(user_msg);

        let mut all_events = Vec::new();
        let mut stop_reason = StopReason::ToolUse;

        // Agent loop: continue until model stops calling tools
        while stop_reason == StopReason::ToolUse {
            self.turn_count += 1;

            all_events.push(StreamEvent::TurnStart {
                model: self.config.model.clone(),
                turn_number: self.turn_count,
            });

            // 1. Build the completion request
            let request = CompletionRequest {
                model: self.config.model.clone(),
                system_prompt: self.build_system_prompt(),
                messages: self.build_api_messages(),
                tools: self.tools.definitions(),
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
            };

            // 2. Call the provider
            let assistant_events = self.provider.stream(request).await?;

            // 3. Process events: collect text, tool uses, and stop reason
            let mut text_parts = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new(); // (id, name, input)
            let mut turn_usage = TokenUsage::default();
            stop_reason = StopReason::EndTurn; // default if no explicit stop

            for event in &assistant_events {
                match event {
                    AssistantEvent::TextDelta(text) => {
                        all_events.push(StreamEvent::TextDelta {
                            text: text.clone(),
                        });
                        text_parts.push(text.clone());
                    }
                    AssistantEvent::ToolUse { id, name, input } => {
                        all_events.push(StreamEvent::ToolUseStart {
                            tool_use_id: id.clone(),
                            tool_name: name.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    AssistantEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        turn_usage.input_tokens += input_tokens;
                        turn_usage.output_tokens += output_tokens;
                    }
                    AssistantEvent::MessageStop {
                        stop_reason: reason,
                    } => {
                        stop_reason = match reason.as_str() {
                            "tool_use" => StopReason::ToolUse,
                            "end_turn" => StopReason::EndTurn,
                            "max_tokens" => StopReason::MaxTokens,
                            "stop_sequence" => StopReason::StopSequence,
                            _ => StopReason::EndTurn,
                        };
                    }
                }
            }

            // 4. Build assistant message content blocks
            let mut assistant_blocks = Vec::new();
            let full_text: String = text_parts.concat();
            if !full_text.is_empty() {
                assistant_blocks.push(ContentBlock::Text { text: full_text });
            }
            for (id, name, input) in &tool_uses {
                assistant_blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }

            // 5. Add assistant message to session
            let assistant_msg = Message::Assistant(AssistantMessage {
                id: Uuid::new_v4(),
                content: assistant_blocks,
                model: self.config.model.clone(),
                usage: turn_usage.clone(),
                stop_reason: Some(stop_reason.clone()),
            });
            self.session.add_message(assistant_msg);

            // 6. Execute tools and add results to session
            if stop_reason == StopReason::ToolUse {
                let tool_ctx = ToolContext {
                    cwd: self.cwd().to_string(),
                    interactive: true,
                };

                for (tool_id, tool_name, tool_input) in &tool_uses {
                    all_events.push(StreamEvent::ToolExecutionStart {
                        tool_use_id: tool_id.clone(),
                        tool_name: tool_name.clone(),
                        description: format!("Executing {tool_name}"),
                    });

                    // Execute the tool
                    let result = self
                        .tools
                        .execute(tool_name, tool_input.clone(), &tool_ctx)
                        .await;

                    let tool_result = match result {
                        Ok(r) => r,
                        Err(e) => cisco_code_protocol::ToolResult::error(format!(
                            "Tool execution failed: {e}"
                        )),
                    };

                    // Emit tool result event
                    all_events.push(StreamEvent::ToolResult {
                        tool_use_id: tool_id.clone(),
                        result: tool_result.output.clone(),
                        is_error: tool_result.is_error,
                    });

                    // Add tool result message to session
                    let result_msg = Message::ToolResult(ToolResultMessage {
                        id: Uuid::new_v4(),
                        tool_use_id: tool_id.clone(),
                        content: tool_result.output,
                        is_error: tool_result.is_error,
                        injected_messages: tool_result.injected_messages,
                    });
                    self.session.add_message(result_msg);
                }
            }

            // 7. Update cumulative usage
            self.total_usage.merge(&turn_usage);

            all_events.push(StreamEvent::TurnEnd {
                stop_reason: stop_reason.clone(),
                usage: turn_usage,
            });

            // Safety: prevent infinite loops
            if self.turn_count >= self.config.max_turns {
                all_events.push(StreamEvent::Error {
                    message: format!("Turn limit reached ({})", self.config.max_turns),
                    recoverable: false,
                });
                break;
            }

            // Budget check
            if let Some(budget) = self.config.max_budget_usd {
                // Rough cost estimate: $3/M input, $15/M output for Claude Sonnet
                let estimated_cost = (self.total_usage.input_tokens as f64 * 3.0
                    + self.total_usage.output_tokens as f64 * 15.0)
                    / 1_000_000.0;
                if estimated_cost > budget {
                    all_events.push(StreamEvent::Error {
                        message: format!(
                            "Budget limit reached (${:.2} > ${:.2})",
                            estimated_cost, budget
                        ),
                        recoverable: false,
                    });
                    break;
                }
            }
        }

        Ok(all_events)
    }

    /// Get cumulative token usage for this session.
    pub fn total_usage(&self) -> &TokenUsage {
        &self.total_usage
    }

    /// Get the number of turns completed.
    pub fn turn_count(&self) -> u32 {
        self.turn_count
    }
}
