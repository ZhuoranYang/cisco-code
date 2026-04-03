//! Stream events for real-time output.
//!
//! Design insight: Both Claude Code and Codex use generator/stream-based architectures
//! where events are yielded, not collected. This enables real-time TUI rendering
//! and efficient memory usage for long sessions.

use serde::{Deserialize, Serialize};

use crate::messages::{TokenUsage, StopReason};

/// Events emitted during a conversation turn.
/// The TUI/CLI consumes these to render output in real-time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// Turn has started, model is being called
    TurnStart {
        model: String,
        turn_number: u32,
    },

    /// Text delta from the model (for streaming display)
    TextDelta {
        text: String,
    },

    /// Model is requesting a tool call
    ToolUseStart {
        tool_use_id: String,
        tool_name: String,
    },

    /// Tool input being streamed (for large inputs)
    ToolInputDelta {
        tool_use_id: String,
        delta: String,
    },

    /// Tool execution has started
    ToolExecutionStart {
        tool_use_id: String,
        tool_name: String,
        description: String,
    },

    /// Tool execution progress update
    ToolProgress {
        tool_use_id: String,
        progress: ToolProgressData,
    },

    /// Tool execution completed
    ToolResult {
        tool_use_id: String,
        result: String,
        is_error: bool,
    },

    /// Permission requested from user
    PermissionRequest {
        tool_use_id: String,
        tool_name: String,
        input_summary: String,
    },

    /// Turn completed
    TurnEnd {
        stop_reason: StopReason,
        usage: TokenUsage,
    },

    /// Error occurred
    Error {
        message: String,
        recoverable: bool,
    },
}

/// Tool-specific progress data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ToolProgressData {
    /// Command execution progress (bash tool)
    Command {
        stdout_line: Option<String>,
        stderr_line: Option<String>,
    },
    /// File operation progress
    FileOp {
        path: String,
        operation: String,
    },
    /// Search progress
    Search {
        files_searched: u64,
        matches_found: u64,
    },
    /// Generic progress
    Generic {
        message: String,
        percentage: Option<f64>,
    },
}
