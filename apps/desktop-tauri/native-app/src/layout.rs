//! Pure layout sizing helpers for the desktop shell.

/// Navigation rail width (icon-first).
pub const NAV_RAIL_WIDTH: f32 = 56.0;

/// Context sidebar default and bounds.
pub const SIDEBAR_DEFAULT: f32 = 280.0;
pub const SIDEBAR_MIN: f32 = 220.0;
pub const SIDEBAR_MAX: f32 = 400.0;

/// Top title bar and bottom status bar heights.
pub const TITLE_BAR_HEIGHT: f32 = 36.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Composer minimum height.
pub const COMPOSER_MIN_HEIGHT: f32 = 96.0;

/// Computed panel rectangles for one viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutMetrics {
    pub viewport: (f32, f32),
    pub nav_rail: (f32, f32),
    pub sidebar: (f32, f32),
    pub central: (f32, f32),
    pub composer: (f32, f32),
}

impl LayoutMetrics {
    /// Computes metrics for a viewport, clamping the sidebar to its bounds.
    pub fn for_viewport(width: f32, height: f32, sidebar_width: f32) -> Self {
        let sidebar = sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        let central_width = (width - NAV_RAIL_WIDTH - sidebar).max(0.0);
        let central_height = (height - TITLE_BAR_HEIGHT - STATUS_BAR_HEIGHT).max(0.0);
        Self {
            viewport: (width, height),
            nav_rail: (NAV_RAIL_WIDTH, height),
            sidebar: (sidebar, height - TITLE_BAR_HEIGHT),
            central: (central_width, central_height),
            composer: (central_width, COMPOSER_MIN_HEIGHT),
        }
    }

    /// True when the central pane has room for a transcript and composer.
    pub fn central_usable(&self) -> bool {
        self.central.0 >= 320.0 && self.central.1 >= 200.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_fit_common_viewports() {
        for (width, height) in [(1024.0, 768.0), (1440.0, 900.0), (1920.0, 1080.0), (1119.0, 1381.0)] {
            let metrics = LayoutMetrics::for_viewport(width, height, SIDEBAR_DEFAULT);
            assert!(metrics.central_usable(), "{width}x{height}: {metrics:?}");
            assert_eq!(metrics.nav_rail.0, NAV_RAIL_WIDTH);
            assert!(metrics.sidebar.0 >= SIDEBAR_MIN);
        }
    }

    #[test]
    fn sidebar_clamps_to_bounds() {
        let too_small = LayoutMetrics::for_viewport(1440.0, 900.0, 100.0);
        assert_eq!(too_small.sidebar.0, SIDEBAR_MIN);
        let too_large = LayoutMetrics::for_viewport(1440.0, 900.0, 999.0);
        assert_eq!(too_large.sidebar.0, SIDEBAR_MAX);
    }

    #[test]
    fn central_width_never_negative() {
        let metrics = LayoutMetrics::for_viewport(100.0, 100.0, SIDEBAR_DEFAULT);
        assert!(metrics.central.0 >= 0.0);
        assert!(metrics.central.1 >= 0.0);
    }
}
