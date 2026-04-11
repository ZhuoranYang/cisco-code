//! Application state and event loop.

use std::time::{Duration, Instant};

use cisco_code_protocol::StreamEvent;

/// Application mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// Normal input mode.
    Normal,
    /// Agent is running (streaming output).
    Running,
    /// Permission prompt visible.
    PermissionPrompt {
        tool_use_id: String,
        tool_name: String,
        input_summary: String,
    },
    /// Plan mode — reviewing before execution.
    PlanMode,
}

/// A displayable message in the conversation view.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool { name: String, is_error: bool },
}

/// A tool execution in progress.
#[derive(Debug, Clone)]
pub struct ActiveTool {
    pub tool_use_id: String,
    pub tool_name: String,
    pub description: String,
    pub started_at: Instant,
}

/// Status bar information.
#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub model: String,
    pub turn_count: u32,
    pub token_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub session_id: String,
    pub mode: String,
}

impl Default for StatusInfo {
    fn default() -> Self {
        Self {
            model: String::new(),
            turn_count: 0,
            token_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            session_id: String::new(),
            mode: "normal".into(),
        }
    }
}

/// Core application state.
pub struct App {
    /// Current mode.
    pub mode: AppMode,
    /// Conversation messages.
    pub messages: Vec<DisplayMessage>,
    /// Current input text.
    pub input: String,
    /// Cursor position in input.
    pub cursor_pos: usize,
    /// Input history (for up/down arrow).
    pub history: Vec<String>,
    /// Current history index.
    pub history_idx: Option<usize>,
    /// Current streaming text buffer (assistant is typing).
    pub streaming_text: String,
    /// Active tool execution.
    pub active_tool: Option<ActiveTool>,
    /// Status bar info.
    pub status: StatusInfo,
    /// Scroll offset for message view.
    pub scroll_offset: u16,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Error flash message.
    pub flash_message: Option<(String, Instant)>,
}

impl App {
    pub fn new(model: &str, session_id: &str) -> Self {
        Self {
            mode: AppMode::Normal,
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_idx: None,
            streaming_text: String::new(),
            active_tool: None,
            status: StatusInfo {
                model: model.to_string(),
                session_id: session_id.to_string(),
                ..Default::default()
            },
            scroll_offset: 0,
            should_quit: false,
            flash_message: None,
        }
    }

    /// Process a stream event from the agent runtime.
    pub fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::TurnStart {
                model,
                turn_number,
            } => {
                self.mode = AppMode::Running;
                self.status.model = model;
                self.status.turn_count = turn_number;
                self.streaming_text.clear();
            }

            StreamEvent::TextDelta { text } => {
                self.streaming_text.push_str(&text);
            }

            StreamEvent::ToolUseStart {
                tool_use_id,
                tool_name,
            } => {
                // Flush any pending streaming text as an assistant message
                if !self.streaming_text.is_empty() {
                    self.push_message(MessageRole::Assistant, std::mem::take(&mut self.streaming_text));
                }
            }

            StreamEvent::ToolExecutionStart {
                tool_use_id,
                tool_name,
                description,
            } => {
                self.active_tool = Some(ActiveTool {
                    tool_use_id,
                    tool_name,
                    description,
                    started_at: Instant::now(),
                });
            }

            StreamEvent::ToolResult {
                tool_use_id,
                result,
                is_error,
            } => {
                let name = self
                    .active_tool
                    .as_ref()
                    .map(|t| t.tool_name.clone())
                    .unwrap_or_else(|| "tool".into());
                self.active_tool = None;

                let display = if result.len() > 500 {
                    format!("{}...", &result[..500])
                } else {
                    result
                };
                self.push_message(
                    MessageRole::Tool {
                        name,
                        is_error,
                    },
                    display,
                );
            }

            StreamEvent::PermissionRequest {
                tool_use_id,
                tool_name,
                input_summary,
            } => {
                self.mode = AppMode::PermissionPrompt {
                    tool_use_id,
                    tool_name,
                    input_summary,
                };
            }

            StreamEvent::TurnEnd {
                stop_reason,
                usage,
            } => {
                // Flush any remaining streaming text
                if !self.streaming_text.is_empty() {
                    self.push_message(MessageRole::Assistant, std::mem::take(&mut self.streaming_text));
                }

                self.status.input_tokens += usage.input_tokens;
                self.status.output_tokens += usage.output_tokens;
                self.status.token_count = self.status.input_tokens + self.status.output_tokens;

                if stop_reason != cisco_code_protocol::StopReason::ToolUse {
                    self.mode = AppMode::Normal;
                }
            }

            StreamEvent::Error {
                message,
                recoverable,
            } => {
                self.push_message(MessageRole::System, format!("[error] {message}"));
                if !recoverable {
                    self.mode = AppMode::Normal;
                }
                self.flash_message = Some((message, Instant::now()));
            }

            _ => {}
        }
    }

    /// Add a user message to the display.
    pub fn add_user_message(&mut self, content: String) {
        // Save to history
        if !content.is_empty() {
            self.history.push(content.clone());
        }
        self.history_idx = None;
        self.push_message(MessageRole::User, content);
    }

    /// Submit the current input.
    pub fn submit_input(&mut self) -> Option<String> {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return None;
        }
        self.input.clear();
        self.cursor_pos = 0;
        self.add_user_message(input.clone());
        Some(input)
    }

    /// Insert a character at cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    /// Delete character before cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_pos -= prev;
            self.input.remove(self.cursor_pos);
        }
    }

    /// Delete character at cursor.
    pub fn delete_char(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.input.remove(self.cursor_pos);
        }
    }

    /// Move cursor left.
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let prev = self.input[..self.cursor_pos]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_pos -= prev;
        }
    }

    /// Move cursor right.
    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            let next = self.input[self.cursor_pos..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            self.cursor_pos += next;
        }
    }

    /// Move cursor to start of input.
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end of input.
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Navigate history up.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            Some(i) if i > 0 => i - 1,
            Some(_) => return,
            None => self.history.len() - 1,
        };
        self.history_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor_pos = self.input.len();
    }

    /// Navigate history down.
    pub fn history_down(&mut self) {
        match self.history_idx {
            Some(i) if i + 1 < self.history.len() => {
                let idx = i + 1;
                self.history_idx = Some(idx);
                self.input = self.history[idx].clone();
                self.cursor_pos = self.input.len();
            }
            Some(_) => {
                self.history_idx = None;
                self.input.clear();
                self.cursor_pos = 0;
            }
            None => {}
        }
    }

    /// Scroll message view up.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    /// Scroll message view down.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    /// Get the flash message if still valid (within 5 seconds).
    pub fn active_flash(&self) -> Option<&str> {
        self.flash_message.as_ref().and_then(|(msg, time)| {
            if time.elapsed() < Duration::from_secs(5) {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    fn push_message(&mut self, role: MessageRole, content: String) {
        self.messages.push(DisplayMessage {
            role,
            content,
            timestamp: Instant::now(),
        });
        // Auto-scroll to bottom
        self.scroll_offset = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_creation() {
        let app = App::new("claude-sonnet-4-6", "sess-1");
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.status.model, "claude-sonnet-4-6");
        assert_eq!(app.status.session_id, "sess-1");
        assert!(app.messages.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn test_input_editing() {
        let mut app = App::new("m", "s");
        app.insert_char('h');
        app.insert_char('i');
        assert_eq!(app.input, "hi");
        assert_eq!(app.cursor_pos, 2);

        app.backspace();
        assert_eq!(app.input, "h");
        assert_eq!(app.cursor_pos, 1);

        app.cursor_left();
        assert_eq!(app.cursor_pos, 0);
        app.insert_char('a');
        assert_eq!(app.input, "ah");

        app.cursor_end();
        assert_eq!(app.cursor_pos, 2);
        app.cursor_home();
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_submit_input() {
        let mut app = App::new("m", "s");
        app.input = "hello world".into();
        app.cursor_pos = 11;

        let submitted = app.submit_input();
        assert_eq!(submitted, Some("hello world".into()));
        assert!(app.input.is_empty());
        assert_eq!(app.cursor_pos, 0);
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, MessageRole::User);
        assert_eq!(app.history.len(), 1);
    }

    #[test]
    fn test_submit_empty() {
        let mut app = App::new("m", "s");
        assert!(app.submit_input().is_none());
    }

    #[test]
    fn test_history_navigation() {
        let mut app = App::new("m", "s");
        app.history = vec!["first".into(), "second".into(), "third".into()];

        app.history_up();
        assert_eq!(app.input, "third");
        app.history_up();
        assert_eq!(app.input, "second");
        app.history_up();
        assert_eq!(app.input, "first");
        app.history_up(); // should stay at first
        assert_eq!(app.input, "first");

        app.history_down();
        assert_eq!(app.input, "second");
        app.history_down();
        assert_eq!(app.input, "third");
        app.history_down(); // should clear
        assert!(app.input.is_empty());
    }

    #[test]
    fn test_handle_text_delta() {
        let mut app = App::new("m", "s");
        app.handle_stream_event(StreamEvent::TurnStart {
            model: "claude-sonnet-4-6".into(),
            turn_number: 1,
        });
        assert_eq!(app.mode, AppMode::Running);

        app.handle_stream_event(StreamEvent::TextDelta {
            text: "Hello ".into(),
        });
        app.handle_stream_event(StreamEvent::TextDelta {
            text: "world".into(),
        });
        assert_eq!(app.streaming_text, "Hello world");
    }

    #[test]
    fn test_handle_turn_end_flushes_text() {
        let mut app = App::new("m", "s");
        app.streaming_text = "some output".into();
        app.mode = AppMode::Running;

        app.handle_stream_event(StreamEvent::TurnEnd {
            stop_reason: cisco_code_protocol::StopReason::EndTurn,
            usage: cisco_code_protocol::TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
        });

        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.streaming_text.is_empty());
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "some output");
        assert_eq!(app.status.input_tokens, 100);
        assert_eq!(app.status.output_tokens, 50);
    }

    #[test]
    fn test_handle_tool_result() {
        let mut app = App::new("m", "s");
        app.active_tool = Some(ActiveTool {
            tool_use_id: "tu_1".into(),
            tool_name: "Bash".into(),
            description: "running ls".into(),
            started_at: Instant::now(),
        });

        app.handle_stream_event(StreamEvent::ToolResult {
            tool_use_id: "tu_1".into(),
            result: "file.txt".into(),
            is_error: false,
        });

        assert!(app.active_tool.is_none());
        assert_eq!(app.messages.len(), 1);
        match &app.messages[0].role {
            MessageRole::Tool { name, is_error } => {
                assert_eq!(name, "Bash");
                assert!(!is_error);
            }
            _ => panic!("wrong role"),
        }
    }

    #[test]
    fn test_handle_error() {
        let mut app = App::new("m", "s");
        app.handle_stream_event(StreamEvent::Error {
            message: "budget exceeded".into(),
            recoverable: false,
        });
        assert_eq!(app.messages.len(), 1);
        assert!(app.flash_message.is_some());
    }

    #[test]
    fn test_scroll() {
        let mut app = App::new("m", "s");
        app.scroll_up(5);
        assert_eq!(app.scroll_offset, 5);
        app.scroll_down(3);
        assert_eq!(app.scroll_offset, 2);
        app.scroll_down(10); // saturating
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_delete_char() {
        let mut app = App::new("m", "s");
        app.input = "abc".into();
        app.cursor_pos = 1;
        app.delete_char();
        assert_eq!(app.input, "ac");
    }

    #[test]
    fn test_permission_prompt() {
        let mut app = App::new("m", "s");
        app.handle_stream_event(StreamEvent::PermissionRequest {
            tool_use_id: "tu_1".into(),
            tool_name: "Bash".into(),
            input_summary: "rm -rf /".into(),
        });
        match &app.mode {
            AppMode::PermissionPrompt { tool_name, .. } => {
                assert_eq!(tool_name, "Bash");
            }
            _ => panic!("expected permission prompt"),
        }
    }
}
