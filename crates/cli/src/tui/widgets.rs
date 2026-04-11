//! Custom ratatui widgets for the cisco-code TUI.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use super::app::{ActiveTool, AppMode, DisplayMessage, MessageRole, StatusInfo};

// ---------------------------------------------------------------------------
// Status Bar
// ---------------------------------------------------------------------------

/// Status bar widget showing model, tokens, and mode.
pub struct StatusBar<'a> {
    pub info: &'a StatusInfo,
    pub mode: &'a AppMode,
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mode_str = match self.mode {
            AppMode::Normal => "READY",
            AppMode::Running => "RUNNING",
            AppMode::PlanMode => "PLAN",
            AppMode::PermissionPrompt { .. } => "PERMISSION",
        };

        let mode_color = match self.mode {
            AppMode::Normal => Color::Green,
            AppMode::Running => Color::Yellow,
            AppMode::PlanMode => Color::Cyan,
            AppMode::PermissionPrompt { .. } => Color::Red,
        };

        let left = format!(
            " {} | {} | {}in/{}out ",
            mode_str,
            self.info.model,
            format_tokens(self.info.input_tokens),
            format_tokens(self.info.output_tokens),
        );

        let right = format!(
            "turn {} | {} ",
            self.info.turn_count,
            &self.info.session_id[..8.min(self.info.session_id.len())],
        );

        let style = Style::default().bg(Color::DarkGray).fg(Color::White);

        // Fill background
        for x in area.x..area.x + area.width {
            buf[(x, area.y)].set_style(style);
            buf[(x, area.y)].set_char(' ');
        }

        // Mode badge
        let mode_end = mode_str.len() + 2;
        let mode_badge_style = Style::default()
            .bg(mode_color)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD);
        for (i, c) in format!(" {mode_str} ").chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(mode_badge_style);
            }
        }

        // Left content (after mode badge)
        let left_after_mode = format!(
            "| {} | {}in/{}out ",
            self.info.model,
            format_tokens(self.info.input_tokens),
            format_tokens(self.info.output_tokens),
        );
        for (i, c) in left_after_mode.chars().enumerate() {
            let x = area.x + mode_end as u16 + 1 + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(style);
            }
        }

        // Right-aligned content
        let right_start = area.width.saturating_sub(right.len() as u16);
        for (i, c) in right.chars().enumerate() {
            let x = area.x + right_start + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(style);
            }
        }
    }
}

/// Format token count in human-readable form.
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

// ---------------------------------------------------------------------------
// Input Line
// ---------------------------------------------------------------------------

/// Input line widget with cursor.
pub struct InputLine<'a> {
    pub input: &'a str,
    pub cursor_pos: usize,
    pub mode: &'a AppMode,
}

impl Widget for InputLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let prompt = match self.mode {
            AppMode::Normal => "> ",
            AppMode::Running => "  ",
            AppMode::PlanMode => "plan> ",
            AppMode::PermissionPrompt { .. } => "[y/n] ",
        };

        let prompt_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);

        // Render prompt
        for (i, c) in prompt.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(prompt_style);
            }
        }

        // Render input text
        let prompt_len = prompt.len() as u16;
        let input_style = Style::default().fg(Color::White);
        for (i, c) in self.input.chars().enumerate() {
            let x = area.x + prompt_len + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(input_style);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool Spinner
// ---------------------------------------------------------------------------

/// Spinner widget for active tool execution.
pub struct ToolSpinner<'a> {
    pub tool: &'a ActiveTool,
    pub tick: usize,
}

impl Widget for ToolSpinner<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let frames = ['|', '/', '-', '\\'];
        let spinner = frames[self.tick % frames.len()];
        let elapsed = self.tool.started_at.elapsed().as_secs();

        let line = format!(
            " {} {} — {} ({}s)",
            spinner, self.tool.tool_name, self.tool.description, elapsed
        );

        let style = Style::default().fg(Color::Yellow);
        for (i, c) in line.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.x + area.width {
                buf[(x, area.y)].set_char(c);
                buf[(x, area.y)].set_style(style);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Message formatting helpers
// ---------------------------------------------------------------------------

/// Format a display message into styled lines.
pub fn format_message(msg: &DisplayMessage) -> Vec<Line<'static>> {
    let (prefix, style) = match &msg.role {
        MessageRole::User => (
            "You: ".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        MessageRole::Assistant => (
            "Agent: ".to_string(),
            Style::default().fg(Color::Cyan),
        ),
        MessageRole::System => (
            "System: ".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        MessageRole::Tool { name, is_error } => {
            let color = if *is_error { Color::Red } else { Color::Blue };
            (format!("[{name}]: "), Style::default().fg(color))
        }
    };

    let mut lines = Vec::new();
    let content_lines: Vec<&str> = msg.content.lines().collect();

    if content_lines.is_empty() {
        lines.push(Line::from(Span::styled(prefix, style)));
    } else {
        // First line gets the prefix
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::raw(content_lines[0].to_string()),
        ]));
        // Remaining lines are indented
        for line in &content_lines[1..] {
            lines.push(Line::from(Span::raw(format!("       {line}"))));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Permission dialog
// ---------------------------------------------------------------------------

/// Render a permission prompt overlay.
pub fn permission_prompt_lines(tool_name: &str, input_summary: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Tool: {tool_name}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("  Action: {input_summary}")),
        Line::from(""),
        Line::from(Span::styled(
            "  Allow this operation? (y)es / (n)o",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_format_tokens_small() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn test_format_tokens_k() {
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(15_500), "15.5K");
    }

    #[test]
    fn test_format_tokens_m() {
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn test_format_user_message() {
        let msg = DisplayMessage {
            role: MessageRole::User,
            content: "hello".into(),
            timestamp: Instant::now(),
        };
        let lines = format_message(&msg);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_format_multiline_message() {
        let msg = DisplayMessage {
            role: MessageRole::Assistant,
            content: "line1\nline2\nline3".into(),
            timestamp: Instant::now(),
        };
        let lines = format_message(&msg);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_format_tool_error() {
        let msg = DisplayMessage {
            role: MessageRole::Tool {
                name: "Bash".into(),
                is_error: true,
            },
            content: "command failed".into(),
            timestamp: Instant::now(),
        };
        let lines = format_message(&msg);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_permission_prompt_lines() {
        let lines = permission_prompt_lines("Bash", "rm -rf /tmp");
        assert!(lines.len() >= 4);
    }

    #[test]
    fn test_format_empty_message() {
        let msg = DisplayMessage {
            role: MessageRole::System,
            content: String::new(),
            timestamp: Instant::now(),
        };
        let lines = format_message(&msg);
        assert_eq!(lines.len(), 1);
    }
}
