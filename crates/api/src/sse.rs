//! Server-Sent Events parser.
//!
//! Parses chunked byte streams into SSE frames. Each frame is delimited by
//! a blank line (\n\n or \r\n\r\n). Within a frame, lines starting with
//! "event:" and "data:" are extracted.
//!
//! Pattern adapted from Claw-Code-Parity's api/src/sse.rs.

use anyhow::Result;
use serde::Deserialize;

/// Incremental SSE parser that buffers partial data across chunks.
pub struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Push a chunk of bytes and extract any complete SSE frames.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();

        while let Some(raw_frame) = self.next_frame() {
            if let Some(frame) = Self::parse_frame(&raw_frame)? {
                frames.push(frame);
            }
        }

        Ok(frames)
    }

    /// Extract the next complete frame (delimited by \n\n) from the buffer.
    fn next_frame(&mut self) -> Option<String> {
        let pos = self
            .buffer
            .windows(2)
            .position(|w| w == b"\n\n")
            .map(|p| (p, 2))
            .or_else(|| {
                self.buffer
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| (p, 4))
            })?;

        let (position, sep_len) = pos;
        let frame_bytes: Vec<u8> = self.buffer.drain(..position + sep_len).collect();
        let frame_len = frame_bytes.len().saturating_sub(sep_len);
        Some(String::from_utf8_lossy(&frame_bytes[..frame_len]).into_owned())
    }

    /// Parse a raw SSE frame string into a structured SseFrame.
    fn parse_frame(raw: &str) -> Result<Option<SseFrame>> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let mut event_name: Option<String> = None;
        let mut data_lines: Vec<&str> = Vec::new();

        for line in trimmed.lines() {
            if line.starts_with(':') {
                continue; // SSE comment
            }
            if let Some(name) = line.strip_prefix("event:") {
                event_name = Some(name.trim().to_string());
            } else if let Some(data) = line.strip_prefix("data:") {
                data_lines.push(data.trim_start());
            }
        }

        if matches!(event_name.as_deref(), Some("ping")) {
            return Ok(None);
        }
        if data_lines.is_empty() {
            return Ok(None);
        }

        let payload = data_lines.join("\n");
        if payload == "[DONE]" {
            return Ok(None);
        }

        Ok(Some(SseFrame {
            event: event_name,
            data: payload,
        }))
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

/// A parsed SSE frame with optional event name and JSON data.
#[derive(Debug, Clone)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

/// Anthropic SSE event types from the Messages API.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: serde_json::Value },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: usize,
        content_block: serde_json::Value,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        index: usize,
        delta: serde_json::Value,
    },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: serde_json::Value,
        #[serde(default)]
        usage: serde_json::Value,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: serde_json::Value },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let mut parser = SseParser::new();
        let chunk = b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n";
        let frames = parser.push(chunk).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event.as_deref(), Some("content_block_delta"));
    }

    #[test]
    fn test_partial_chunks() {
        let mut parser = SseParser::new();
        let frames1 = parser
            .push(b"event: message_start\ndata: {\"type\":\"mess")
            .unwrap();
        assert!(frames1.is_empty());
        let frames2 = parser
            .push(b"age_start\",\"message\":{\"id\":\"msg_1\"}}\n\n")
            .unwrap();
        assert_eq!(frames2.len(), 1);
    }

    #[test]
    fn test_ping_filtered() {
        let mut parser = SseParser::new();
        let frames = parser.push(b"event: ping\ndata: {}\n\n").unwrap();
        assert!(frames.is_empty());
    }
}
