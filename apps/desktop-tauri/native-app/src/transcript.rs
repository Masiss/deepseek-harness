//! Durable session history projected into readable transcript rows.
//!
//! The snapshot and live `session/follow` frames carry the same event log, so
//! one projection renders both. Internal events that never reach a reader are
//! skipped; unknown durable types are omitted rather than dumped as JSON.

use dsh_client_core::follow::SessionWireEvent;
use serde_json::Value;

/// Most rows kept in memory; older rows are dropped first.
pub const MAX_ROWS: usize = 2_000;
/// Total text budget for retained rows.
pub const MAX_BYTES: usize = 256 * 1024;
/// Longest single row text; longer bodies keep a truncated prefix.
pub const MAX_ROW_CHARS: usize = 8_000;

/// What a transcript row represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    User,
    Assistant,
    ToolResult,
    Approval,
    Job,
}

/// One rendered line group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRow {
    pub seq: i64,
    pub kind: RowKind,
    pub title: String,
    pub text: String,
}

/// Bounded transcript built from durable events.
#[derive(Debug, Default)]
pub struct Transcript {
    rows: Vec<TranscriptRow>,
    last_seq: i64,
    bytes: usize,
}

impl Transcript {
    /// Drops every row; used when a new snapshot replaces the projection.
    pub fn reset(&mut self) {
        self.rows.clear();
        self.last_seq = i64::MIN;
        self.bytes = 0;
    }

    /// Applies one durable event, ignoring events already applied.
    pub fn apply(&mut self, event: &SessionWireEvent) {
        if event.seq <= self.last_seq {
            return;
        }
        self.last_seq = event.seq;
        if let Some(row) = project(event) {
            self.bytes += row.text.len();
            self.rows.push(row);
            while self.rows.len() > MAX_ROWS || self.bytes > MAX_BYTES {
                let dropped = self.rows.remove(0);
                self.bytes -= dropped.text.len();
            }
        }
    }

    /// Applies a snapshot's records in order.
    pub fn apply_snapshot(&mut self, records: &[dsh_client_core::follow::EventFrame]) {
        self.reset();
        for record in records {
            self.apply(&record.event);
        }
    }

    /// Retained rows.
    pub fn rows(&self) -> &[TranscriptRow] {
        &self.rows
    }

    /// True before any row was projected.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

fn project(event: &SessionWireEvent) -> Option<TranscriptRow> {
    let row = match event.event_type.as_str() {
        "user/message" => {
            let source = event
                .data
                .get("source")
                .and_then(|source| source.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("user");
            let title = if source == "user-question-reply" {
                "You (answer)"
            } else {
                "You"
            };
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::User,
                title: title.to_owned(),
                text: content_text(event.data.get("content")),
            }
        }
        "assistant/message" => {
            let message = event.data.get("message").unwrap_or(&event.data);
            let mut text = Vec::new();
            let mut tools = Vec::new();
            if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(value) = block.get("text").and_then(Value::as_str) {
                                if !value.trim().is_empty() {
                                    text.push(value.to_owned());
                                }
                            }
                        }
                        Some("tool-call") => {
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("tool");
                            let arguments = block
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("{}");
                            tools.push(format!("{name}: {}", tool_summary(arguments)));
                        }
                        _ => {}
                    }
                }
            }
            if text.is_empty() && tools.is_empty() {
                return None;
            }
            let mut body = text.join("\n");
            if !tools.is_empty() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&tools.join("\n"));
            }
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::Assistant,
                title: "Assistant".to_owned(),
                text: body,
            }
        }
        "tool/result" => {
            let message = event.data.get("message").unwrap_or(&event.data);
            let is_error = message
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::ToolResult,
                title: if is_error {
                    "Tool error".to_owned()
                } else {
                    "Tool result".to_owned()
                },
                text: content_text(message.get("content")),
            }
        }
        "approval/asked" => {
            let tool = event
                .data
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let reason = event
                .data
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("");
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::Approval,
                title: format!("Permission requested: {tool}"),
                text: reason.to_owned(),
            }
        }
        "approval/decided" => {
            let outcome = event
                .data
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::Approval,
                title: format!("Permission {outcome}"),
                text: String::new(),
            }
        }
        "job/registered" => {
            let label = event
                .data
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("job");
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::Job,
                title: "Job started".to_owned(),
                text: label.to_owned(),
            }
        }
        "job/settled" => {
            let status = event
                .data
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let detail = event
                .data
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("");
            TranscriptRow {
                seq: event.seq,
                kind: RowKind::Job,
                title: format!("Job {status}"),
                text: detail.to_owned(),
            }
        }
        _ => return None,
    };
    Some(TranscriptRow {
        text: truncate(&sanitize(&row.text)),
        ..row
    })
}

/// Joins text blocks and names non-text blocks.
fn content_text(content: Option<&Value>) -> String {
    let Some(blocks) = content.and_then(Value::as_array) else {
        return String::new();
    };
    let mut parts = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parts.push(text.to_owned());
                }
            }
            Some("image") | Some("image-ref") => parts.push("[image]".to_owned()),
            Some("tool-call") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                parts.push(format!("[{name}]"));
            }
            Some(other) => parts.push(format!("[{other}]")),
            None => {}
        }
    }
    parts.join("\n")
}

/// One line naming a tool call's key argument.
fn tool_summary(arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) => {
            for key in ["description", "command", "path", "query"] {
                if let Some(text) = value.get(key).and_then(Value::as_str) {
                    let line = text.lines().next().unwrap_or_default().trim();
                    if !line.is_empty() {
                        return line.chars().take(160).collect();
                    }
                }
            }
            let compact = value.to_string();
            compact.chars().take(160).collect()
        }
        Err(_) => arguments.chars().take(160).collect(),
    }
}

/// Removes terminal control characters that would disturb layout.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|character| {
            *character == '\n' || *character == '\t' || !character.is_control()
        })
        .collect()
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_ROW_CHARS {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(MAX_ROW_CHARS).collect();
    kept.push_str("\n… (truncated)");
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: i64, event_type: &str, data: Value) -> SessionWireEvent {
        SessionWireEvent {
            event_type: event_type.into(),
            seq,
            time: 0,
            data,
            ignorable: None,
            source_event_seqs: None,
            surface_op: None,
        }
    }

    #[test]
    fn projects_user_and_assistant_messages() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "user/message",
            serde_json::json!({"source": {"kind": "user"}, "content": [
                {"type": "text", "text": "Hello"},
                {"type": "image", "attachmentId": "a1"}
            ]}),
        ));
        transcript.apply(&event(
            2,
            "assistant/message",
            serde_json::json!({"message": {"content": [
                {"type": "text", "text": "Running it."},
                {"type": "tool-call", "id": "c1", "name": "bash", "arguments": "{\"description\":\"Build the tree\",\"command\":\"cargo build\"}"}
            ]}}),
        ));
        let rows = transcript.rows();
        assert_eq!(rows[0].kind, RowKind::User);
        assert_eq!(rows[0].text, "Hello\n[image]");
        assert_eq!(rows[1].kind, RowKind::Assistant);
        assert_eq!(rows[1].text, "Running it.\nbash: Build the tree");
    }

    #[test]
    fn projects_tool_results_approvals_and_jobs() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "tool/result",
            serde_json::json!({"message": {"isError": true, "content": [{"type": "text", "text": "exit 1"}]}}),
        ));
        transcript.apply(&event(
            2,
            "approval/asked",
            serde_json::json!({"toolName": "bash", "reason": "escalate"}),
        ));
        transcript.apply(&event(
            3,
            "approval/decided",
            serde_json::json!({"outcome": "allowed-once"}),
        ));
        transcript.apply(&event(
            4,
            "job/registered",
            serde_json::json!({"label": "cargo build"}),
        ));
        transcript.apply(&event(
            5,
            "job/settled",
            serde_json::json!({"status": "completed", "detail": "exit code: 0"}),
        ));
        let titles: Vec<_> = transcript.rows().iter().map(|row| row.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Tool error",
                "Permission requested: bash",
                "Permission allowed-once",
                "Job started",
                "Job completed"
            ]
        );
        assert_eq!(transcript.rows()[0].text, "exit 1");
    }

    #[test]
    fn ignores_internal_events_and_duplicate_sequences() {
        let mut transcript = Transcript::default();
        for event_type in [
            "system/message",
            "step/start",
            "request/header",
            "session-log-deepseek/delivery-accepted",
        ] {
            transcript.apply(&event(1, event_type, serde_json::json!({})));
        }
        assert!(transcript.is_empty());
        let message = event(
            2,
            "user/message",
            serde_json::json!({"content": [{"type": "text", "text": "once"}]}),
        );
        transcript.apply(&message);
        transcript.apply(&message);
        assert_eq!(transcript.rows().len(), 1);
    }

    #[test]
    fn bounds_retained_rows_and_row_text() {
        let mut transcript = Transcript::default();
        for seq in 0..(MAX_ROWS as i64 + 5) {
            transcript.apply(&event(
                seq,
                "user/message",
                serde_json::json!({"content": [{"type": "text", "text": "x"}]}),
            ));
        }
        assert_eq!(transcript.rows().len(), MAX_ROWS);
        assert_eq!(transcript.rows().last().unwrap().seq, MAX_ROWS as i64 + 4);

        let long = "a".repeat(MAX_ROW_CHARS + 100);
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "user/message",
            serde_json::json!({"content": [{"type": "text", "text": long}]}),
        ));
        assert!(transcript.rows()[0].text.ends_with("… (truncated)"));
    }

    #[test]
    fn strips_control_characters() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "user/message",
            serde_json::json!({"content": [{"type": "text", "text": "a\u{1b}[31mb\tc"}]}),
        ));
        assert_eq!(transcript.rows()[0].text, "a[31mb\tc");
    }
}
