//! Shared sizing values and headless geometry checks for the desktop shell.

/// V2 top bar height (px).
pub const TOP_BAR_HEIGHT: f32 = 30.0;
/// V2 agent tab height (px).
pub const AGENT_TAB_HEIGHT: f32 = 34.0;
/// V2 bottom bar height (px).
pub const BOTTOM_BAR_HEIGHT: f32 = 26.0;
/// Context sidebar default and bounds.
pub const SIDEBAR_DEFAULT: f32 = 278.0;
pub const SIDEBAR_MIN: f32 = 220.0;
pub const SIDEBAR_MAX: f32 = 400.0;

/// Explorer panel default and bounds when it is docked beside the conversation.
pub const EXPLORER_DEFAULT: f32 = 300.0;
pub const EXPLORER_MIN: f32 = 210.0;
pub const EXPLORER_MAX: f32 = 390.0;

/// Changes column default and bounds when it is docked on the right.
pub const CHANGES_DEFAULT: f32 = 565.0;
pub const CHANGES_MIN: f32 = 340.0;
pub const CHANGES_MAX: f32 = 720.0;

/// Width the conversation keeps when both docks are shown (px).
pub const AGENT_MIN_WIDTH: f32 = 420.0;

/// Grab width of the draggable divider between two docked panels (px).
pub const PANEL_DIVIDER_WIDTH: f32 = 4.0;

/// Width below which the Explorer switches between the tree and preview panes.
pub const EXPLORER_SINGLE_PANE_MAX_WIDTH: f32 = 900.0;

/// Minimum viewport width for showing Explorer, conversation, and Changes together.
///
/// It is the two docks at their defaults plus the conversation's minimum, so a
/// viewport that shows both docks never leaves the conversation narrower than
/// [`AGENT_MIN_WIDTH`]. Narrower viewports keep the Changes column and drop the
/// Explorer, which needs [`EXPLORER_SINGLE_PANE_MAX_WIDTH`] to be useful anyway.
pub const EXPLORER_DOCK_MIN_WIDTH: f32 = EXPLORER_DEFAULT
    + PANEL_DIVIDER_WIDTH
    + AGENT_MIN_WIDTH
    + PANEL_DIVIDER_WIDTH
    + CHANGES_DEFAULT;
/// Minimum viewport width for showing the Changes column beside the conversation.
pub const CHANGES_DOCK_MIN_WIDTH: f32 = 1180.0;

/// Which docked side panels fit in one viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockVisibility {
    pub explorer: bool,
    pub changes: bool,
}

/// Decides side-panel visibility for a viewport, prioritizing the Agent column.
pub fn dock_visibility(
    viewport_width: f32,
    show_explorer: bool,
    show_changes: bool,
) -> DockVisibility {
    DockVisibility {
        explorer: show_explorer && viewport_width >= EXPLORER_DOCK_MIN_WIDTH,
        changes: show_changes && viewport_width >= CHANGES_DOCK_MIN_WIDTH,
    }
}

/// Computed panel rectangles for one viewport.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    pub viewport: (f32, f32),
    pub nav_rail: (f32, f32),
    pub sidebar: (f32, f32),
    pub central: (f32, f32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_docks_keep_the_agent_column_at_its_minimum_width() {
        let docks = dock_visibility(EXPLORER_DOCK_MIN_WIDTH, true, true);
        assert!(docks.explorer && docks.changes);
        let agent = EXPLORER_DOCK_MIN_WIDTH
            - EXPLORER_DEFAULT
            - CHANGES_DEFAULT
            - 2.0 * PANEL_DIVIDER_WIDTH;
        assert_eq!(agent, AGENT_MIN_WIDTH);
    }

    #[test]
    fn a_narrower_viewport_drops_the_explorer_before_the_changes_column() {
        let docks = dock_visibility(EXPLORER_DOCK_MIN_WIDTH - 1.0, true, true);
        assert!(!docks.explorer);
        assert!(docks.changes);
        let agent = CHANGES_DOCK_MIN_WIDTH - CHANGES_DEFAULT - PANEL_DIVIDER_WIDTH;
        assert!(agent >= AGENT_MIN_WIDTH);
        let none = dock_visibility(CHANGES_DOCK_MIN_WIDTH - 1.0, true, true);
        assert!(!none.explorer && !none.changes);
    }
}
