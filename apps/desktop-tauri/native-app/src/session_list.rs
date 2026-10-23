//! Session list rendering helpers.

use dsh_client_core::session::SessionSummary;
use dsh_client_core::sessions::SessionSearchResponse;
use eframe::egui;

/// Truncates a string to `max` characters with an ellipsis.
pub fn truncate(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(max.saturating_sub(1))
                .collect::<String>()
        )
    }
}

/// Renders a compact session row label without truncating its relative age.
pub fn session_row_label(title: &str, running: bool, blank: bool, age: &str) -> String {
    let prefix = if running { "● " } else { "" };
    let suffix = if blank { " (blank)" } else { "" };
    format!("{prefix}{}{suffix}  ·  {age}", truncate(title, 26))
}

/// Returns the next visible session index for an arrow-key movement.
pub fn next_index(current: Option<usize>, direction: i8, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(current) = current else {
        return Some(0);
    };
    Some(if direction > 0 {
        current.saturating_add(1).min(len - 1)
    } else {
        current.saturating_sub(1)
    })
}

/// Renders session summaries or search matches and returns a clicked session id.
pub fn render(
    ui: &mut egui::Ui,
    sessions: &[SessionSummary],
    search: Option<&SessionSearchResponse>,
    selected_session: Option<&str>,
    now: i64,
) -> Option<String> {
    let mut selected = None;
    egui::ScrollArea::vertical()
        .id_salt("session-inventory")
        .max_height(300.0)
        .show(ui, |ui| {
            if let Some(results) = search {
                for item in &results.items {
                    let display = truncate(&item.session_id, 26);
                    if ui
                        .selectable_label(
                            selected_session == Some(item.session_id.as_str()),
                            display,
                        )
                        .on_hover_text(format!("{}\n{}", item.session_id, item.snippet))
                        .clicked()
                    {
                        selected = Some(item.session_id.clone());
                    }
                }
                if results.has_more {
                    ui.small("More matches exist; refine your search.");
                }
            } else {
                for summary in sessions {
                    let title = session_label(summary);
                    let label = session_row_label(
                        &title,
                        summary.running,
                        summary.blank,
                        &relative_time(summary.updated_at, now),
                    );
                    if ui
                        .selectable_label(
                            selected_session == Some(summary.session_id.as_str()),
                            label,
                        )
                        .on_hover_text(format!("{}\n{}", summary.session_id, title))
                        .clicked()
                    {
                        selected = Some(summary.session_id.clone());
                    }
                }
            }
        });
    selected
}

/// Display title projected for a session, or its identity when no title exists.
fn session_label(summary: &SessionSummary) -> String {
    summary
        .projections
        .as_ref()
        .and_then(|projections| projections.get("title"))
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.is_empty())
        .map_or_else(|| summary.session_id.clone(), str::to_owned)
}

/// Relative update age, tolerant of second- or millisecond-scale timestamps.
fn relative_time(updated_at: i64, now: i64) -> String {
    let updated_at = if updated_at < 1_000_000_000_000 {
        updated_at.saturating_mul(1000)
    } else {
        updated_at
    };
    let seconds = now.saturating_sub(updated_at) / 1000;
    match seconds {
        ..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        86_400..=2_591_999 => format!("{}d ago", seconds / 86_400),
        _ => format!("{}mo ago", seconds / 2_592_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_strings() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a".repeat(30).as_str(), 5), "aaaa…");
        assert_eq!(truncate("long", 0), "");
        assert_eq!(truncate("é🙂abc", 3), "é🙂…");
    }

    #[test]
    fn row_label_includes_status_and_age() {
        let label = session_row_label("Fix login", true, false, "2m ago");
        assert!(label.contains("● Fix login"));
        assert!(label.contains("2m ago"));
    }

    #[test]
    fn row_label_marks_blank() {
        let label = session_row_label("Empty", false, true, "1h ago");
        assert!(label.contains("(blank)"));
    }

    #[test]
    fn arrow_navigation_starts_at_first_row_and_stays_in_bounds() {
        assert_eq!(next_index(None, 1, 3), Some(0));
        assert_eq!(next_index(None, -1, 3), Some(0));
        assert_eq!(next_index(Some(0), -1, 3), Some(0));
        assert_eq!(next_index(Some(2), 1, 3), Some(2));
        assert_eq!(next_index(Some(0), 1, 3), Some(1));
        assert_eq!(next_index(None, 1, 0), None);
    }

    fn summary(projections: Option<serde_json::Value>) -> SessionSummary {
        SessionSummary {
            agent_available: true,
            format_status: None,
            session_id: "session-1".into(),
            updated_at: 1_760_000_000_000,
            running: false,
            blank: false,
            parent_session_id: None,
            origin: None,
            cwd: None,
            projections,
        }
    }

    #[test]
    fn prefers_projected_titles_and_falls_back_to_identity() {
        assert_eq!(
            session_label(&summary(Some(serde_json::json!({"title": "Roadmap"})))),
            "Roadmap"
        );
        for projections in [
            Some(serde_json::json!({"title": null})),
            Some(serde_json::json!({"title": ""})),
            Some(serde_json::json!({})),
            None,
        ] {
            assert_eq!(session_label(&summary(projections)), "session-1");
        }
    }

    #[test]
    fn relative_age_handles_seconds_milliseconds_and_clock_skew() {
        let now = 1_760_000_000_000;
        for (updated, expected) in [
            (now, "just now"),
            (now - 30_000, "just now"),
            (now - 90_000, "1m ago"),
            (now - 7_200_000, "2h ago"),
            (now - 172_800_000, "2d ago"),
            (now - 5_184_000_000, "2mo ago"),
        ] {
            assert_eq!(relative_time(updated, now), expected);
        }
        assert_eq!(relative_time(1_760_000_000 - 120, now), "2m ago");
        assert_eq!(relative_time(now + 60_000, now), "just now");
    }
}
