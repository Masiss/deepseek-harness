//! Durable session history projected into readable transcript rows.
//!
//! The snapshot and live `session/follow` frames carry the same event log, so
//! one projection renders both. Internal events that never reach a reader are
//! skipped; unknown durable types are omitted rather than dumped as JSON.

use dsh_client_core::follow::SessionWireEvent;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

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
    /// A tool result the runtime marked as failed.
    ToolError,
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
    pub message_id: Option<String>,
}

/// Bounded transcript built from durable events.
///
/// History pages and live follow frames project through the same function, so a
/// Session renders one ordered, deduplicated log regardless of where each event
/// came from.
#[derive(Debug)]
pub struct Transcript {
    rows: Vec<TranscriptRow>,
    applied: BTreeSet<i64>,
    bytes: usize,
    has_cjk: bool,
    /// Whether the newest `turn/start` has no matching `turn/end` yet.
    open_turn: bool,
    /// `reason.kind` of the newest `turn/end`; unknown kinds stay verbatim.
    last_turn_end: Option<String>,
    /// Sequence of the newest turn event that set the two fields above.
    state_seq: i64,
    /// Highest event sequence projected; `-1` when nothing was projected.
    last_seq: i64,
    /// Folded plan-mode state; the last `plan/mode` event wins.
    plan_active: bool,
    /// Sequence of the newest `plan/mode` event applied.
    plan_seq: i64,
    /// `tool/call` arrival times by call id, for result durations.
    call_times: HashMap<String, (i64, i64)>,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            applied: BTreeSet::new(),
            bytes: 0,
            has_cjk: false,
            open_turn: false,
            last_turn_end: None,
            state_seq: 0,
            last_seq: -1,
            plan_active: false,
            plan_seq: -1,
            call_times: HashMap::new(),
        }
    }
}

impl Transcript {
    /// Drops every row; used when the viewer switches sessions.
    pub fn reset(&mut self) {
        self.rows.clear();
        self.applied.clear();
        self.bytes = 0;
        self.has_cjk = false;
        self.open_turn = false;
        self.last_turn_end = None;
        self.state_seq = 0;
        self.last_seq = -1;
        self.plan_active = false;
        self.plan_seq = -1;
        self.call_times.clear();
    }

    /// Merges durable events in sequence order, ignoring any already projected.
    ///
    /// A windowed `session/follow` snapshot, an older history page, and a live
    /// event can arrive in any order; rows stay sorted by `seq` and each event
    /// contributes once.
    ///
    /// @param events - Durable events from any source.
    pub fn merge<'a>(&mut self, events: impl IntoIterator<Item = &'a SessionWireEvent>) {
        let mut incoming: Vec<&SessionWireEvent> = events
            .into_iter()
            .filter(|event| !self.applied.contains(&event.seq))
            .collect();
        if incoming.is_empty() {
            return;
        }
        incoming.sort_by_key(|event| event.seq);
        for event in incoming {
            // The same sequence can repeat inside one batch (a snapshot edge
            // plus a live frame); project it once.
            if !self.applied.insert(event.seq) {
                continue;
            }
            self.last_seq = self.last_seq.max(event.seq);
            // Pair each result with its call for the rendered duration.
            if event.event_type == "tool/call" {
                if let Some(call_id) = call_id_of(event) {
                    if self.call_times.len() >= 2048 {
                        // Drop the oldest in-flight call to stay bounded.
                        if let Some(oldest) = self
                            .call_times
                            .iter()
                            .min_by_key(|(_, (seq, _))| *seq)
                            .map(|(id, _)| id.clone())
                        {
                            self.call_times.remove(&oldest);
                        }
                    }
                    self.call_times.insert(call_id, (event.seq, event.time));
                }
            }
            let at = self.rows.partition_point(|row| row.seq < event.seq);
            if event.event_type == "plan/mode" && event.seq >= self.plan_seq {
                self.plan_active = event
                    .data
                    .get("active")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.plan_seq = event.seq;
            }
            // Turn outcome comes from `turn/end.reason.kind`, which the runtime
            // records for every closed turn. A failed tool call is a row above,
            // not a turn failure, so it never sets the run state. Batches can
            // arrive out of order across pages, so only turn events at or
            // after the newest seen sequence may move the state.
            if event.seq >= self.state_seq {
                match event.event_type.as_str() {
                    "turn/start" => {
                        self.open_turn = true;
                        self.state_seq = event.seq;
                    }
                    "turn/end" => {
                        self.open_turn = false;
                        self.last_turn_end = event
                            .data
                            .get("reason")
                            .and_then(|reason| reason.get("kind"))
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        self.state_seq = event.seq;
                    }
                    _ => {}
                }
            }
            let duration_ms = match event.event_type.as_str() {
                "tool/result" => call_duration_of(&mut self.call_times, event),
                _ => None,
            };
            for (offset, row) in project(event, duration_ms).into_iter().enumerate() {
                self.bytes += row.text.len();
                self.has_cjk |= crate::fonts::contains_cjk(&row.text);
                self.rows.insert(at + offset, row);
            }
        }
        self.trim();
    }

    /// Applies one durable event.
    pub fn apply(&mut self, event: &SessionWireEvent) {
        self.merge(std::iter::once(event));
    }

    /// Applies a snapshot's records without discarding older retained history.
    pub fn apply_snapshot(&mut self, records: &[dsh_client_core::follow::EventFrame]) {
        self.merge(records.iter().map(|record| &record.event));
    }

    /// State of the most recent turn, derived from `turn/start` and
    /// `turn/end` events only. Tool errors never make a turn look failed;
    /// `max-tokens` reads as `partial` and an aborted turn as `interrupted`.
    pub fn run_state(&self, streaming: bool) -> &str {
        if streaming {
            "streaming"
        } else if self.open_turn {
            "running"
        } else {
            match self.last_turn_end.as_deref() {
                Some("error") => "failed",
                Some("blocked") => "blocked",
                Some("aborted") | Some("interrupted") => "interrupted",
                Some("max-tokens") => "partial",
                Some("forked") => "forked",
                Some(other) => other,
                None => "completed",
            }
        }
    }

    /// Highest projected sequence, or `-1` before any event.
    pub fn last_seq(&self) -> i64 {
        self.last_seq
    }

    /// Whether the folded plan-mode state is active.
    pub fn plan_active(&self) -> bool {
        self.plan_active
    }

    /// Retained rows, oldest first.
    pub fn rows(&self) -> &[TranscriptRow] {
        &self.rows
    }

    /// Sequence of the oldest retained event, if any.
    pub fn oldest_seq(&self) -> Option<i64> {
        self.rows.first().map(|row| row.seq)
    }

    /// Whether any projected row needed a CJK face.
    pub fn has_cjk(&self) -> bool {
        self.has_cjk
    }

    /// True before any row was projected.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Enforces the row and byte budgets from the oldest row.
    fn trim(&mut self) {
        while self.rows.len() > MAX_ROWS || self.bytes > MAX_BYTES {
            let dropped = self.rows.remove(0);
            self.bytes -= dropped.text.len();
        }
        if let Some(oldest) = self.oldest_seq() {
            self.applied = self.applied.split_off(&oldest);
        }
    }
}

/// Resolves the pairing call id carried by a call or result event.
fn call_id_of(event: &SessionWireEvent) -> Option<String> {
    event
        .data
        .get("message")
        .and_then(|message| message.get("source"))
        .and_then(|source| source.get("callId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            event
                .data
                .get("message")
                .and_then(|message| message.get("toolCallId"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| event.data.get("callId").and_then(Value::as_str).map(str::to_owned))
}

/// Pops the paired call time for one result and reports the elapsed time.
fn call_duration_of(
    call_times: &mut HashMap<String, (i64, i64)>,
    event: &SessionWireEvent,
) -> Option<i64> {
    let (_, call_time) = call_times.remove(&call_id_of(event)?)?;
    let duration = event.time - call_time;
    (duration >= 0).then_some(duration)
}

/// Extracts the exit code the shell renderers stamp as a marker line.
fn marker_exit_code(text: &str) -> Option<i64> {
    for line in text.lines() {
        if let Some(code) = line.strip_prefix("[exit code: ") {
            return code.strip_suffix(']')?.parse().ok();
        }
    }
    None
}

/// Summarizes one rendered tool output as stdout/stderr sizes plus exit and
/// duration facts, without reinterpreting the output itself.
fn tool_run_summary(text: &str, duration_ms: Option<i64>) -> Option<String> {
    let (_, stderr) = match text.split_once("[stderr]\n") {
        Some((_, stderr)) => ("", stderr),
        None => ("", ""),
    };
    let stderr = stderr
        .lines()
        .filter(|line| {
            !line.starts_with("[exit code: ")
                && !line.starts_with("[timed out after ")
                && !line.starts_with("[killed by signal: ")
                && !line.starts_with("[stopped: ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut parts = Vec::new();
    if let Some(duration) = duration_ms {
        parts.push(format!("{duration}ms"));
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr {}B", stderr.len()));
    }
    if let Some(exit_code) = marker_exit_code(text) {
        if exit_code != 0 {
            parts.push(format!("exit code {exit_code}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn project(event: &SessionWireEvent, duration_ms: Option<i64>) -> Vec<TranscriptRow> {
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
            message_id: None,
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
                message_id: None,
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
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
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
                    message_id: None,
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
                    message_id: event
                        .data
                        .get("message")
                        .and_then(|message| message.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
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
            let mut text = content_text(message.get("content"));
            if is_error {
                if let Some(error) = event.data.get("error") {
                    let name = error.get("name").and_then(Value::as_str);
                    let code = error.get("code").and_then(Value::as_str);
                    let detail = match (name, code) {
                        (Some(name), Some(code)) => Some(format!("{name} ({code})")),
                        (Some(name), None) => Some(name.to_owned()),
                        (None, Some(code)) => Some(code.to_owned()),
                        (None, None) => None,
                    };
                    if let Some(detail) = detail {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str("Error: ");
                        text.push_str(&detail);
                    }
                }
            }
            if let Some(summary) = tool_run_summary(&text, duration_ms) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("— ");
                text.push_str(&summary);
            }
            vec![TranscriptRow {
                seq: event.seq,
                kind: if is_error {
                    RowKind::ToolError
                } else {
                    RowKind::ToolResult
                },
                title: if is_error {
                    "Tool error".to_owned()
                } else {
                    "Tool result".to_owned()
                },
                text,
                message_id: None,
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
                message_id: None,
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
                message_id: None,
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
                message_id: None,
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
                message_id: None,
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
            let status = todo
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
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
        message_id: None,
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
        message_id: None,
    }
}

fn goal_row(event: &SessionWireEvent) -> TranscriptRow {
    if event.data.get("operation").and_then(Value::as_str) == Some("clear") {
        return TranscriptRow {
            seq: event.seq,
            kind: RowKind::Goal,
            title: "Goal cleared".to_owned(),
            text: String::new(),
            message_id: None,
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
        message_id: None,
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
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
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

    fn message(seq: i64, text: &str) -> SessionWireEvent {
        event(
            seq,
            "user/message",
            serde_json::json!({"content": [{"type": "text", "text": text}]}),
        )
    }

    fn frame(seq: i64, text: &str) -> dsh_client_core::follow::EventFrame {
        dsh_client_core::follow::EventFrame {
            frame_type: String::new(),
            event: message(seq, text),
        }
    }

    #[test]
    fn records_when_projected_text_needs_a_cjk_face() {
        let mut transcript = Transcript::default();
        transcript.merge([&message(1, "việt nam")]);
        assert!(!transcript.has_cjk());
        transcript.merge([&message(2, "中文")]);
        assert!(transcript.has_cjk());
    }

    #[test]
    fn merge_orders_events_from_any_source_and_drops_duplicates() {
        let mut transcript = Transcript::default();
        transcript.merge([
            &message(3, "third"),
            &message(1, "first"),
            &message(3, "third"),
        ]);
        assert_eq!(
            transcript
                .rows()
                .iter()
                .map(|row| row.seq)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        // A history page can land after a newer live event.
        transcript.merge([&message(2, "second")]);
        assert_eq!(
            transcript
                .rows()
                .iter()
                .map(|row| row.seq)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            transcript
                .rows()
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    #[test]
    fn windowed_snapshot_keeps_older_history() {
        let mut transcript = Transcript::default();
        let history: Vec<_> = (1..=5).map(|seq| message(seq, "history")).collect();
        transcript.merge(&history);
        // `session/follow` snapshots carry a bounded window, not the whole log.
        transcript.apply_snapshot(&[frame(4, "history"), frame(5, "history"), frame(6, "live")]);
        assert_eq!(
            transcript
                .rows()
                .iter()
                .map(|row| row.seq)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn older_page_prepends_in_sequence_order() {
        let mut transcript = Transcript::default();
        let newer: Vec<_> = (101..=110).map(|seq| message(seq, "newer")).collect();
        let older: Vec<_> = (1..=100).map(|seq| message(seq, "older")).collect();
        transcript.merge(&newer);
        transcript.merge(&older);
        let seqs: Vec<_> = transcript.rows().iter().map(|row| row.seq).collect();
        assert_eq!(seqs.first(), Some(&1));
        assert_eq!(seqs.last(), Some(&110));
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(seqs.len(), 110);
    }

    #[test]
    fn row_budget_keeps_the_newest_rows_and_their_dedupe_set() {
        let mut transcript = Transcript::default();
        let events: Vec<_> = (1..=MAX_ROWS as i64 + 100)
            .map(|seq| message(seq, "row"))
            .collect();
        transcript.merge(&events);
        assert_eq!(transcript.rows().len(), MAX_ROWS);
        assert_eq!(transcript.oldest_seq(), Some(101));
        assert_eq!(
            transcript.rows().last().map(|row| row.seq),
            Some(MAX_ROWS as i64 + 100)
        );
        // Replaying a dropped event must neither duplicate nor reorder rows.
        let before = transcript.rows().len();
        transcript.merge([&events[0]]);
        assert_eq!(transcript.rows().len(), before);
    }

    #[test]
    fn plan_mode_folds_latest_event_in_sequence_order() {
        let mut transcript = Transcript::default();
        assert!(!transcript.plan_active());
        transcript.apply(&event(1, "plan/mode", serde_json::json!({"active": true})));
        assert!(transcript.plan_active());
        transcript.apply(&event(2, "plan/mode", serde_json::json!({"active": false})));
        assert!(!transcript.plan_active());
        // An older plan event arriving late must not overwrite the newer state.
        transcript.apply(&event(0, "plan/mode", serde_json::json!({"active": true})));
        assert!(!transcript.plan_active());
        transcript.reset();
        assert!(!transcript.plan_active());
    }

    #[test]
    fn tool_results_carry_duration_exit_and_stderr_facts() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(1, "tool/call", serde_json::json!({
            "turn": 1, "step": 1, "callId": "call-1", "name": "bash", "arguments": "{}"
        })));
        transcript.apply(&event(2, "tool/result", serde_json::json!({
            "turn": 1, "step": 1,
            "message": {
                "source": {"kind": "tool", "callId": "call-1"},
                "content": [{"type": "text", "text": "out\n[stderr]\nboom\n[exit code: 2]"}],
                "isError": false
            }
        })));
        let row = transcript.rows().last().unwrap();
        assert!(row.text.contains("out"), "{row:?}");
        assert!(row.text.contains("boom"), "{row:?}");
        // Both fixture times are 0, so the duration reads 0ms; exit/stderr shown.
        assert!(row.text.contains("0ms"), "{row:?}");
        assert!(row.text.contains("exit code 2"), "{row:?}");
        assert!(row.text.contains("stderr 4B"), "{row:?}");
    }

    #[test]
    fn tool_results_pair_with_calls_for_wall_time_durations() {
        let mut transcript = Transcript::default();
        let call = SessionWireEvent {
            event_type: "tool/call".into(),
            seq: 1,
            time: 100,
            data: serde_json::json!({"turn": 1, "step": 1, "callId": "call-1", "name": "bash", "arguments": "{}"}),
            ignorable: None,
            source_event_seqs: None,
            surface_op: None,
        };
        let result = SessionWireEvent {
            event_type: "tool/result".into(),
            seq: 2,
            time: 250,
            data: serde_json::json!({
                "turn": 1, "step": 1,
                "message": {
                    "toolCallId": "call-1",
                    "content": [{"type": "text", "text": "ok"}],
                    "isError": false
                }
            }),
            ignorable: None,
            source_event_seqs: None,
            surface_op: None,
        };
        transcript.merge([&call, &result]);
        let row = transcript.rows().last().unwrap();
        assert!(row.text.contains("150ms"), "{row:?}");
    }

    #[test]
    fn last_seq_tracks_the_highest_projected_event() {
        let mut transcript = Transcript::default();
        assert_eq!(transcript.last_seq(), -1);
        transcript.apply(&event(5, "user/message", serde_json::json!({"content": []})));
        assert_eq!(transcript.last_seq(), 5);
        transcript.apply(&event(3, "user/message", serde_json::json!({"content": []})));
        assert_eq!(transcript.last_seq(), 5);
        transcript.reset();
        assert_eq!(transcript.last_seq(), -1);
    }

    #[test]
    fn run_state_follows_turn_end_reason_not_tool_errors() {
        let mut transcript = Transcript::default();
        assert_eq!(transcript.run_state(false), "completed");
        assert_eq!(transcript.run_state(true), "streaming");
        // A failed tool call is a row, not a failed turn.
        transcript.apply(&event(
            1,
            "tool/result",
            serde_json::json!({"message": {"isError": true, "content": []}}),
        ));
        assert_eq!(transcript.run_state(false), "completed");
        // A turn without its closer reads as running.
        transcript.apply(&event(2, "turn/start", serde_json::json!({"turn": 1})));
        assert_eq!(transcript.run_state(false), "running");
        transcript.apply(&event(
            3,
            "turn/end",
            serde_json::json!({"turn": 1, "reason": {"kind": "error"}}),
        ));
        assert_eq!(transcript.run_state(false), "failed");
    }

    #[test]
    fn run_state_distinguishes_blocked_partial_and_interrupted_turns() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "turn/end",
            serde_json::json!({"turn": 1, "reason": {"kind": "blocked"}}),
        ));
        assert_eq!(transcript.run_state(false), "blocked");
        transcript.apply(&event(
            2,
            "turn/end",
            serde_json::json!({"turn": 2, "reason": {"kind": "max-tokens"}}),
        ));
        assert_eq!(transcript.run_state(false), "partial");
        transcript.apply(&event(
            3,
            "turn/end",
            serde_json::json!({"turn": 3, "reason": {"kind": "aborted"}}),
        ));
        assert_eq!(transcript.run_state(false), "interrupted");
        transcript.apply(&event(
            4,
            "turn/end",
            serde_json::json!({"turn": 4, "reason": {"kind": "completed"}}),
        ));
        assert_eq!(transcript.run_state(false), "completed");
    }

    #[test]
    fn older_turn_events_do_not_regress_run_state() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            5,
            "turn/end",
            serde_json::json!({"turn": 2, "reason": {"kind": "error"}}),
        ));
        assert_eq!(transcript.run_state(false), "failed");
        // An older page arriving late must not overwrite the newer outcome.
        transcript.apply(&event(
            1,
            "turn/end",
            serde_json::json!({"turn": 1, "reason": {"kind": "completed"}}),
        ));
        assert_eq!(transcript.run_state(false), "failed");
    }

    #[test]
    fn live_follow_frames_project_in_wire_order() {
        // These are the frames `session/follow` delivers: a snapshot, then events.
        let snapshot: dsh_client_core::follow::FollowFrame =
            serde_json::from_value(serde_json::json!({
                "type": "snapshot",
                "header": {"version": 1, "id": "s", "createdAt": 0, "isSeeded": false},
                "cursor": 2,
                "records": [frame(1, "first"), frame(2, "second")],
                "hasMore": false,
                "projections": {},
            }))
            .unwrap();
        let live: dsh_client_core::follow::FollowFrame = serde_json::from_value(
            serde_json::json!({"type": "event", "event": frame(3, "third").event}),
        )
        .unwrap();

        let mut transcript = Transcript::default();
        let dsh_client_core::follow::FollowFrame::Snapshot(snapshot) = snapshot else {
            panic!("expected a snapshot frame");
        };
        transcript.apply_snapshot(&snapshot.records);
        let dsh_client_core::follow::FollowFrame::Event(live) = live else {
            panic!("expected an event frame");
        };
        transcript.apply(&live.event);
        transcript.apply(&live.event);
        assert_eq!(
            transcript
                .rows()
                .iter()
                .map(|row| row.text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

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
            serde_json::json!({
                "message": {"isError": true, "content": [{"type": "text", "text": "exit 1"}]},
                "error": {"name": "FsError", "code": "ENOENT"}
            }),
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
        let titles: Vec<_> = transcript
            .rows()
            .iter()
            .map(|row| row.title.as_str())
            .collect();
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
        assert_eq!(transcript.rows()[0].text, "exit 1\nError: FsError (ENOENT)");
        assert_eq!(transcript.rows()[0].kind, RowKind::ToolError);
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
        transcript.apply(&event(5, "plan/mode", serde_json::json!({"active": true})));
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
