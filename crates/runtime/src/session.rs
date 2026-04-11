//! Session persistence with JSONL format.
//!
//! Design insight from Codex: JSONL (one JSON object per line) is ideal for
//! session persistence because it supports append-only writes — no need to
//! rewrite the entire file on each turn.
//!
//! Design insight from Claude Code: Sessions are stored as JSONL transcript
//! files, one per conversation, with message-level granularity. A sidecar
//! `.meta.json` file stores session metadata (name, cost, token counts).
//!
//! Enhanced features matching Claude Code:
//! - CompactBoundary messages mark where context compaction occurred
//! - Session metadata with name, cost, token totals, first prompt preview
//! - Session fork: create a new session from an existing one's history

use std::path::{Path, PathBuf};

use anyhow::Result;
use cisco_code_protocol::{ContentBlock, Message, TokenUsage};
use serde::{Deserialize, Serialize};

/// A conversation session.
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub metadata: SessionMetadata,
    /// Path to the JSONL persistence file, if active.
    persist_path: Option<PathBuf>,
}

/// Persistent metadata for a session (stored as sidecar `.meta.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// User-assigned name (or auto-generated from first prompt).
    #[serde(default)]
    pub name: Option<String>,
    /// Total token usage across all turns.
    #[serde(default)]
    pub total_usage: TokenUsage,
    /// Estimated cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Number of turns completed.
    #[serde(default)]
    pub turn_count: u32,
    /// Number of messages in the session.
    #[serde(default)]
    pub message_count: usize,
    /// Preview of the first user prompt (truncated).
    #[serde(default)]
    pub first_prompt: Option<String>,
    /// ID of the parent session if this was forked.
    #[serde(default)]
    pub forked_from: Option<String>,
    /// Number of compactions that have occurred.
    #[serde(default)]
    pub compaction_count: u32,
    /// Session creation timestamp (ISO 8601).
    #[serde(default)]
    pub created_at: Option<String>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
            metadata: SessionMetadata {
                created_at: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
            persist_path: None,
        }
    }

    /// Create a session with JSONL persistence.
    pub fn with_persistence(sessions_dir: &Path) -> Result<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(sessions_dir)?;
        let path = sessions_dir.join(format!("{id}.jsonl"));

        Ok(Self {
            id,
            messages: Vec::new(),
            metadata: SessionMetadata {
                created_at: Some(chrono::Utc::now().to_rfc3339()),
                ..Default::default()
            },
            persist_path: Some(path),
        })
    }

    /// Add a message and persist it if a persist path is set.
    pub fn add_message(&mut self, message: Message) {
        // Update first prompt if this is the first user message
        if self.metadata.first_prompt.is_none() {
            if let Message::User(ref user_msg) = message {
                let text = extract_text_preview(&user_msg.content, 120);
                if !text.is_empty() {
                    self.metadata.first_prompt = Some(text);
                }
            }
        }

        if let Some(ref path) = self.persist_path {
            // Append-only write — each message is one JSONL line
            if let Ok(json) = serde_json::to_string(&message) {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(file, "{json}");
                }
            }
        }

        self.metadata.message_count += 1;
        self.messages.push(message);
    }

    /// Update cumulative usage and persist metadata.
    pub fn update_usage(&mut self, usage: &TokenUsage, cost_usd: f64, turn_count: u32) {
        self.metadata.total_usage = usage.clone();
        self.metadata.cost_usd = cost_usd;
        self.metadata.turn_count = turn_count;
        self.save_metadata();
    }

    /// Record that a compaction occurred.
    pub fn record_compaction(&mut self) {
        self.metadata.compaction_count += 1;
        self.save_metadata();
    }

    /// Set a user-friendly name for this session.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.metadata.name = Some(name.into());
        self.save_metadata();
    }

    /// Get the display name — user-assigned name, first prompt preview, or session ID.
    pub fn display_name(&self) -> String {
        if let Some(ref name) = self.metadata.name {
            name.clone()
        } else if let Some(ref prompt) = self.metadata.first_prompt {
            prompt.clone()
        } else {
            self.id[..8.min(self.id.len())].to_string()
        }
    }

    /// Save session metadata to a sidecar `.meta.json` file.
    fn save_metadata(&self) {
        if let Some(ref path) = self.persist_path {
            let meta_path = path.with_extension("meta.json");
            if let Ok(json) = serde_json::to_string_pretty(&self.metadata) {
                let _ = std::fs::write(meta_path, json);
            }
        }
    }

    /// Load metadata from the sidecar file.
    fn load_metadata(jsonl_path: &Path) -> SessionMetadata {
        let meta_path = jsonl_path.with_extension("meta.json");
        if let Ok(content) = std::fs::read_to_string(&meta_path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            SessionMetadata::default()
        }
    }

    /// Load a session from a JSONL file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut messages = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Message>(line) {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!("Skipping malformed JSONL line: {e}");
                }
            }
        }

        // Extract session ID from filename
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Load sidecar metadata, or reconstruct from messages
        let mut metadata = Self::load_metadata(path);
        metadata.message_count = messages.len();

        // Reconstruct first_prompt if metadata didn't have it
        if metadata.first_prompt.is_none() {
            for msg in &messages {
                if let Message::User(user_msg) = msg {
                    let text = extract_text_preview(&user_msg.content, 120);
                    if !text.is_empty() {
                        metadata.first_prompt = Some(text);
                        break;
                    }
                }
            }
        }

        Ok(Self {
            id,
            messages,
            metadata,
            persist_path: Some(path.to_path_buf()),
        })
    }

    /// Fork this session: create a new session with the same message history.
    /// The new session gets its own ID and persistence path.
    pub fn fork(&self, sessions_dir: &Path) -> Result<Self> {
        let mut forked = Session::with_persistence(sessions_dir)?;
        forked.metadata.forked_from = Some(self.id.clone());
        forked.metadata.first_prompt = self.metadata.first_prompt.clone();

        // Copy messages to new session (with persistence)
        for msg in &self.messages {
            forked.add_message(msg.clone());
        }

        forked.save_metadata();
        Ok(forked)
    }

    /// List available sessions in a directory, sorted by modification time (newest first).
    /// Returns rich session info including first prompt preview.
    pub fn list_sessions(sessions_dir: &Path) -> Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();

        if !sessions_dir.exists() {
            return Ok(sessions);
        }

        for entry in std::fs::read_dir(sessions_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "jsonl") {
                let id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                let file_meta = entry.metadata()?;
                let modified = file_meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);

                // Load session metadata from sidecar
                let metadata = Self::load_metadata(&path);

                // If no sidecar metadata, try to extract first prompt from JSONL
                let first_prompt = metadata.first_prompt.clone().or_else(|| {
                    extract_first_prompt_from_jsonl(&path)
                });

                sessions.push(SessionInfo {
                    id,
                    path,
                    modified,
                    name: metadata.name,
                    first_prompt,
                    message_count: metadata.message_count,
                    turn_count: metadata.turn_count,
                    cost_usd: metadata.cost_usd,
                    forked_from: metadata.forked_from,
                });
            }
        }

        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(sessions)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary info about a stored session.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
    pub name: Option<String>,
    pub first_prompt: Option<String>,
    pub message_count: usize,
    pub turn_count: u32,
    pub cost_usd: f64,
    pub forked_from: Option<String>,
}

impl SessionInfo {
    /// Get the display name — user-assigned name, first prompt preview, or short ID.
    pub fn display_name(&self) -> String {
        if let Some(ref name) = self.name {
            name.clone()
        } else if let Some(ref prompt) = self.first_prompt {
            prompt.clone()
        } else {
            self.id[..8.min(self.id.len())].to_string()
        }
    }
}

/// Extract a text preview from content blocks (truncated to max_len).
fn extract_text_preview(blocks: &[ContentBlock], max_len: usize) -> String {
    for block in blocks {
        if let ContentBlock::Text { text } = block {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                if trimmed.len() > max_len {
                    return format!("{}...", &trimmed[..max_len]);
                }
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

/// Extract the first user prompt from a JSONL file without loading full session.
/// Reads only until the first user message is found (efficient for listing).
fn extract_first_prompt_from_jsonl(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines().take(20) {
        // Only check first 20 lines to avoid reading huge files
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(msg) = serde_json::from_str::<Message>(line) {
            if let Message::User(user_msg) = msg {
                let text = extract_text_preview(&user_msg.content, 120);
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_protocol::{ContentBlock, UserMessage};

    fn make_user_msg(text: &str) -> Message {
        Message::User(UserMessage {
            id: uuid::Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            attachments: None,
        })
    }

    #[test]
    fn test_session_new() {
        let session = Session::new();
        assert!(!session.id.is_empty());
        assert!(session.messages.is_empty());
        assert!(session.metadata.created_at.is_some());
    }

    #[test]
    fn test_session_add_message() {
        let mut session = Session::new();
        session.add_message(make_user_msg("hello"));
        session.add_message(make_user_msg("world"));
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.metadata.message_count, 2);
    }

    #[test]
    fn test_session_first_prompt_extracted() {
        let mut session = Session::new();
        session.add_message(make_user_msg("Fix the login bug"));
        assert_eq!(session.metadata.first_prompt.as_deref(), Some("Fix the login bug"));
    }

    #[test]
    fn test_session_first_prompt_only_first() {
        let mut session = Session::new();
        session.add_message(make_user_msg("First prompt"));
        session.add_message(make_user_msg("Second prompt"));
        // Should still be the first one
        assert_eq!(session.metadata.first_prompt.as_deref(), Some("First prompt"));
    }

    #[test]
    fn test_session_display_name_with_name() {
        let mut session = Session::new();
        session.set_name("Debug auth issue");
        assert_eq!(session.display_name(), "Debug auth issue");
    }

    #[test]
    fn test_session_display_name_from_prompt() {
        let mut session = Session::new();
        session.add_message(make_user_msg("Refactor the config module"));
        assert_eq!(session.display_name(), "Refactor the config module");
    }

    #[test]
    fn test_session_display_name_fallback_to_id() {
        let session = Session::new();
        // No name, no prompt — should show short ID
        assert_eq!(session.display_name().len(), 8);
    }

    #[test]
    fn test_session_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        // Create session and add messages
        let mut session = Session::with_persistence(&sessions_dir).unwrap();
        let id = session.id.clone();
        session.add_message(make_user_msg("first message"));
        session.add_message(make_user_msg("second message"));

        // Load it back
        let jsonl_path = sessions_dir.join(format!("{id}.jsonl"));
        let loaded = Session::load(&jsonl_path).unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.metadata.first_prompt.as_deref(), Some("first message"));
    }

    #[test]
    fn test_session_metadata_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        let mut session = Session::with_persistence(&sessions_dir).unwrap();
        let id = session.id.clone();
        session.add_message(make_user_msg("test prompt"));
        session.set_name("My Test Session");
        session.update_usage(
            &TokenUsage { input_tokens: 1000, output_tokens: 500, ..Default::default() },
            0.05,
            3,
        );

        // Load back and verify metadata
        let jsonl_path = sessions_dir.join(format!("{id}.jsonl"));
        let loaded = Session::load(&jsonl_path).unwrap();
        assert_eq!(loaded.metadata.name.as_deref(), Some("My Test Session"));
        assert_eq!(loaded.metadata.total_usage.input_tokens, 1000);
        assert_eq!(loaded.metadata.turn_count, 3);
    }

    #[test]
    fn test_session_fork() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        let mut original = Session::with_persistence(&sessions_dir).unwrap();
        original.add_message(make_user_msg("original prompt"));
        original.add_message(make_user_msg("follow up"));

        let forked = original.fork(&sessions_dir).unwrap();
        assert_ne!(forked.id, original.id);
        assert_eq!(forked.messages.len(), 2);
        assert_eq!(forked.metadata.forked_from.as_deref(), Some(original.id.as_str()));
        assert_eq!(forked.metadata.first_prompt.as_deref(), Some("original prompt"));
    }

    #[test]
    fn test_session_list() {
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("sessions");

        let mut s1 = Session::with_persistence(&sessions_dir).unwrap();
        s1.add_message(make_user_msg("first session"));

        let mut s2 = Session::with_persistence(&sessions_dir).unwrap();
        s2.add_message(make_user_msg("second session"));

        let list = Session::list_sessions(&sessions_dir).unwrap();
        assert_eq!(list.len(), 2);

        // Sessions should have first_prompt extracted
        for info in &list {
            assert!(info.first_prompt.is_some());
        }
    }

    #[test]
    fn test_session_info_display_name() {
        let info = SessionInfo {
            id: "abc12345-full-uuid".to_string(),
            path: PathBuf::from("/tmp/test.jsonl"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            name: Some("Named Session".to_string()),
            first_prompt: Some("prompt text".to_string()),
            message_count: 5,
            turn_count: 2,
            cost_usd: 0.01,
            forked_from: None,
        };
        assert_eq!(info.display_name(), "Named Session");
    }

    #[test]
    fn test_list_sessions_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let list = Session::list_sessions(&dir.path().join("nonexistent")).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_session_load_malformed_lines_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "not valid json\n{\"bad\": \"structure\"}\n").unwrap();

        let loaded = Session::load(&path).unwrap();
        assert!(loaded.messages.is_empty());
    }

    #[test]
    fn test_extract_text_preview_truncation() {
        let blocks = vec![ContentBlock::Text {
            text: "a".repeat(200),
        }];
        let preview = extract_text_preview(&blocks, 50);
        assert_eq!(preview.len(), 53); // 50 chars + "..."
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn test_extract_text_preview_empty() {
        let blocks = vec![ContentBlock::Text { text: "  ".to_string() }];
        let preview = extract_text_preview(&blocks, 50);
        assert!(preview.is_empty());
    }
}
