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
    Reasoning,
    ToolResult,
    Approval,
    Job,
    Todo,
    Deliverable,
    Goal,
    Notice,
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
        for row in project(event) {
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

fn project(event: &SessionWireEvent) -> Vec<TranscriptRow> {
    let rows = match event.event_type.as_str() {
        "todo/write" => vec![todo_row(event)],
        "deliverables/presented" => vec![deliverables_row(event)],
        "goal/change" => vec![goal_row(event)],
        "plan/mode" => vec![TranscriptRow {
            seq: event.seq,
            kind: RowKind::Notice,
            title: if event.data.get("active").and_then(Value::as_bool) == Some(true) {
                "Plan mode on".to_owned()
            } else {
                "Plan mode off".to_owned()
            },
            text: String::new(),
        }],
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
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::User,
                title: title.to_owned(),
                text: content_text(event.data.get("content")),
            }]
        }
        "assistant/message" => {
            let message = event.data.get("message").unwrap_or(&event.data);
            let mut text = Vec::new();
            let mut tools = Vec::new();
            let mut reasoning = Vec::new();
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
                        Some("reasoning") => {
                            if let Some(value) = block.get("text").and_then(Value::as_str) {
                                if !value.trim().is_empty() {
                                    reasoning.push(value.to_owned());
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
                            tools.push(format!("{name}: {}", tool_summary(name, arguments)));
                        }
                        _ => {}
                    }
                }
            }
            let mut rows = Vec::new();
            if !reasoning.is_empty() {
                rows.push(TranscriptRow {
                    seq: event.seq,
                    kind: RowKind::Reasoning,
                    title: "Reasoning".to_owned(),
                    text: reasoning.join("\n"),
                });
            }
            if !text.is_empty() || !tools.is_empty() {
                let mut body = text.join("\n");
                if !tools.is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    body.push_str(&tools.join("\n"));
                }
                rows.push(TranscriptRow {
                    seq: event.seq,
                    kind: RowKind::Assistant,
                    title: "Assistant".to_owned(),
                    text: body,
                });
            }
            rows
        }
        "tool/result" => {
            let message = event.data.get("message").unwrap_or(&event.data);
            let is_error = message
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::ToolResult,
                title: if is_error {
                    "Tool error".to_owned()
                } else {
                    "Tool result".to_owned()
                },
                text: content_text(message.get("content")),
            }]
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
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::Approval,
                title: format!("Permission requested: {tool}"),
                text: reason.to_owned(),
            }]
        }
        "approval/decided" => {
            let outcome = event
                .data
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::Approval,
                title: format!("Permission {outcome}"),
                text: String::new(),
            }]
        }
        "job/registered" => {
            let label = event
                .data
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("job");
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::Job,
                title: "Job started".to_owned(),
                text: label.to_owned(),
            }]
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
            vec![TranscriptRow {
                seq: event.seq,
                kind: RowKind::Job,
                title: format!("Job {status}"),
                text: detail.to_owned(),
            }]
        }
        _ => return Vec::new(),
    };
    rows.into_iter()
        .map(|row| TranscriptRow {
            text: truncate(&sanitize(&row.text)),
            ..row
        })
        .collect()
}

fn todo_row(event: &SessionWireEvent) -> TranscriptRow {
    let todos = event
        .data
        .get("todos")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let text = todos
        .iter()
        .map(|todo| {
            let status = todo.get("status").and_then(Value::as_str).unwrap_or("pending");
            let mark = match status {
                "completed" => "[x]",
                "in_progress" => "[~]",
                _ => "[ ]",
            };
            let content = todo
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("{mark} {content}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    TranscriptRow {
        seq: event.seq,
        kind: RowKind::Todo,
        title: "Todo list".to_owned(),
        text,
    }
}

fn deliverables_row(event: &SessionWireEvent) -> TranscriptRow {
    let files = event
        .data
        .get("files")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let text = files
        .iter()
        .map(|file| {
            let path = file.get("path").and_then(Value::as_str).unwrap_or_default();
            match file.get("description").and_then(Value::as_str) {
                Some(description) if !description.is_empty() => format!("{path} — {description}"),
                _ => path.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    TranscriptRow {
        seq: event.seq,
        kind: RowKind::Deliverable,
        title: "Deliverables".to_owned(),
        text,
    }
}

fn goal_row(event: &SessionWireEvent) -> TranscriptRow {
    if event.data.get("operation").and_then(Value::as_str) == Some("clear") {
        return TranscriptRow {
            seq: event.seq,
            kind: RowKind::Goal,
            title: "Goal cleared".to_owned(),
            text: String::new(),
        };
    }
    let goal = event.data.get("goal");
    let objective = goal
        .and_then(|goal| goal.get("objective"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let phase = goal
        .and_then(|goal| goal.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    TranscriptRow {
        seq: event.seq,
        kind: RowKind::Goal,
        title: format!("Goal · {phase}"),
        text: objective.to_owned(),
    }
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
fn tool_summary(name: &str, arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) => {
            if name == "exit_plan_mode" {
                if let Some(plan) = value.get("plan").and_then(Value::as_str) {
                    let heading = plan
                        .lines()
                        .find_map(|line| line.trim_start().strip_prefix('#'))
                        .map(|rest| rest.trim_start_matches('#').trim())
                        .filter(|heading| !heading.is_empty());
                    return heading.unwrap_or("plan").chars().take(160).collect();
                }
            }
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
    fn projects_reasoning_todos_deliverables_goals_and_plan_mode() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "assistant/message",
            serde_json::json!({"message": {"content": [
                {"type": "reasoning", "text": "weigh options"},
                {"type": "text", "text": "Here."},
                {"type": "tool-call", "name": "exit_plan_mode", "arguments": "{\"plan\":\"# Ship it\\n\\nStep one\"}"}
            ]}}),
        ));
        transcript.apply(&event(
            2,
            "todo/write",
            serde_json::json!({"todos": [
                {"content": "read", "status": "completed"},
                {"content": "edit", "status": "in_progress"},
                {"content": "test", "status": "pending"}
            ]}),
        ));
        transcript.apply(&event(
            3,
            "deliverables/presented",
            serde_json::json!({"files": [
                {"path": "out/report.pdf", "description": "Final report"},
                {"path": "out/data.csv"}
            ]}),
        ));
        transcript.apply(&event(
            4,
            "goal/change",
            serde_json::json!({"operation": "create", "goal": {"objective": "Ship the lane", "phase": "active"}}),
        ));
        transcript.apply(&event(
            5,
            "plan/mode",
            serde_json::json!({"active": true}),
        ));
        transcript.apply(&event(
            6,
            "goal/change",
            serde_json::json!({"operation": "clear"}),
        ));
        let rows = transcript.rows();
        assert_eq!(rows[0].kind, RowKind::Reasoning);
        assert_eq!(rows[0].text, "weigh options");
        assert_eq!(rows[1].text, "Here.\nexit_plan_mode: Ship it");
        assert_eq!(rows[2].title, "Todo list");
        assert_eq!(rows[2].text, "[x] read\n[~] edit\n[ ] test");
        assert_eq!(rows[3].title, "Deliverables");
        assert_eq!(rows[3].text, "out/report.pdf — Final report\nout/data.csv");
        assert_eq!(rows[4].title, "Goal · active");
        assert_eq!(rows[4].text, "Ship the lane");
        assert_eq!(rows[5].title, "Plan mode on");
        assert_eq!(rows[6].title, "Goal cleared");
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
