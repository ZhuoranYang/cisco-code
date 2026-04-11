//! Export telemetry data in JSON and CSV formats.

use crate::audit::AuditEntry;
use crate::metrics::SessionMetrics;
use crate::spans::Span;

/// Export spans as a JSON array.
pub fn export_spans_json(spans: &[Span]) -> String {
    serde_json::to_string_pretty(spans).unwrap_or_else(|_| "[]".into())
}

/// Export session metrics as a JSON object.
pub fn export_metrics_json(metrics: &SessionMetrics) -> String {
    serde_json::to_string_pretty(metrics).unwrap_or_else(|_| "{}".into())
}

/// Export audit entries as CSV for compliance tools.
///
/// The first line is a header row. Fields containing commas or double quotes
/// are escaped per RFC 4180.
pub fn export_audit_csv(entries: &[AuditEntry]) -> String {
    let mut csv = String::new();
    csv.push_str("id,timestamp,session_id,event_type,tool_name,input_summary,output_summary,is_error,duration_ms,user\n");

    for entry in entries {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&entry.id),
            csv_escape(&entry.timestamp.to_rfc3339()),
            csv_escape(&entry.session_id),
            csv_escape(&entry.event_type),
            csv_escape(entry.tool_name.as_deref().unwrap_or("")),
            csv_escape(entry.input_summary.as_deref().unwrap_or("")),
            csv_escape(entry.output_summary.as_deref().unwrap_or("")),
            entry.is_error,
            entry.duration_ms.map(|d| d.to_string()).unwrap_or_default(),
            csv_escape(entry.user.as_deref().unwrap_or("")),
        ));
    }

    csv
}

/// Escape a string for CSV (quote if it contains commas, quotes, or newlines).
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditEntry;
    use crate::spans::{SpanKind, SpanStatus};
    use chrono::Utc;
    use std::collections::HashMap;

    #[test]
    fn test_export_spans_json_roundtrip() {
        let spans = vec![
            Span {
                id: "span-1".into(),
                parent_id: None,
                name: "interaction".into(),
                kind: SpanKind::Interaction,
                start_time: Utc::now(),
                end_time: Some(Utc::now()),
                attributes: HashMap::new(),
                status: SpanStatus::Ok,
            },
            Span {
                id: "span-2".into(),
                parent_id: Some("span-1".into()),
                name: "llm_call".into(),
                kind: SpanKind::LlmRequest,
                start_time: Utc::now(),
                end_time: Some(Utc::now()),
                attributes: {
                    let mut m = HashMap::new();
                    m.insert("model".into(), serde_json::json!("claude-sonnet-4-6"));
                    m
                },
                status: SpanStatus::Ok,
            },
        ];

        let json = export_spans_json(&spans);
        let parsed: Vec<Span> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "span-1");
        assert_eq!(parsed[1].parent_id.as_deref(), Some("span-1"));
        assert_eq!(
            parsed[1].attributes["model"],
            serde_json::json!("claude-sonnet-4-6")
        );
    }

    #[test]
    fn test_export_metrics_json_roundtrip() {
        let metrics = SessionMetrics {
            session_id: "sess-1".into(),
            model_name: "claude-sonnet-4-6".into(),
            total_tokens: 1500,
            total_turns: 3,
            total_tool_calls: 5,
            total_errors: 0,
            elapsed_secs: 45.2,
            avg_latency_ms: 250.0,
        };

        let json = export_metrics_json(&metrics);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["total_tokens"], 1500);
        assert_eq!(parsed["session_id"], "sess-1");
        assert_eq!(parsed["model_name"], "claude-sonnet-4-6");

        // Full round-trip
        let deserialized: SessionMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_tokens, 1500);
        assert_eq!(deserialized.session_id, "sess-1");
    }

    #[test]
    fn test_export_audit_csv_format() {
        let entries = vec![AuditEntry {
            id: "e1".into(),
            timestamp: chrono::DateTime::parse_from_rfc3339("2026-01-15T10:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            session_id: "s1".into(),
            event_type: "tool_execution".into(),
            tool_name: Some("Bash".into()),
            input_summary: Some("ls -la".into()),
            output_summary: Some("total 42".into()),
            is_error: false,
            duration_ms: Some(150),
            user: Some("zhuoran".into()),
        }];

        let csv = export_audit_csv(&entries);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 2); // header + 1 row
        assert!(lines[0].starts_with("id,timestamp,session_id"));
        assert!(lines[0].contains("output_summary"));
        assert!(lines[1].contains("e1"));
        assert!(lines[1].contains("s1"));
        assert!(lines[1].contains("Bash"));
        assert!(lines[1].contains("total 42"));
        assert!(lines[1].contains("false"));
        assert!(lines[1].contains("150"));
    }

    #[test]
    fn test_export_audit_csv_escaping() {
        let entries = vec![AuditEntry {
            id: "e2".into(),
            timestamp: Utc::now(),
            session_id: "s1".into(),
            event_type: "tool_execution".into(),
            tool_name: Some("Bash".into()),
            input_summary: Some("echo \"hello, world\"".into()),
            output_summary: None,
            is_error: false,
            duration_ms: None,
            user: None,
        }];

        let csv = export_audit_csv(&entries);
        // The input_summary contains both comma and quotes, so it should be escaped
        assert!(csv.contains("\"echo \"\"hello, world\"\"\""));
    }

    #[test]
    fn test_export_empty_spans() {
        let json = export_spans_json(&[]);
        assert_eq!(json, "[]");
    }

    #[test]
    fn test_csv_escape_fn() {
        assert_eq!(csv_escape("simple"), "simple");
        assert_eq!(csv_escape("has,comma"), "\"has,comma\"");
        assert_eq!(csv_escape("has\"quote"), "\"has\"\"quote\"");
    }
}
