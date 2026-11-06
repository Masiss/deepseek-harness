//! Shared sizing values and headless geometry checks for the desktop shell.

/// V2 top bar height (px).
pub const TOP_BAR_HEIGHT: f32 = 30.0;
/// V2 agent tab height (px).
pub const AGENT_TAB_HEIGHT: f32 = 34.0;
/// V2 bottom bar height (px).
pub const BOTTOM_BAR_HEIGHT: f32 = 26.0;
/// Retired egui shell's icon rail width; the gpui shell follows the V2 top bar instead.
#[cfg_attr(not(feature = "legacy-egui"), allow(dead_code))]
pub const NAV_RAIL_WIDTH: f32 = 82.0;

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

#[cfg(test)]
impl LayoutMetrics {
    /// True when the central pane has room for a transcript and composer.
    pub fn central_usable(&self) -> bool {
        self.central.0 >= 320.0 && self.central.1 >= 200.0
    }
}

#[cfg(all(test, feature = "legacy-egui"))]
mod tests {
    use super::*;
    use eframe::egui::{self, Pos2, RawInput, Rect, Vec2};

    fn render_shell(width: f32, height: f32) -> LayoutMetrics {
        let context = egui::Context::default();
        let mut measured = None;
        context.begin_pass(RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
            ..RawInput::default()
        });
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height));
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("layout-test"),
            egui::UiBuilder::new().max_rect(screen),
        );
        let rail = egui::Panel::left("layout-test-rail")
            .resizable(false)
            .exact_size(NAV_RAIL_WIDTH)
            .show(&mut ui, |ui| ui.max_rect());
        let sidebar = egui::Panel::left("layout-test-sidebar")
            .resizable(true)
            .default_size(SIDEBAR_DEFAULT)
            .min_size(SIDEBAR_MIN)
            .max_size(SIDEBAR_MAX)
            .show(&mut ui, |ui| ui.max_rect());
        egui::CentralPanel::default().show(&mut ui, |ui| {
            let rect = ui.max_rect();
            measured = Some(LayoutMetrics {
                viewport: (width, height),
                nav_rail: (rail.response.rect.width(), rail.response.rect.height()),
                sidebar: (
                    sidebar.response.rect.width(),
                    sidebar.response.rect.height(),
                ),
                central: (rect.width(), rect.height()),
            });
        });
        let mut output = context.end_pass();
        output.textures_delta.clear();
        measured.expect("layout panels should render")
    }

    #[test]
    fn metrics_fit_common_viewports() {
        for (width, height) in [
            (1024.0, 768.0),
            (1440.0, 900.0),
            (1920.0, 1080.0),
            (1119.0, 1381.0),
        ] {
            let metrics = render_shell(width, height);
            assert!(metrics.central_usable(), "{width}x{height}: {metrics:?}");
            assert_eq!(metrics.nav_rail.0, NAV_RAIL_WIDTH);
            assert!(metrics.sidebar.0 >= SIDEBAR_MIN);
            assert!(metrics.sidebar.0 <= SIDEBAR_MAX);
            assert!(metrics.central.0 <= width);
            assert!(metrics.central.1 <= height);
        }
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    #[test]
    fn sidebar_bounds_are_ordered() {
        assert!(SIDEBAR_MIN < SIDEBAR_DEFAULT);
        assert!(SIDEBAR_DEFAULT < SIDEBAR_MAX);
    }

    #[test]
    fn explorer_bounds_are_ordered() {
        assert!(EXPLORER_MIN < EXPLORER_DEFAULT);
        assert!(EXPLORER_DEFAULT < EXPLORER_MAX);
        assert!(EXPLORER_SINGLE_PANE_MAX_WIDTH < EXPLORER_DOCK_MIN_WIDTH);
    }

    #[test]
    fn side_panels_are_dropped_before_the_agent_column_squeezes() {
        let wide = dock_visibility(1920.0, true, true);
        assert_eq!(
            wide,
            DockVisibility {
                explorer: true,
                changes: true
            }
        );
        let laptop = dock_visibility(1366.0, true, true);
        assert!(laptop.explorer && laptop.changes);
        let narrow = dock_visibility(1024.0, true, true);
        assert_eq!(
            narrow,
            DockVisibility {
                explorer: false,
                changes: false
            }
        );
        assert!(!dock_visibility(1366.0, false, false).explorer);
        assert!(!dock_visibility(1366.0, false, false).changes);
    }

    #[test]
    fn changes_bounds_are_ordered() {
        assert!(CHANGES_MIN < CHANGES_DEFAULT);
        assert!(CHANGES_DEFAULT < CHANGES_MAX);
        assert!(CHANGES_DOCK_MIN_WIDTH <= EXPLORER_DOCK_MIN_WIDTH);
    }

    #[test]
    fn bar_heights_are_positive() {
        assert!(TOP_BAR_HEIGHT > 0.0);
        assert!(AGENT_TAB_HEIGHT > 0.0);
        assert!(BOTTOM_BAR_HEIGHT > 0.0);
        assert!(NAV_RAIL_WIDTH > 0.0);
    }
}
