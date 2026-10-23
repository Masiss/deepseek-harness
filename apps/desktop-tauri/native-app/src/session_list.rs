//! Session list rendering helpers.

/// Truncates a string to `max` characters with an ellipsis.
pub fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(max.saturating_sub(1)).collect::<String>())
    }
}

/// Renders a compact session row label.
pub fn session_row_label(title: &str, running: bool, blank: bool, age: &str) -> String {
    let prefix = if running { "● " } else { "" };
    let suffix = if blank { " (blank)" } else { "" };
    format!("{prefix}{title}{suffix}  ·  {age}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_strings() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a".repeat(30).as_str(), 5), "aaaa…");
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
}
