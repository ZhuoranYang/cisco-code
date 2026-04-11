//! Span-based tracing for agent operations.
//!
//! Hierarchy: interaction → llm_request → tool_execution
//! Each span tracks timing, attributes, and parent-child relationships.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kind of operation a span represents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// Top-level user interaction (submit_message).
    Interaction,
    /// LLM API request.
    LlmRequest,
    /// Tool execution.
    ToolExecution,
    /// Context compaction.
    Compaction,
    /// Hook execution.
    Hook,
}

/// Span completion status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Ok,
    Error(String),
}

/// A single trace span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub kind: SpanKind,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub attributes: HashMap<String, serde_json::Value>,
    pub status: SpanStatus,
}

impl Span {
    /// Duration in milliseconds (if ended).
    pub fn duration_ms(&self) -> Option<i64> {
        self.end_time.map(|end| (end - self.start_time).num_milliseconds())
    }
}

/// Collects spans for a session.
pub struct SpanCollector {
    active: HashMap<String, Span>,
    completed: Vec<Span>,
}

impl SpanCollector {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            completed: Vec::new(),
        }
    }

    /// Start a new span. Returns the span ID.
    pub fn start_span(&mut self, name: &str, kind: SpanKind, parent_id: Option<String>) -> String {
        let id = Uuid::new_v4().to_string();
        let span = Span {
            id: id.clone(),
            parent_id,
            name: name.to_string(),
            kind,
            start_time: Utc::now(),
            end_time: None,
            attributes: HashMap::new(),
            status: SpanStatus::Ok,
        };
        self.active.insert(id.clone(), span);
        id
    }

    /// End an active span with a status.
    pub fn end_span(&mut self, id: &str, status: SpanStatus) {
        if let Some(mut span) = self.active.remove(id) {
            span.end_time = Some(Utc::now());
            span.status = status;
            self.completed.push(span);
        }
    }

    /// Add an attribute to an active span.
    pub fn add_attribute(&mut self, id: &str, key: &str, value: serde_json::Value) {
        if let Some(span) = self.active.get_mut(id) {
            span.attributes.insert(key.to_string(), value);
        }
    }

    /// Get a reference to a span by ID (checks active first, then completed).
    pub fn get_span(&self, id: &str) -> Option<&Span> {
        self.active
            .get(id)
            .or_else(|| self.completed.iter().find(|s| s.id == id))
    }

    /// Get all completed spans.
    pub fn completed_spans(&self) -> &[Span] {
        &self.completed
    }

    /// Number of active (unfinished) spans.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Take all completed spans out of the collector.
    pub fn drain_completed(&mut self) -> Vec<Span> {
        std::mem::take(&mut self.completed)
    }

    /// Total spans (active + completed).
    pub fn total_count(&self) -> usize {
        self.active.len() + self.completed.len()
    }
}

impl Default for SpanCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_and_end_span() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("test-op", SpanKind::Interaction, None);
        assert_eq!(collector.active_count(), 1);
        assert_eq!(collector.completed_spans().len(), 0);

        collector.end_span(&id, SpanStatus::Ok);
        assert_eq!(collector.active_count(), 0);
        assert_eq!(collector.completed_spans().len(), 1);
        assert_eq!(collector.completed_spans()[0].name, "test-op");
        assert_eq!(collector.completed_spans()[0].status, SpanStatus::Ok);
    }

    #[test]
    fn test_parent_child() {
        let mut collector = SpanCollector::new();
        let parent = collector.start_span("interaction", SpanKind::Interaction, None);
        let child = collector.start_span("llm_request", SpanKind::LlmRequest, Some(parent.clone()));

        let child_span = collector.get_span(&child).unwrap();
        assert_eq!(child_span.parent_id.as_deref(), Some(parent.as_str()));
    }

    #[test]
    fn test_add_attribute() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("op", SpanKind::ToolExecution, None);
        collector.add_attribute(&id, "tool_name", serde_json::json!("Bash"));
        collector.add_attribute(&id, "duration_ms", serde_json::json!(150));

        let span = collector.get_span(&id).unwrap();
        assert_eq!(span.attributes["tool_name"], "Bash");
        assert_eq!(span.attributes["duration_ms"], 150);
    }

    #[test]
    fn test_error_status() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("failing-op", SpanKind::LlmRequest, None);
        collector.end_span(&id, SpanStatus::Error("timeout".into()));

        let span = &collector.completed_spans()[0];
        assert_eq!(span.status, SpanStatus::Error("timeout".into()));
    }

    #[test]
    fn test_drain_completed() {
        let mut collector = SpanCollector::new();
        let id1 = collector.start_span("a", SpanKind::Interaction, None);
        let id2 = collector.start_span("b", SpanKind::LlmRequest, None);
        collector.end_span(&id1, SpanStatus::Ok);
        collector.end_span(&id2, SpanStatus::Ok);

        let drained = collector.drain_completed();
        assert_eq!(drained.len(), 2);
        assert!(collector.completed_spans().is_empty());
    }

    #[test]
    fn test_span_duration() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("timed", SpanKind::ToolExecution, None);
        // End immediately
        collector.end_span(&id, SpanStatus::Ok);
        let span = &collector.completed_spans()[0];
        assert!(span.duration_ms().is_some());
        assert!(span.duration_ms().unwrap() >= 0);
    }

    #[test]
    fn test_span_kinds() {
        let mut collector = SpanCollector::new();
        let _ = collector.start_span("a", SpanKind::Interaction, None);
        let _ = collector.start_span("b", SpanKind::LlmRequest, None);
        let _ = collector.start_span("c", SpanKind::ToolExecution, None);
        let _ = collector.start_span("d", SpanKind::Compaction, None);
        let _ = collector.start_span("e", SpanKind::Hook, None);
        assert_eq!(collector.active_count(), 5);
    }

    #[test]
    fn test_end_nonexistent_span() {
        let mut collector = SpanCollector::new();
        collector.end_span("nonexistent", SpanStatus::Ok);
        assert_eq!(collector.completed_spans().len(), 0);
    }

    #[test]
    fn test_get_span_finds_completed() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("done", SpanKind::Compaction, None);
        collector.end_span(&id, SpanStatus::Ok);
        // Should still find it among completed spans
        let span = collector.get_span(&id).unwrap();
        assert_eq!(span.name, "done");
        assert!(span.end_time.is_some());
    }

    #[test]
    fn test_get_span_not_found() {
        let collector = SpanCollector::new();
        assert!(collector.get_span("nope").is_none());
    }

    #[test]
    fn test_total_count() {
        let mut collector = SpanCollector::new();
        let id = collector.start_span("a", SpanKind::Interaction, None);
        let _ = collector.start_span("b", SpanKind::LlmRequest, None);
        collector.end_span(&id, SpanStatus::Ok);
        assert_eq!(collector.total_count(), 2);
    }

    #[test]
    fn test_span_serialization() {
        let span = Span {
            id: "test-id".into(),
            parent_id: None,
            name: "test".into(),
            kind: SpanKind::Interaction,
            start_time: Utc::now(),
            end_time: Some(Utc::now()),
            attributes: HashMap::new(),
            status: SpanStatus::Ok,
        };
        let json = serde_json::to_string(&span).unwrap();
        let deserialized: Span = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, "test-id");
        assert_eq!(deserialized.kind, SpanKind::Interaction);
    }
}
