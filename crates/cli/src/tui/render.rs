//! TUI rendering — draws the App state using ratatui.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, AppMode};
use super::widgets::{self, InputLine, StatusBar, ToolSpinner};

/// Render the complete TUI frame.
pub fn render(frame: &mut Frame, app: &App, tick: usize) {
    let size = frame.area();

    // Layout: [messages | tool_spinner? | input | status_bar]
    let has_spinner = app.active_tool.is_some();
    let has_permission = matches!(app.mode, AppMode::PermissionPrompt { .. });

    let constraints = if has_permission {
        vec![
            Constraint::Min(5),         // messages
            Constraint::Length(8),       // permission dialog
            Constraint::Length(1),       // input
            Constraint::Length(1),       // status bar
        ]
    } else if has_spinner {
        vec![
            Constraint::Min(5),         // messages
            Constraint::Length(1),       // spinner
            Constraint::Length(1),       // input
            Constraint::Length(1),       // status bar
        ]
    } else {
        vec![
            Constraint::Min(5),         // messages
            Constraint::Length(1),       // input
            Constraint::Length(1),       // status bar
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(size);

    // --- Messages area ---
    render_messages(frame, app, chunks[0]);

    // --- Middle section (spinner or permission) ---
    let (input_area, status_area) = if has_permission {
        render_permission_dialog(frame, app, chunks[1]);
        (chunks[2], chunks[3])
    } else if has_spinner {
        if let Some(tool) = &app.active_tool {
            let spinner = ToolSpinner { tool, tick };
            frame.render_widget(spinner, chunks[1]);
        }
        (chunks[2], chunks[3])
    } else {
        (chunks[1], chunks[2])
    };

    // --- Input line ---
    let input_widget = InputLine {
        input: &app.input,
        cursor_pos: app.cursor_pos,
        mode: &app.mode,
    };
    frame.render_widget(input_widget, input_area);

    // Set cursor position
    if app.mode == AppMode::Normal || app.mode == AppMode::PlanMode {
        let prompt_len = match app.mode {
            AppMode::PlanMode => 6,
            _ => 2,
        };
        let cursor_x = input_area.x + prompt_len + app.cursor_pos as u16;
        let cursor_y = input_area.y;
        if cursor_x < input_area.x + input_area.width {
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    // --- Status bar ---
    let status = StatusBar {
        info: &app.status,
        mode: &app.mode,
    };
    frame.render_widget(status, status_area);

    // --- Flash message overlay ---
    if let Some(flash) = app.active_flash() {
        let flash_area = Rect {
            x: size.x + 1,
            y: size.y,
            width: size.width.saturating_sub(2).min(flash.len() as u16 + 4),
            height: 1,
        };
        let flash_widget = Paragraph::new(format!(" {flash} "))
            .style(Style::default().bg(Color::Red).fg(Color::White));
        frame.render_widget(flash_widget, flash_area);
    }
}

/// Render the messages area.
fn render_messages(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::NONE)
        .title(format!(" cisco-code v{} ", env!("CARGO_PKG_VERSION")))
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    let inner = block.inner(area);

    // Build all lines from messages
    let mut all_lines: Vec<Line<'static>> = Vec::new();

    for msg in &app.messages {
        let lines = widgets::format_message(msg);
        all_lines.extend(lines);
        all_lines.push(Line::from("")); // spacing
    }

    // Add streaming text if any
    if !app.streaming_text.is_empty() {
        all_lines.push(Line::from(vec![
            Span::styled(
                "Agent: ",
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(app.streaming_text.clone()),
        ]));

        // Blinking cursor
        all_lines.push(Line::from(Span::styled(
            "       _",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::SLOW_BLINK),
        )));
    }

    // Empty state
    if all_lines.is_empty() {
        all_lines.push(Line::from(Span::styled(
            "  Type a message to start, or /help for commands",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(all_lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.scroll_offset, 0));

    frame.render_widget(paragraph, area);
}

/// Render the permission dialog.
fn render_permission_dialog(frame: &mut Frame, app: &App, area: Rect) {
    if let AppMode::PermissionPrompt {
        tool_name,
        input_summary,
        ..
    } = &app.mode
    {
        let lines = widgets::permission_prompt_lines(tool_name, input_summary);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Permission Required ")
            .title_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, area);
    }
}

/// Calculate terminal layout info for testing.
pub fn calculate_layout(width: u16, height: u16) -> LayoutInfo {
    let has_room_for_spinner = height >= 10;
    let message_height = if has_room_for_spinner {
        height.saturating_sub(3)
    } else {
        height.saturating_sub(2)
    };

    LayoutInfo {
        message_height,
        input_y: height.saturating_sub(2),
        status_y: height.saturating_sub(1),
        has_room_for_spinner,
    }
}

/// Layout measurements for testing.
#[derive(Debug)]
pub struct LayoutInfo {
    pub message_height: u16,
    pub input_y: u16,
    pub status_y: u16,
    pub has_room_for_spinner: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_standard_terminal() {
        let info = calculate_layout(120, 40);
        assert!(info.message_height > 30);
        assert_eq!(info.status_y, 39);
        assert_eq!(info.input_y, 38);
        assert!(info.has_room_for_spinner);
    }

    #[test]
    fn test_layout_small_terminal() {
        let info = calculate_layout(80, 8);
        assert!(info.message_height >= 5);
        assert!(!info.has_room_for_spinner);
    }

    #[test]
    fn test_layout_minimum() {
        let info = calculate_layout(40, 3);
        assert_eq!(info.status_y, 2);
        assert_eq!(info.input_y, 1);
    }
}
