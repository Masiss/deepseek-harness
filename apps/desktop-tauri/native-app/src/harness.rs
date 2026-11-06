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
        let mut harness = Self::build(width, height, false);
        harness.resize(width, height);
        harness
    }

    /// Opens the shell after running `setup` against its state, then draws once.
    ///
    /// The fresh window gives `debug_bounds` a clean map, so a selector that is
    /// absent here really did not render in this state.
    ///
    /// @param width - Viewport width in logical pixels.
    /// @param height - Viewport height in logical pixels.
    /// @param setup - State change applied before the first frame.
    /// @returns The harness laid out in that state.
    pub fn open_with(
        width: f32,
        height: f32,
        setup: impl FnOnce(&mut DesktopApp),
    ) -> Self {
        let mut harness = Self::build(width, height, false);
        harness.with(|app, _| setup(app));
        harness.resize(width, height);
        harness
    }

    /// Builds the harness with or without the Host launch.
    fn build(width: f32, height: f32, spawn_host: bool) -> Self {
        let _ = (width, height);
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
            spawn_host,
            commands: Arc::new(Mutex::new(Vec::<CommandId>::new())),
        };
        let mut cx = TestAppContext::single();
        let (app, visual) =
            cx.add_window_view(|window, cx| DesktopApp::new(services, window, cx));
        Self {
            app,
            visual: visual.clone(),
            preferences,
        }
    }

    /// Opens the shell with the Desktop Host launch enabled.
    ///
    /// The Host command comes from the `DSH_DESKTOP_HOST_BIN` family of launch
    /// variables, exactly as the shipped binary reads them, so a packaged tree
    /// can drive this path.
    ///
    /// @param width - Viewport width in logical pixels.
    /// @param height - Viewport height in logical pixels.
    /// @returns The harness with no Host connection yet.
    pub fn open_with_host(width: f32, height: f32) -> Self {
        let mut harness = Self::build(width, height, true);
        harness.resize(width, height);
        harness
    }

    /// Drives the shell's poll loop until `condition` holds or the timeout expires.
    ///
    /// Real worker threads (Host launch, HTTP, streams) need wall-clock time, so
    /// this pumps frames and sleeps instead of only advancing the test clock.
    ///
    /// @param label - Short description used in the failure message.
    /// @param timeout - Longest wait before giving up.
    /// @param condition - Predicate evaluated against the shell state.
    /// @returns Whether the condition held.
    pub fn pump_until(
        &mut self,
        label: &str,
        timeout: std::time::Duration,
        condition: impl Fn(&DesktopApp) -> bool,
    ) -> bool {
        let started = std::time::Instant::now();
        loop {
            if self.with(|app, _| condition(app)) {
                return true;
            }
            if started.elapsed() > timeout {
                eprintln!("harness: timed out waiting for {label}");
                return false;
            }
            self.visual.update(|window, cx| {
                self.app
                    .update(cx, |app, cx| app.tick(window, cx))
            });
            self.visual.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
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

/// Packaged-Host journey: the shell connects to a real Desktop Host and creates
/// a Session through its own RPC path.
///
/// Ignored because it needs the packaged tree; `native-journey-probe.sh shell`
/// supplies the launch variables.
#[test]
#[ignore = "needs the packaged Host; run scripts/native-journey-probe.sh shell"]
fn packaged_shell_connects_and_creates_a_session() {
    use std::time::Duration;

    assert!(
        std::env::var("DSH_DESKTOP_HOST_BIN").is_ok(),
        "DSH_DESKTOP_HOST_BIN is unset; run scripts/native-journey-probe.sh shell"
    );
    let mut harness = Harness::open_with_host(1440., 900.);
    assert!(
        harness.pump_until("Host connection", Duration::from_secs(180), |app| {
            app.transport.is_some()
        }),
        "the shell never connected to the packaged Host"
    );
    // Let the first Session list settle: a create request is dropped while a
    // list request is already in flight.
    assert!(
        harness.pump_until("Session list idle", Duration::from_secs(120), |app| {
            !app.sessions_request_pending()
        }),
        "the shell never finished its first Session list"
    );
    harness.with(|app, _| app.new_session());
    let created = harness.pump_until("Session creation", Duration::from_secs(120), |app| {
        app.selected_session.is_some()
    });
    if !created {
        let (status, error, page_error) = harness.with(|app, _| {
            (
                app.sessions_status.clone(),
                app.sessions_error.clone(),
                app.page_error.clone(),
            )
        });
        panic!("the shell never selected the created Session: {status} / {error:?} / {page_error:?}");
    }
    let session = harness
        .with(|app, _| app.selected_session.clone())
        .expect("selected Session");
    assert!(
        harness.pump_until("Session list", Duration::from_secs(60), |app| {
            app.sessions
                .iter()
                .any(|summary| summary.session_id == session)
        }),
        "the created Session never appeared in the shell's list"
    );
    assert!(
        harness.bounds("agent-column").is_some(),
        "the agent column did not render for the packaged Session"
    );
    println!("RESULT packaged-shell=OK session={session}");
}

#[cfg(test)]
mod surface_tests {
    use super::*;
    use crate::command_registry::CommandId;

    #[test]
    fn settings_page_replaces_the_conversation_and_both_docks() {
        let mut harness = Harness::open_with(1440., 900., |app| {
            app.run_command(CommandId::ToggleSettings)
        });
        assert!(harness.bounds("shell-root").is_some());
        assert!(harness.bounds("top-bar").is_some());
        // A page owns the body: both docks come from the agent layout only.
        assert!(harness.bounds("explorer-panel").is_none());
        assert!(harness.bounds("changes-panel").is_none());
        let body = harness.bounds("agent-column").expect("body renders");
        assert_eq!(f32::from(body.size.width), 1440.);
    }

    #[test]
    fn command_palette_keeps_the_shell_laid_out_and_closes_on_dismiss() {
        let mut harness = Harness::open_with(1440., 900., |app| {
            app.run_command(CommandId::ShowPalette)
        });
        assert!(harness.with(|app, _| app.palette_open));
        let root = harness.bounds("shell-root").expect("root stays laid out");
        assert_eq!(f32::from(root.size.height), 900.);
        assert!(harness.bounds("agent-column").is_some());
        assert!(harness.with(|app, _| app.dismiss_overlay()));
        assert!(!harness.with(|app, _| app.palette_open));
    }
}
