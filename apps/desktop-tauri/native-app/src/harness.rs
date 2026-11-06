//! Headless gpui harness for the native shell.
//!
//! The harness opens the real `DesktopApp` in gpui's test window, so tests read
//! the layout the renderer actually produced instead of recomputing it. Elements
//! exposed with `debug_selector` are addressable through `debug_bounds`, and
//! input is simulated at their centre.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use gpui::{point, px, size, AppContext, Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext};

use crate::command_registry::CommandId;
use crate::shell::HarnessApp as DesktopApp;
use crate::shell::ShellServices;

/// One harness instance holding the window, the shell entity, and its test context.
pub struct Harness {
    app: Entity<DesktopApp>,
    visual: VisualTestContext,
    preferences: PathBuf,
}

impl Harness {
    /// Opens the shell at `width`×`height` in gpui's test window without a Host.
    ///
    /// @param width - Viewport width in logical pixels.
    /// @param height - Viewport height in logical pixels.
    /// @returns The harness whose shell is already laid out once.
    pub fn open(width: f32, height: f32) -> Self {
        let preferences = std::env::temp_dir().join(format!(
            "dsh-gpui-harness-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        let services = ShellServices {
            quitting: Arc::new(AtomicBool::new(false)),
            show_requested: Arc::new(AtomicBool::new(false)),
            hide_requested: Arc::new(AtomicBool::new(false)),
            hidden: Arc::new(AtomicBool::new(false)),
            tray_available: false,
            ui_preferences_path: preferences.clone(),
            spawn_host: false,
            commands: Arc::new(Mutex::new(Vec::<CommandId>::new())),
        };
        let mut cx = TestAppContext::single();
        let (app, visual) =
            cx.add_window_view(|window, cx| DesktopApp::new(services, window, cx));
        let mut harness = Self {
            app,
            visual: visual.clone(),
            preferences,
        };
        harness.resize(width, height);
        harness
    }

    /// Resizes the test window and lays the shell out again.
    ///
    /// @param width - Viewport width in logical pixels.
    /// @param height - Viewport height in logical pixels.
    pub fn resize(&mut self, width: f32, height: f32) {
        self.visual.simulate_resize(size(px(width), px(height)));
        self.visual.run_until_parked();
    }

    /// Bounds of one element exposed through `debug_selector`.
    ///
    /// @param selector - Selector passed to the element's `debug_selector`.
    /// @returns The painted bounds, or `None` when the element did not render.
    pub fn bounds(&mut self, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.visual.debug_bounds(selector)
    }

    /// Updates the shell entity, for direct state assertions.
    ///
    /// @param update - Closure receiving the shell and its context.
    /// @returns Whatever the closure returns.
    pub fn with<R>(
        &mut self,
        update: impl FnOnce(&mut DesktopApp, &mut gpui::Context<DesktopApp>) -> R,
    ) -> R {
        self.app.update(&mut self.visual, update)
    }

    /// Machine-readable rectangle report for the selectors that rendered.
    ///
    /// Values are rounded to one decimal so the report compares stably across
    /// runs; a selector that did not render is omitted rather than zeroed.
    ///
    /// @param selectors - Selectors to include, in report order.
    /// @returns One `selector x y width height` line per rendered element.
    pub fn rect_report(&mut self, selectors: &[&'static str]) -> String {
        let mut report = String::new();
        for selector in selectors {
            let Some(bounds) = self.bounds(selector) else {
                continue;
            };
            report.push_str(&format!(
                "{selector} {:.1} {:.1} {:.1} {:.1}\n",
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            ));
        }
        report
    }

    /// Compares a rectangle report against its committed baseline, or rewrites it
    /// when `UPDATE_SNAPSHOTS=1`.
    ///
    /// @param name - Baseline file name under `tests/snapshots`.
    /// @param actual - Report produced by [`Harness::rect_report`].
    /// @returns The baseline text now on disk.
    pub fn check_snapshot(name: &str, actual: &str) -> String {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
        let path = directory.join(name);
        if std::env::var("UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
            std::fs::create_dir_all(&directory).expect("snapshot directory");
            std::fs::write(&path, actual).expect("write snapshot");
            return actual.to_owned();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing snapshot {}: {error}; run UPDATE_SNAPSHOTS=1 cargo test",
                path.display()
            )
        });
        assert_eq!(
            actual.trim_end(),
            expected.trim_end(),
            "layout snapshot {} drifted; review the change, then rerun with UPDATE_SNAPSHOTS=1",
            path.display()
        );
        expected
    }

    /// Clicks the centre of one element.
    ///
    /// @param selector - Selector of the element to click.
    /// @returns Whether the element rendered and received the click.
    pub fn click(&mut self, selector: &'static str) -> bool {
        let Some(bounds) = self.bounds(selector) else {
            return false;
        };
        let centre = point(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
        );
        self.visual.simulate_click(centre, Modifiers::none());
        self.visual.run_until_parked();
        true
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.preferences);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_lays_out_the_v2_bars_and_panels_at_1366x768() {
        let mut harness = Harness::open(1366., 768.);
        let root = harness.bounds("shell-root").expect("shell root renders");
        assert_eq!(f32::from(root.size.width), 1366.);
        assert_eq!(f32::from(root.size.height), 768.);

        let top = harness.bounds("top-bar").expect("top bar renders");
        assert_eq!(f32::from(top.size.height), 30.);

        let bottom = harness.bounds("bottom-bar").expect("bottom bar renders");
        assert_eq!(f32::from(bottom.size.height), 26.);

        let explorer = harness.bounds("explorer-panel").expect("explorer docks at 1366");
        let agent = harness.bounds("agent-column").expect("agent column renders");
        let changes = harness.bounds("changes-panel").expect("changes docks at 1366");
        // Explorer is left of the conversation, the conversation is left of Changes.
        assert!(explorer.origin.x + explorer.size.width <= agent.origin.x);
        assert!(agent.origin.x + agent.size.width <= changes.origin.x);
        // The bars sandwich the body: none of the panels overlap them.
        assert!(top.origin.y + top.size.height <= agent.origin.y);
        assert!(agent.origin.y + agent.size.height <= bottom.origin.y + px(1.));
    }

    #[test]
    fn changes_toggle_widens_the_conversation_and_restores_it() {
        // `debug_bounds` keeps entries after an element stops rendering, so the
        // observable is the conversation width the layout actually produced.
        let mut harness = Harness::open(1366., 768.);
        let docked = f32::from(harness.bounds("agent-column").expect("agent column").size.width);
        assert!(harness.click("toggle-changes"));
        assert!(!harness.with(|app, _| app.show_changes));
        let widened = f32::from(harness.bounds("agent-column").expect("agent column").size.width);
        assert!(widened > docked + 250., "docked {docked}, widened {widened}");
        assert!(harness.click("toggle-changes"));
        assert!(harness.with(|app, _| app.show_changes));
        let restored = f32::from(harness.bounds("agent-column").expect("agent column").size.width);
        assert_eq!(restored, docked);
    }

    /// Selectors every desktop layout exposes, in paint order.
    const REPORT_SELECTORS: [&str; 6] = [
        "shell-root",
        "top-bar",
        "explorer-panel",
        "agent-column",
        "changes-panel",
        "bottom-bar",
    ];

    #[test]
    fn shell_layout_matches_the_recorded_baselines() {
        for (name, width, height) in [
            ("shell-1366x768.txt", 1366., 768.),
            ("shell-1920x1080.txt", 1920., 1080.),
        ] {
            let mut harness = Harness::open(width, height);
            let report = harness.rect_report(&REPORT_SELECTORS);
            assert!(report.contains("shell-root"), "{name}: {report}");
            Harness::check_snapshot(name, &report);
        }
    }

    #[test]
    fn explorer_dock_is_hidden_below_its_minimum_width() {
        let mut harness = Harness::open(1100., 800.);
        assert!(harness.bounds("explorer-panel").is_none());
        assert!(harness.bounds("agent-column").is_some());
        harness.resize(1400., 900.);
        assert!(harness.bounds("explorer-panel").is_some());
    }
}
