//! The core conversation runtime — the heart of cisco-code.
//!
//! Design insight from Claw-Code-Parity: `ConversationRuntime<P, T>` is generic
//! over Provider and ToolExecutor traits, making it testable and composable.
//!
//! Design insight from Claude Code: The turn loop yields StreamEvents (generator
//! pattern) for real-time TUI rendering, rather than collecting all output.

use anyhow::Result;
use cisco_code_api::Provider;
use cisco_code_protocol::{Message, StreamEvent, StopReason};
use cisco_code_tools::{ToolRegistry, ToolContext};

use crate::config::RuntimeConfig;
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
}

impl<P: Provider> ConversationRuntime<P> {
    pub fn new(
        provider: P,
        tools: ToolRegistry,
        config: RuntimeConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            session: Session::new(),
            config,
            turn_count: 0,
        }
    }

    /// Submit a user message and execute the agent loop.
    ///
    /// Returns a stream of events for real-time rendering.
    /// The loop continues until the model stops using tools (end_turn)
    /// or a budget/turn limit is reached.
    ///
    /// Design pattern from Claude Code's QueryEngine:
    /// 1. Add user message to history
    /// 2. Build prompt (system + context + history + tools)
    /// 3. Call LLM with streaming
    /// 4. Parse tool_use blocks → execute tools → collect results
    /// 5. Send results back to LLM
    /// 6. Loop until stop_reason = EndTurn
    pub async fn submit_message(
        &mut self,
        user_input: &str,
    ) -> Result<Vec<StreamEvent>> {
        // Add user message to session
        let user_msg = Message::User(cisco_code_protocol::UserMessage {
            id: uuid::Uuid::new_v4(),
            content: vec![cisco_code_protocol::ContentBlock::Text {
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

            // TODO: Phase 1 implementation
            // 1. Build CompletionRequest from session + config
            // 2. Call provider.stream(request)
            // 3. Parse streaming events
            // 4. For each tool_use: check permissions → execute → collect result
            // 5. Update session with assistant message + tool results
            // 6. Check stop_reason

            // Placeholder: end turn immediately
            stop_reason = StopReason::EndTurn;

            all_events.push(StreamEvent::TurnEnd {
                stop_reason: stop_reason.clone(),
                usage: cisco_code_protocol::TokenUsage::default(),
            });

            // Safety: prevent infinite loops
            if self.turn_count >= self.config.max_turns {
                break;
            }
        }

        Ok(all_events)
    }
}
