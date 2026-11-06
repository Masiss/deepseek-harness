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
pub const EXPLORER_DEFAULT: f32 = 390.0;
pub const EXPLORER_MIN: f32 = 280.0;
pub const EXPLORER_MAX: f32 = 520.0;

/// Changes column default and bounds when it is docked on the right.
pub const CHANGES_DEFAULT: f32 = 380.0;
pub const CHANGES_MIN: f32 = 300.0;
pub const CHANGES_MAX: f32 = 620.0;

/// Width below which the Explorer switches between the tree and preview panes.
pub const EXPLORER_SINGLE_PANE_MAX_WIDTH: f32 = 900.0;

/// Minimum viewport width for showing Explorer, conversation, and Changes together.
pub const EXPLORER_DOCK_MIN_WIDTH: f32 = 1240.0;
/// Minimum viewport width for adding the Changes column to the Explorer dock.
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
