//! Shared sizing values and headless geometry checks for the desktop shell.

/// Navigation rail width (icon-first).
pub const NAV_RAIL_WIDTH: f32 = 56.0;

/// Context sidebar default and bounds.
pub const SIDEBAR_DEFAULT: f32 = 280.0;
pub const SIDEBAR_MIN: f32 = 220.0;
pub const SIDEBAR_MAX: f32 = 400.0;

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

#[cfg(test)]
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
