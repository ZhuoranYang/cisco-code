//! Session persistence with JSONL format.
//!
//! Design insight from Codex: JSONL (one JSON object per line) is ideal for
//! session persistence because it supports append-only writes — no need to
//! rewrite the entire file on each turn.

use cisco_code_protocol::Message;

/// A conversation session.
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
        }
    }

    pub fn add_message(&mut self, message: Message) {
        self.messages.push(message);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
