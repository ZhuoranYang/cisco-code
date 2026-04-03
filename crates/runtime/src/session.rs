//! Session persistence with JSONL format.
//!
//! Design insight from Codex: JSONL (one JSON object per line) is ideal for
//! session persistence because it supports append-only writes — no need to
//! rewrite the entire file on each turn.
//!
//! Design insight from Claude Code: Sessions are stored as JSONL transcript
//! files, one per conversation, with message-level granularity.

use std::path::{Path, PathBuf};

use anyhow::Result;
use cisco_code_protocol::Message;

/// A conversation session.
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    /// Path to the JSONL persistence file, if active.
    persist_path: Option<PathBuf>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
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
            persist_path: Some(path),
        })
    }

    /// Add a message and persist it if a persist path is set.
    pub fn add_message(&mut self, message: Message) {
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
        self.messages.push(message);
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

        Ok(Self {
            id,
            messages,
            persist_path: Some(path.to_path_buf()),
        })
    }

    /// List available sessions in a directory, sorted by modification time (newest first).
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
                let metadata = entry.metadata()?;
                let modified = metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                sessions.push(SessionInfo {
                    id,
                    path,
                    modified,
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
}
