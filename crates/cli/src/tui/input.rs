//! Keyboard input handling for the TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, AppMode};

/// Result of processing a key event.
pub enum InputAction {
    /// No action needed.
    None,
    /// Submit the current input line.
    Submit(String),
    /// User quit.
    Quit,
    /// Redraw needed.
    Redraw,
    /// Permission response (approved or denied).
    PermissionResponse { tool_use_id: String, approved: bool },
}

/// Process a key event and return what action to take.
pub fn handle_key(app: &mut App, key: KeyEvent) -> InputAction {
    // Global keys (work in any mode)
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            if app.mode == AppMode::Running {
                // Cancel current run
                app.mode = AppMode::Normal;
                return InputAction::Redraw;
            }
            app.should_quit = true;
            return InputAction::Quit;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            if app.input.is_empty() {
                app.should_quit = true;
                return InputAction::Quit;
            }
        }
        _ => {}
    }

    // Mode-specific handling
    match &app.mode {
        AppMode::PermissionPrompt { tool_use_id, .. } => {
            let tool_use_id = tool_use_id.clone();
            handle_permission_key(app, key, &tool_use_id)
        }
        AppMode::Running => {
            // While running, only scroll keys work
            handle_scroll_key(app, key)
        }
        AppMode::Normal | AppMode::PlanMode => handle_normal_key(app, key),
    }
}

/// Handle keys in normal input mode.
fn handle_normal_key(app: &mut App, key: KeyEvent) -> InputAction {
    match (key.modifiers, key.code) {
        // Submit on Enter
        (_, KeyCode::Enter) => {
            if let Some(input) = app.submit_input() {
                InputAction::Submit(input)
            } else {
                InputAction::None
            }
        }

        // Editing
        (_, KeyCode::Backspace) => {
            app.backspace();
            InputAction::Redraw
        }
        (_, KeyCode::Delete) => {
            app.delete_char();
            InputAction::Redraw
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            // Clear input line
            app.input.clear();
            app.cursor_pos = 0;
            InputAction::Redraw
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            // Delete word backwards
            delete_word_back(app);
            InputAction::Redraw
        }

        // Navigation
        (_, KeyCode::Left) => {
            app.cursor_left();
            InputAction::Redraw
        }
        (_, KeyCode::Right) => {
            app.cursor_right();
            InputAction::Redraw
        }
        (_, KeyCode::Home) | (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            app.cursor_home();
            InputAction::Redraw
        }
        (_, KeyCode::End) | (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            app.cursor_end();
            InputAction::Redraw
        }

        // History
        (_, KeyCode::Up) => {
            app.history_up();
            InputAction::Redraw
        }
        (_, KeyCode::Down) => {
            app.history_down();
            InputAction::Redraw
        }

        // Scroll
        (_, KeyCode::PageUp) => {
            app.scroll_up(10);
            InputAction::Redraw
        }
        (_, KeyCode::PageDown) => {
            app.scroll_down(10);
            InputAction::Redraw
        }

        // Character input
        (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char(c)) => {
            app.insert_char(c);
            InputAction::Redraw
        }

        // Tab completion placeholder
        (_, KeyCode::Tab) => {
            // TODO: slash command completion
            InputAction::None
        }

        _ => InputAction::None,
    }
}

/// Handle keys during permission prompt.
fn handle_permission_key(app: &mut App, key: KeyEvent, tool_use_id: &str) -> InputAction {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            app.mode = AppMode::Running;
            InputAction::PermissionResponse {
                tool_use_id: tool_use_id.to_string(),
                approved: true,
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.mode = AppMode::Running;
            InputAction::PermissionResponse {
                tool_use_id: tool_use_id.to_string(),
                approved: false,
            }
        }
        _ => InputAction::None,
    }
}

/// Handle scroll keys during agent execution.
fn handle_scroll_key(app: &mut App, key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::PageUp => {
            app.scroll_up(10);
            InputAction::Redraw
        }
        KeyCode::PageDown => {
            app.scroll_down(10);
            InputAction::Redraw
        }
        KeyCode::Up => {
            app.scroll_up(1);
            InputAction::Redraw
        }
        KeyCode::Down => {
            app.scroll_down(1);
            InputAction::Redraw
        }
        _ => InputAction::None,
    }
}

/// Delete the word before the cursor.
fn delete_word_back(app: &mut App) {
    if app.cursor_pos == 0 {
        return;
    }
    let before = &app.input[..app.cursor_pos];
    // Skip trailing whitespace
    let trimmed = before.trim_end();
    // Find the last word boundary
    let word_start = trimmed.rfind(|c: char| c.is_whitespace()).map(|i| i + 1).unwrap_or(0);
    app.input = format!("{}{}", &app.input[..word_start], &app.input[app.cursor_pos..]);
    app.cursor_pos = word_start;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn test_char_input() {
        let mut app = App::new("m", "s");
        let action = handle_key(&mut app, key(KeyCode::Char('a')));
        assert!(matches!(action, InputAction::Redraw));
        assert_eq!(app.input, "a");
    }

    #[test]
    fn test_enter_submits() {
        let mut app = App::new("m", "s");
        app.input = "hello".into();
        app.cursor_pos = 5;
        let action = handle_key(&mut app, key(KeyCode::Enter));
        match action {
            InputAction::Submit(text) => assert_eq!(text, "hello"),
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn test_enter_empty_no_submit() {
        let mut app = App::new("m", "s");
        let action = handle_key(&mut app, key(KeyCode::Enter));
        assert!(matches!(action, InputAction::None));
    }

    #[test]
    fn test_ctrl_c_quits() {
        let mut app = App::new("m", "s");
        let action = handle_key(&mut app, ctrl_key('c'));
        assert!(matches!(action, InputAction::Quit));
        assert!(app.should_quit);
    }

    #[test]
    fn test_ctrl_c_cancels_running() {
        let mut app = App::new("m", "s");
        app.mode = AppMode::Running;
        let action = handle_key(&mut app, ctrl_key('c'));
        assert!(matches!(action, InputAction::Redraw));
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_ctrl_d_quits_empty() {
        let mut app = App::new("m", "s");
        let action = handle_key(&mut app, ctrl_key('d'));
        assert!(matches!(action, InputAction::Quit));
    }

    #[test]
    fn test_ctrl_u_clears_line() {
        let mut app = App::new("m", "s");
        app.input = "some text".into();
        app.cursor_pos = 9;
        let _ = handle_key(&mut app, ctrl_key('u'));
        assert!(app.input.is_empty());
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn test_backspace() {
        let mut app = App::new("m", "s");
        app.input = "ab".into();
        app.cursor_pos = 2;
        let _ = handle_key(&mut app, key(KeyCode::Backspace));
        assert_eq!(app.input, "a");
    }

    #[test]
    fn test_arrow_keys() {
        let mut app = App::new("m", "s");
        app.input = "abc".into();
        app.cursor_pos = 3;

        handle_key(&mut app, key(KeyCode::Left));
        assert_eq!(app.cursor_pos, 2);

        handle_key(&mut app, key(KeyCode::Right));
        assert_eq!(app.cursor_pos, 3);

        handle_key(&mut app, key(KeyCode::Home));
        assert_eq!(app.cursor_pos, 0);

        handle_key(&mut app, key(KeyCode::End));
        assert_eq!(app.cursor_pos, 3);
    }

    #[test]
    fn test_history_arrows() {
        let mut app = App::new("m", "s");
        app.history = vec!["cmd1".into(), "cmd2".into()];

        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.input, "cmd2");

        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.input, "cmd1");

        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.input, "cmd2");
    }

    #[test]
    fn test_permission_approve() {
        let mut app = App::new("m", "s");
        app.mode = AppMode::PermissionPrompt {
            tool_use_id: "tu_1".into(),
            tool_name: "Bash".into(),
            input_summary: "rm something".into(),
        };
        let action = handle_key(&mut app, key(KeyCode::Char('y')));
        match action {
            InputAction::PermissionResponse {
                tool_use_id,
                approved,
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert!(approved);
            }
            _ => panic!("expected PermissionResponse"),
        }
    }

    #[test]
    fn test_permission_deny() {
        let mut app = App::new("m", "s");
        app.mode = AppMode::PermissionPrompt {
            tool_use_id: "tu_1".into(),
            tool_name: "Bash".into(),
            input_summary: "danger".into(),
        };
        let action = handle_key(&mut app, key(KeyCode::Char('n')));
        match action {
            InputAction::PermissionResponse {
                approved, ..
            } => assert!(!approved),
            _ => panic!("expected PermissionResponse"),
        }
    }

    #[test]
    fn test_delete_word_back() {
        let mut app = App::new("m", "s");
        app.input = "hello world".into();
        app.cursor_pos = 11;
        delete_word_back(&mut app);
        assert_eq!(app.input, "hello ");
        assert_eq!(app.cursor_pos, 6);
    }

    #[test]
    fn test_scroll_while_running() {
        let mut app = App::new("m", "s");
        app.mode = AppMode::Running;
        let action = handle_key(&mut app, key(KeyCode::PageUp));
        assert!(matches!(action, InputAction::Redraw));
        assert_eq!(app.scroll_offset, 10);
    }
}
