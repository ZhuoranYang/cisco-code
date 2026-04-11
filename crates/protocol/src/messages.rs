//! Message types for the conversation system.
//!
//! Design insight from Claude Code: Messages are a rich union type with variants for
//! user, assistant, system, tool use, and tool result. The key pattern is that tool
//! results can carry side effects (new messages, context modifications).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a message.
pub type MessageId = Uuid;

/// A conversation message — the fundamental unit of the agent loop.
///
/// Claude Code uses 7 message types; Codex uses a simpler model.
/// We follow Claude Code's richer model because it enables tool results
/// to carry side effects (new messages, context modifications).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    #[serde(rename = "user")]
    User(UserMessage),
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    #[serde(rename = "system")]
    System(SystemMessage),
    #[serde(rename = "tool_use")]
    ToolUse(ToolUseMessage),
    #[serde(rename = "tool_result")]
    ToolResult(ToolResultMessage),
    /// Marks where context compaction occurred.
    /// Everything before this boundary was summarized into the summary text.
    /// Matches Claude Code's `SystemCompactBoundaryMessage`.
    #[serde(rename = "compact_boundary")]
    CompactBoundary(CompactBoundaryMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub id: MessageId,
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub id: MessageId,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub usage: TokenUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMessage {
    pub id: MessageId,
    pub content: String,
    pub system_type: SystemMessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemMessageType {
    Informational,
    Error,
    Warning,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseMessage {
    pub id: MessageId,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub id: MessageId,
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
    /// Additional messages injected by the tool (Claude Code pattern).
    /// This enables tools to reshape the conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub injected_messages: Option<Vec<Message>>,
}

/// Marks a context compaction boundary in the conversation.
/// When the context window fills up, earlier messages are summarized and
/// replaced with a compact summary. This message marks where that happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactBoundaryMessage {
    pub id: MessageId,
    /// Summary of the compacted messages.
    pub summary: String,
    /// Number of original messages that were compacted.
    pub compacted_message_count: usize,
    /// Timestamp when compaction occurred.
    pub timestamp: String,
}

/// Content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    /// Extended thinking block — model's internal reasoning (not shown to user by default).
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    pub media_type: String,
    pub data: String, // base64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub content: String,
}

/// Token usage tracking.
/// Design insight from Codex: track cache hits separately for cost optimization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    pub fn merge(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_usage_default() {
        let usage = TokenUsage::default();
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.total(), 0);
    }

    #[test]
    fn test_token_usage_merge() {
        let mut a = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 10,
            cache_read_tokens: 5,
        };
        let b = TokenUsage {
            input_tokens: 200,
            output_tokens: 100,
            cache_creation_tokens: 20,
            cache_read_tokens: 10,
        };
        a.merge(&b);
        assert_eq!(a.input_tokens, 300);
        assert_eq!(a.output_tokens, 150);
        assert_eq!(a.total(), 450);
        assert_eq!(a.cache_creation_tokens, 30);
        assert_eq!(a.cache_read_tokens, 15);
    }

    #[test]
    fn test_tool_result_success() {
        let result = crate::ToolResult::success("output text");
        assert_eq!(result.output, "output text");
        assert!(!result.is_error);
        assert!(result.injected_messages.is_none());
    }

    #[test]
    fn test_tool_result_error() {
        let result = crate::ToolResult::error("something broke");
        assert_eq!(result.output, "something broke");
        assert!(result.is_error);
    }

    #[test]
    fn test_message_serialization_roundtrip() {
        let msg = Message::User(UserMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            attachments: None,
        });

        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();

        match deserialized {
            Message::User(u) => {
                assert_eq!(u.content.len(), 1);
                match &u.content[0] {
                    ContentBlock::Text { text } => assert_eq!(text, "hello"),
                    _ => panic!("wrong content block type"),
                }
            }
            _ => panic!("wrong message type"),
        }
    }

    #[test]
    fn test_stop_reason_equality() {
        assert_eq!(StopReason::EndTurn, StopReason::EndTurn);
        assert_ne!(StopReason::EndTurn, StopReason::ToolUse);
    }

    #[test]
    fn test_content_block_tool_use_serialization() {
        let block = ContentBlock::ToolUse {
            id: "tu_123".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "tool_use");
        assert_eq!(json["name"], "Bash");
        assert_eq!(json["input"]["command"], "ls");
    }

    #[test]
    fn test_thinking_content_block() {
        let block = ContentBlock::Thinking {
            thinking: "Let me analyze the code step by step...".into(),
        };
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "thinking");
        assert!(json["thinking"].as_str().unwrap().contains("step by step"));

        // Roundtrip
        let deserialized: ContentBlock = serde_json::from_value(json).unwrap();
        match deserialized {
            ContentBlock::Thinking { thinking } => {
                assert!(thinking.contains("analyze"));
            }
            _ => panic!("wrong content block type"),
        }
    }

    #[test]
    fn test_assistant_message_with_usage() {
        let msg = AssistantMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: "response".into(),
            }],
            model: "claude-sonnet-4-6".into(),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
            stop_reason: Some(StopReason::EndTurn),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("claude-sonnet-4-6"));
        assert!(json.contains("EndTurn"));
    }

    #[test]
    fn test_compact_boundary_serialization() {
        let msg = Message::CompactBoundary(CompactBoundaryMessage {
            id: Uuid::new_v4(),
            summary: "Compacted 42 messages about login refactor".into(),
            compacted_message_count: 42,
            timestamp: "2026-04-03T12:00:00Z".into(),
        });

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("compact_boundary"));
        assert!(json.contains("42"));

        // Roundtrip
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        match deserialized {
            Message::CompactBoundary(cb) => {
                assert_eq!(cb.compacted_message_count, 42);
                assert!(cb.summary.contains("login refactor"));
            }
            _ => panic!("wrong message type"),
        }
    }
}
