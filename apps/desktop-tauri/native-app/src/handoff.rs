//! Handoff summary built from one Session's durable transcript.
//!
//! The summary is derived, never invented: it restates the last turn's state,
//! the goal, the remaining plan work, and the tool failures already recorded in
//! the log, so another person or agent can continue without re-reading
//! everything. Writing the summary to the workspace is a separate, explicit
//! action owned by the shell.

use crate::transcript::{RowKind, Transcript, TranscriptRow};
use dsh_client_core::goals::{GoalActivation, GoalPhase, GoalView};

/// Most recent user prompts quoted in the summary.
const RECENT_PROMPTS: usize = 5;
/// Longest single quoted row body.
const QUOTE_CHARS: usize = 300;

/// One Session's handoff inputs.
#[derive(Default)]
pub struct HandoffInput<'a> {
    pub session_id: Option<&'a str>,
    pub title: &'a str,
    pub goal: Option<&'a GoalView>,
    pub plan_active: bool,
    pub run_state: &'a str,
    pub rows: &'a [TranscriptRow],
}

/// Renders the handoff as Markdown.
///
/// @param input - Session identity, goal, run state, and projected rows.
/// @returns Markdown ending with a newline.
pub fn render(input: &HandoffInput<'_>) -> String {
    let mut out = String::new();
    out.push_str("# Session handoff\n\n");
    if let Some(session) = input.session_id {
        out.push_str(&format!("- Session: `{session}`\n"));
    }
    if !input.title.is_empty() {
        out.push_str(&format!("- Title: {}\n", input.title));
    }
    out.push_str(&format!("- Last turn: {}\n", input.run_state));
    out.push_str(&format!(
        "- Plan mode: {}\n",
        if input.plan_active { "on" } else { "off" }
    ));
    if let Some(goal) = input.goal {
        out.push_str(&format!("- Goal: {} ({:?})\n", goal.objective, goal.phase));
        out.push_str(&format!(
            "- Goal rounds: {} of {}\n",
            goal.rounds_started, goal.max_goal_rounds
        ));
        if let Some(reason) = &goal.blocked_reason {
            out.push_str(&format!("- Blocked reason: {reason:?}\n"));
        }
        let activation = match goal.activation {
            GoalActivation::Armed => "armed",
            GoalActivation::Disarmed => "disarmed",
        };
        out.push_str(&format!("- Goal continuation: {activation}\n"));
        if goal.phase == GoalPhase::Complete {
            out.push_str("- The goal is recorded complete; no continuation is offered.\n");
        }
    }

    let prompts: Vec<&TranscriptRow> = input
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::User)
        .rev()
        .take(RECENT_PROMPTS)
        .collect();
    if !prompts.is_empty() {
        out.push_str("\n## Recent requests\n\n");
        for row in prompts.into_iter().rev() {
            out.push_str(&format!("- {}\n", quote(&row.text)));
        }
    }

    let failures: Vec<&TranscriptRow> = input
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::ToolError)
        .rev()
        .take(5)
        .collect();
    if !failures.is_empty() {
        out.push_str("\n## Unresolved tool failures\n\n");
        for row in failures.into_iter().rev() {
            out.push_str(&format!("- {}\n", quote(&row.text)));
        }
        out.push_str(
            "\nThese are the failures the runtime recorded; confirm each one before retrying it.\n",
        );
    }

    let todos: Vec<&TranscriptRow> = input
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::Todo)
        .rev()
        .take(1)
        .collect();
    if let Some(todo) = todos.first() {
        out.push_str("\n## Last todo snapshot\n\n");
        out.push_str(&format!("{}\n", quote(&todo.text)));
    }

    let artifacts: Vec<&TranscriptRow> = input
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::Deliverable)
        .rev()
        .take(10)
        .collect();
    if !artifacts.is_empty() {
        out.push_str("\n## Artifacts\n\n");
        for row in artifacts.into_iter().rev() {
            out.push_str(&format!("- {}\n", quote(&row.text)));
        }
    }

    out.push_str(
        "\n## Next steps\n\nFill in what should happen next; this file was generated from the Session log and nothing here was executed.\n",
    );
    out
}

/// Renders the same handoff from a transcript and optional goal.
///
/// @param transcript - Projected durable rows.
/// @param session_id - Session identity, for the header line.
/// @param title - Session title, when known.
/// @param goal - Current goal, when the Session has one.
/// @param run_state - Last turn state as derived by the transcript.
/// @returns Markdown ending with a newline.
pub fn render_from(
    transcript: &Transcript,
    session_id: Option<&str>,
    title: &str,
    goal: Option<&GoalView>,
    run_state: &str,
) -> String {
    render(&HandoffInput {
        session_id,
        title,
        goal,
        plan_active: transcript.plan_active(),
        run_state,
        rows: transcript.rows(),
    })
}

/// Trims a quoted row to one bounded line.
fn quote(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= QUOTE_CHARS {
        return flat;
    }
    let truncated: String = flat.chars().take(QUOTE_CHARS).collect();
    format!("{truncated}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{RowKind, TranscriptRow};

    fn row(kind: RowKind, text: &str) -> TranscriptRow {
        TranscriptRow {
            seq: 1,
            kind,
            title: String::new(),
            text: text.to_owned(),
            message_id: None,
        }
    }

    #[test]
    fn summary_states_state_goal_and_failures_without_inventing_work() {
        let rows = vec![
            row(RowKind::User, "Fix the parser"),
            row(RowKind::ToolError, "cargo test  [exit code: 101]"),
            row(RowKind::Deliverable, "reports/out.md"),
        ];
        let goal = GoalView {
            id: "g1".to_owned(),
            revision: 1,
            objective: "Ship the parser".to_owned(),
            phase: GoalPhase::Active,
            blocked_reason: None,
            max_goal_rounds: 4,
            rounds_started: 2,
            created_at: 0,
            updated_at: 0,
            activation: GoalActivation::Armed,
        };
        let markdown = render(&HandoffInput {
            session_id: Some("s1"),
            title: "Parser",
            goal: Some(&goal),
            plan_active: true,
            run_state: "failed",
            rows: &rows,
        });
        assert!(markdown.starts_with("# Session handoff\n"));
        assert!(markdown.contains("- Session: `s1`"));
        assert!(markdown.contains("- Last turn: failed"));
        assert!(markdown.contains("- Plan mode: on"));
        assert!(markdown.contains("Ship the parser (Active)"));
        assert!(markdown.contains("2 of 4"));
        assert!(markdown.contains("Fix the parser"));
        assert!(markdown.contains("cargo test [exit code: 101]"));
        assert!(markdown.contains("reports/out.md"));
        assert!(markdown.contains("nothing here was executed"));
    }

    #[test]
    fn empty_session_still_produces_the_template_only() {
        let markdown = render(&HandoffInput {
            run_state: "completed",
            ..HandoffInput::default()
        });
        assert!(markdown.contains("- Last turn: completed"));
        assert!(!markdown.contains("## Recent requests"));
        assert!(!markdown.contains("## Unresolved tool failures"));
    }

    #[test]
    fn quotes_are_flattened_and_bounded() {
        let long = format!("line one\n{}", "x".repeat(400));
        let quoted = quote(&long);
        assert!(!quoted.contains('\n'));
        assert!(quoted.chars().count() <= QUOTE_CHARS + 1);
        assert!(quoted.ends_with('…'));
    }
}
