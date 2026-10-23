//! Keyboard behavior for the native chat composer.

/// Whether Ctrl+Enter should submit the focused multiline composer.
pub fn submit_requested(focused: bool, enter_pressed: bool, control: bool) -> bool {
    focused && enter_pressed && control
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submits_only_for_focused_control_enter() {
        assert!(submit_requested(true, true, true));
        assert!(!submit_requested(false, true, true));
        assert!(!submit_requested(true, false, true));
        assert!(!submit_requested(true, true, false));
    }
}
