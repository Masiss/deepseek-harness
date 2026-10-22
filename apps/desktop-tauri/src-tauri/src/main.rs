//! Tauri shell for the DeepSeek Harness desktop application.
//!
//! The shell supervises the Desktop Host as a sidecar and hands its
//! authenticated Web URL to a system WebView.
//!
//! Closing the window never ends the process: the page and the Host keep
//! running so the next show resumes the same document. Something must stay
//! visible to say the application is still running, so the shell prefers the
//! tray and, where no StatusNotifier host can show it, explains the background
//! state once before hiding. Relaunching always surfaces the running window.

mod host;
mod locale;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use host::HostClient;

/// Whether the shell applies the light WebKit settings.
///
/// The light preset turns off the page cache and WebGL. The accelerated
/// compositing is not part of it: on a Wayland session it leaves the webview
/// painting into a window that stays blank, so it has a switch of its own.
///
/// @returns true when the environment asks for the light settings.
fn webkit_light() -> bool {
    match std::env::var("DSH_DESKTOP_WEBKIT_LIGHT") {
        Ok(value) => value != "0",
        Err(_) => false,
    }
}

/// Read a WebKit setting switch.
///
/// @param name - environment variable carrying the switch.
/// @param fallback - the value the master switch implies.
/// @returns true when the setting applies.
fn webkit_switch(name: &str, fallback: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => value != "0",
        Err(_) => fallback,
    }
}

/// Apply the WebKit settings a measurement asked for.
///
/// The application keeps the stock settings: the renderer that hands its
/// frames to the compositor without a copy, and the accelerated compositing
/// that puts them on the screen. Both alternatives cost more than they save —
/// the copy costs memory, and the missing compositing leaves a blank window.
#[cfg(target_os = "linux")]
fn configure_webkit(window: &tauri::WebviewWindow) {
    use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};
    if let Err(error) = window.with_webview(|webview| {
        let Some(settings) = webview.inner().settings() else {
            eprintln!("desktop shell: the webview has no settings");
            return;
        };
        // The light settings cost rendering speed and save memory. Each one
        // has its own switch so a measurement can attribute the saving, and
        // the accelerated compositing stays on unless a measurement asks for
        // it: the webview paints into a blank window without it.
        let light = webkit_light();
        if webkit_switch("DSH_DESKTOP_WEBKIT_ACCEL", false) {
            settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        }
        if webkit_switch("DSH_DESKTOP_WEBKIT_CACHE", light) {
            settings.set_enable_page_cache(false);
        }
        if webkit_switch("DSH_DESKTOP_WEBKIT_WEBGL", light) {
            settings.set_enable_webgl(false);
        }
        // Console messages carry whatever the page logs, so they are written
        // out only when an operator asks for them.
        if std::env::var("DSH_DESKTOP_WEB_CONSOLE").is_ok_and(|value| value == "1") {
            settings.set_enable_write_console_messages_to_stdout(true);
        }
    }) {
        eprintln!("desktop shell: webview settings: {error}");
    }
}

/// Remove the Host's session cookies from the webview's store.
///
/// The Host names its session cookie after the authority it serves, and every
/// Host listens on a port of its own, so each launch leaves a cookie behind
/// that the next one no longer accepts. The jar grows until the request
/// headers exceed what the server accepts, and then the application cannot
/// reach the Host at all. Only the cookies this Host family owns are removed,
/// and the exchange that follows mints a fresh one.
#[cfg(target_os = "linux")]
fn clear_session_cookies(window: &tauri::WebviewWindow) {
    use webkit2gtk::{CookieManagerExt, WebContextExt, WebViewExt};
    let _ = window.with_webview(|webview| {
        let view = webview.inner();
        let Some(context) = view.context() else { return };
        let Some(manager) = context.cookie_manager() else { return };
        let deleter = manager.clone();
        manager.cookies(
            "http://127.0.0.1/",
            None::<&webkit2gtk::gio::Cancellable>,
            move |result| {
                let Ok(mut cookies) = result else { return };
                for cookie in cookies.iter_mut() {
                    if cookie
                        .name()
                        .is_some_and(|name| name.starts_with(SESSION_COOKIE_PREFIX))
                    {
                        deleter.delete_cookie(cookie, None::<&webkit2gtk::gio::Cancellable>, |_| {});
                    }
                }
            },
        );
    });
}

/// Report the webview's own view of its session.
///
/// The page asks the Host the same question the shell asks, with the cookie
/// the browser holds, so the answer is the webview's own. A probe reads it
/// from the console output and can tell an authorized application from the
/// Host's unauthorized page.
fn report_webview_authentication(window: &tauri::WebviewWindow) {
    let _ = window.eval(
        "if (location.protocol === 'http:') { fetch('/__dsh-desktop/updates-status', { method: 'POST', headers: { 'content-type': 'application/json' }, body: 'null' }).then(function (response) { console.log('dsh: webview-auth=' + response.status); }).catch(function () { console.log('dsh: webview-auth=error'); }); }",
    );
}

/// Save a picture of what the webview painted.
///
/// The snapshot comes from the webview itself, so a measurement can compare
/// two renderings without a compositor that could show a window on the user's
/// session.
///
/// @param window - window whose content is captured.
/// @param path - file the picture is written to.
#[cfg(target_os = "linux")]
fn save_webview_snapshot(window: &tauri::WebviewWindow, path: &str) {
    use webkit2gtk::WebViewExt;
    let path = path.to_owned();
    let _ = window.with_webview(move |webview| {
        webview.inner().snapshot(
            webkit2gtk::SnapshotRegion::Visible,
            webkit2gtk::SnapshotOptions::NONE,
            None::<&webkit2gtk::gio::Cancellable>,
            move |result: Result<cairo::Surface, webkit2gtk::glib::Error>| {
                if let Ok(surface) = result {
                    if let Ok(mut file) = std::fs::File::create(&path) {
                        let _ = surface.write_to_png(&mut file);
                    }
                }
            },
        );
    });
}

/// Measure how fast the webview paints.
///
/// The page counts the frames it receives over two seconds, so a measurement
/// can tell a setting that costs smoothness from one that does not.
///
/// @param window - window whose page is measured.
fn report_webview_frame_rate(window: &tauri::WebviewWindow) {
    let _ = window.eval(
        "if (location.protocol === 'http:') { var frames = 0; var started = performance.now(); function tick() { frames += 1; var elapsed = performance.now() - started; if (elapsed < 2000) { requestAnimationFrame(tick); } else { var resources = performance.getEntriesByType('resource'); var bytes = 0; resources.forEach(function (entry) { bytes += entry.transferSize || 0; }); console.log('dsh: webview-fps=' + Math.round(frames / (elapsed / 1000))); console.log('dsh: webview-resources=' + resources.length + ' bytes=' + bytes); } } requestAnimationFrame(tick); }",
    );
}

/// Product name used by shell-owned copy: window title, tray tooltip, dialogs.
const PRODUCT: &str = "DeepSeek Harness";

/// Prefix of the cookie the Host keeps its browser session in.
const SESSION_COOKIE_PREFIX: &str = "dsh-auth-";

/// Interface name of the StatusNotifier watcher a tray icon registers with.
#[cfg(target_os = "linux")]
const STATUS_NOTIFIER_WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Set once a quit has been approved, so close requests stop hiding the window.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Start of the shell's own run, the reference every startup milestone reports.
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

/// How long the shell waits for the Host to report its URL.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Tell the user that the Host cannot serve, once per failure.
///
/// The window keeps showing the start page, so a silent failure would look
/// like a hung application.
fn report_host_failure(app: &AppHandle, reported: &AtomicBool, reason: String) {
    if reported.swap(true, Ordering::SeqCst) {
        return;
    }
    eprintln!("desktop shell: {reason}");
    let handle = app.clone();
    app.dialog()
        .message(reason)
        .title(locale::text("startupFailed"))
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .show(move |_| {
            let _ = &handle;
        });
}

/// Report one startup milestone as a structured line.
///
/// The shell has no session log of its own, so the startup timings travel as
/// NDJSON on stderr: a probe reads the fields instead of parsing prose.
/// @param event - Milestone name.
fn report_startup(event: &str) {
    let payload = serde_json::json!({ "event": event, "ms": STARTED.elapsed().as_millis() });
    eprintln!("{payload}");
}

/// Whether a tray icon is registered and visible for this session.
static TRAY_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Application state shared by window, tray, and exit handlers.
struct AppState {
    host: Arc<HostClient>,
}

/// Resolve the Host command from the environment.
///
/// The positional contract is shared with the Electron shell: runtime
/// directory, project directory, primary runtime, pnpm entry, Node launcher
/// directory. The package manager occupies the last two slots, so the primary
/// runtime slot stays positional even though this entry does not read it.
///
/// The runtime and project directories are optional together — a development
/// shell may run the Host with none. The package-manager triple is
/// all-or-nothing: a partial triple would silently leave `dsh plugin` unable to
/// install anything, which is the failure this contract exists to prevent.
///
/// @returns the executable and its positional arguments.
/// @throws when the package-manager variables are only partly set.
fn host_command() -> Result<(String, Vec<String>), String> {
    let binary = std::env::var("DSH_DESKTOP_HOST_BIN").unwrap_or_else(|_| "dsh-host".to_owned());
    let mut args = Vec::new();
    if let (Ok(runtime), Ok(project)) = (
        std::env::var("DSH_DESKTOP_RUNTIME_DIR"),
        std::env::var("DSH_DESKTOP_PROJECT_DIR"),
    ) {
        args.push(runtime);
        args.push(project);
    }

    let primary = std::env::var("DSH_DESKTOP_PRIMARY_RUNTIME").ok();
    let pnpm = std::env::var("DSH_DESKTOP_PNPM_ENTRY").ok();
    let node_bin = std::env::var("DSH_DESKTOP_NODE_BIN").ok();
    match (primary, pnpm, node_bin) {
        (None, None, None) => {}
        (Some(primary), Some(pnpm), Some(node_bin)) => {
            args.push(primary);
            args.push(pnpm);
            args.push(node_bin);
        }
        _ => {
            return Err(
                "set DSH_DESKTOP_PRIMARY_RUNTIME, DSH_DESKTOP_PNPM_ENTRY and DSH_DESKTOP_NODE_BIN together or not at all"
                    .to_owned(),
            )
        }
    }
    Ok((binary, args))
}

/// Reveal and focus the main window.
///
/// @param app - handle used to look the window up by label.
fn show_main(app: &AppHandle) {
    if let Some(window) = application_window(app) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hide the main window, leaving the process and the Host running.
///
/// @param app - handle that owns the window.
fn hide_main(app: &AppHandle) {
    if let Some(window) = application_window(app) {
        let _ = window.hide();
    }
}

/// The window the user interacts with: the application, or the start page
/// while the Host is still booting.
///
/// @param app - handle that owns the windows.
/// @returns The application window, or the start page before it exists.
fn application_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main")
}

/// Whether a StatusNotifier host can actually display a tray icon.
///
/// Tauri's tray builder succeeds even where nothing will render it, so the
/// shell asks the session bus instead of trusting the build result.
#[cfg(target_os = "linux")]
fn status_notifier_host_present() -> bool {
    use zbus::blocking::Connection;
    let Ok(connection) = Connection::session() else { return false };
    let reply = connection.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(STATUS_NOTIFIER_WATCHER,),
    );
    match reply {
        Ok(message) => message.body().deserialize::<bool>().unwrap_or(false),
        Err(_) => false,
    }
}

/// Non-Linux desktops render the tray through their own shell notification area.
#[cfg(not(target_os = "linux"))]
fn status_notifier_host_present() -> bool {
    true
}

/// Create the tray icon and report whether it is a usable way back.
///
/// @param app - handle that owns the menu and tray lifetime.
/// @returns `true` when the icon is expected to be visible.
fn build_tray(app: &AppHandle) -> bool {
    if std::env::var("DSH_DESKTOP_TRAY").is_ok_and(|value| value == "0") {
        return false;
    }
    if !status_notifier_host_present() {
        eprintln!("desktop tray: no StatusNotifier host on this session");
        return false;
    }

    let built = (|| -> Result<(), Box<dyn std::error::Error>> {
        let open = MenuItem::with_id(app, "open", "Open DeepSeek Harness", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", format!("Quit {PRODUCT}"), true, None::<&str>)?;
        let separator = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&open, &separator, &quit])?;

        let mut builder = TrayIconBuilder::with_id("main")
            .menu(&menu)
            .tooltip(PRODUCT)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "open" => show_main(app),
                "quit" => request_quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main(tray.app_handle());
                }
            });
        if let Some(icon) = app.default_window_icon().cloned() {
            builder = builder.icon(icon);
        }
        builder.build(app)?;
        Ok(())
    })();
    match built {
        Ok(()) => true,
        Err(error) => {
            // A desktop without an appindicator host still runs; only the tray is missing.
            eprintln!("desktop tray unavailable: {error}");
            false
        }
    }
}

/// Acknowledgement marker for the background-close explanation.
///
/// @param app - handle that owns the per-user configuration directory.
/// @returns the marker path, or `None` when no writable directory is known.
fn background_marker(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|directory| directory.join("background-close-confirmed"))
}

/// Hide the window, explaining the background state first when no tray is visible.
///
/// A tray is the standing reminder that the application is still running. Without
/// one, the first close explains where the window went and how to get it back;
/// cancelling keeps the window open rather than hiding it unexplained.
///
/// @param app - handle that owns the dialog and the window.
fn close_to_background(app: &AppHandle) {
    if TRAY_VISIBLE.load(Ordering::SeqCst) {
        hide_main(app);
        return;
    }
    let marker = background_marker(app);
    if marker.as_ref().is_some_and(|path| path.exists()) {
        hide_main(app);
        return;
    }

    let handle = app.clone();
    app.dialog()
        .message(format!(
            "{PRODUCT} keeps running in the background so your session stays open. Launch it again to bring the window back."
        ))
        .title(PRODUCT)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom("Continue".to_owned(), "Cancel".to_owned()))
        .show(move |confirmed| {
            if !confirmed {
                return;
            }
            if let Some(path) = marker {
                if let Some(directory) = path.parent() {
                    let _ = std::fs::create_dir_all(directory);
                }
                let _ = std::fs::write(&path, []);
            }
            hide_main(&handle);
        });
}

/// Whether the Host reports interruptible work.
///
/// The Host answers with two flags: running work and armed scheduled
/// reminders. Either one means quitting interrupts something the user can
/// still see, so both are asked about. An unreachable Host counts as active:
/// the shell has no proof that a quit is safe, so it asks instead of
/// discarding work.
fn host_has_active_tasks(host: &HostClient) -> bool {
    match host.request("quit-inspection") {
        Ok(frame) => {
            let active = frame.get("activeTasks").and_then(Value::as_bool).unwrap_or(true);
            let scheduled = frame.get("scheduledTasks").and_then(Value::as_bool).unwrap_or(true);
            active || scheduled
        }
        Err(error) => {
            eprintln!("desktop quit: {error}");
            true
        }
    }
}

/// Ask before quitting when work is running, then let the exit handler stop the Host.
///
/// @param app - handle that owns the Host state and the confirmation dialog.
fn request_quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        app.exit(0);
        return;
    };
    // The Host answers on its own thread, and it may take a moment to answer.
    // Asking it on the event loop would freeze the window and the menu for as
    // long as the Host takes, so the question goes to a worker thread and the
    // answer comes back to the event loop.
    let host = state.host.clone();
    let handle = app.clone();
    std::thread::spawn(move || {
        let active = host_has_active_tasks(&host);
        let answer = handle.clone();
        let inside = handle.clone();
        let _ = answer.run_on_main_thread(move || finish_quit(&inside, active));
    });
}

/// Leave the application once the running work is known.
///
/// @param app - handle that owns the application.
/// @param active - whether work is still running.
fn finish_quit(app: &AppHandle, active: bool) {
    if !active {
        app.exit(0);
        return;
    }

    let handle = app.clone();
    app.dialog()
        .message(locale::text("quitActiveTasks"))
        .title(locale::text("quitTitle"))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            locale::text("quit").to_owned(),
            locale::text("cancel").to_owned(),
        ))
        .show(move |confirmed| {
            if confirmed {
                handle.exit(0);
            } else {
                QUITTING.store(false, Ordering::SeqCst);
                show_main(&handle);
            }
        });
}

/// Show the About dialog.
///
/// @param app - handle that owns the dialog and the package metadata.
fn show_about(app: &AppHandle) {
    let version = std::env::var("DSH_CLIENT_VERSION")
        .unwrap_or_else(|_| app.package_info().version.to_string());
    app.dialog()
        .message(format!("Version {version}"))
        .title(PRODUCT)
        .kind(MessageDialogKind::Info)
        .show(|_| {});
}

/// Whether this build exposes the development entries (reload, developer tools).
fn development() -> bool {
    cfg!(debug_assertions) || std::env::var("DSH_DESKTOP_DEVELOPMENT").is_ok_and(|value| value == "1")
}

/// Build the window menu bar: the application menu and the edit roles.
///
/// @param app - handle the menu belongs to.
/// @returns the menu, or an error when a predefined item cannot be created.
fn build_app_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let about = MenuItem::with_id(app, "about", format!("About {PRODUCT}"), true, None::<&str>)?;
    let quit = MenuItem::with_id(
        app,
        "quit-application",
        format!("Quit {PRODUCT}"),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let first_break = PredefinedMenuItem::separator(app)?;
    let application = if development() {
        let reload = MenuItem::with_id(app, "reload-page", "Reload Page", true, None::<&str>)?;
        let devtools = MenuItem::with_id(
            app,
            "toggle-devtools",
            "Toggle Developer Tools",
            true,
            Some("F12"),
        )?;
        let second_break = PredefinedMenuItem::separator(app)?;
        Submenu::with_items(
            app,
            "Application",
            true,
            &[&about, &first_break, &reload, &devtools, &second_break, &quit],
        )?
    } else {
        Submenu::with_items(app, "Application", true, &[&about, &first_break, &quit])?
    };

    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    Menu::with_items(app, &[&application, &edit])
}

/// Run one menu command that needs the window.
///
/// @param app - handle that owns the window.
/// @param command - menu item id.
fn run_menu_command(app: &AppHandle, command: &str) {
    let Some(window) = application_window(app) else { return };
    match command {
        // The page owns the session; reloading only re-fetches the shell the Host serves.
        "reload-page" => {
            let _ = window.eval("location.reload()");
        }
        "toggle-devtools" => {
            toggle_devtools(&window);
        }
        _ => {}
    }
}

/// Toggle WebView developer tools when this build carries them.
///
/// The menu item exists only in development builds, and debug builds carry the
/// inspector; release builds compile the call out because the Tauri `devtools`
/// feature is not enabled.
///
/// @param window - window whose devtools are toggled.
fn toggle_devtools(window: &tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    {
        if window.is_devtools_open() {
            window.close_devtools();
        } else {
            window.open_devtools();
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = window;
    }
}

/// Let a measurement turn the WebKit web process's dmabuf renderer off.
///
/// The renderer hands the frames to the compositor without a copy. Turning it
/// off makes the shell copy every frame, which costs far more memory on a
/// machine with a graphics device than the renderer saves, so it stays on
/// unless a measurement asks for the copy. The variable is read when the web
/// process starts, so it is set before the window exists.
fn tune_webkit_process_environment() {
    if !webkit_switch("DSH_DESKTOP_WEBKIT_DMABUF", false) {
        return;
    }
    // SAFETY: the shell is single-threaded at this point, and no other thread
    // reads the environment.
    unsafe {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

fn main() {
    // The startup milestones measure the shell's own run, so the reference
    // starts here rather than at the first report.
    LazyLock::force(&STARTED);
    tune_webkit_process_environment();
    tauri::Builder::default()
        // Registered first so a second launch surfaces the first instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|argument| argument.starts_with("dsh://")) {
                // A `dsh://` launch asks to surface the running application;
                // packaging registers the scheme and P3 routes the target.
            }
            show_main(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .on_menu_event(|app, event| match event.id().as_ref() {
            "about" => show_about(app),
            "quit-application" => request_quit(app),
            command => run_menu_command(app, command),
        })
        .setup(|app| {
            let (binary, args) = host_command()?;

            // The Host's URL, kept until the window has exchanged the token.
            let token_url: Arc<Mutex<Option<tauri::Url>>> = Arc::new(Mutex::new(None));
            // One report per failure, so the shell does not repeat itself.
            let reported = Arc::new(AtomicBool::new(false));
            // The window shows the bundled start page while the Host boots.
            // The Host's URL arrives later and replaces it.
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App(PathBuf::from("index.html")))
                .on_page_load({
                    let token_url = Arc::clone(&token_url);
                    let retried = Arc::new(AtomicBool::new(false));
                    let reported = Arc::clone(&reported);
                    move |window, payload| {
                        report_startup(match payload.event() {
                            tauri::webview::PageLoadEvent::Started => "page-load-started",
                            tauri::webview::PageLoadEvent::Finished => "page-load-finished",
                        });
                        if payload.event() != tauri::webview::PageLoadEvent::Finished {
                            return;
                        }
                        report_webview_authentication(&window);
                        report_webview_frame_rate(&window);
                        if let Ok(path) = std::env::var("DSH_DESKTOP_WEBVIEW_SNAPSHOT") {
                            #[cfg(target_os = "linux")]
                            save_webview_snapshot(&window, &path);
                        }
                        // The Host answers the token request with the
                        // application, but the session cookie it mints belongs
                        // to the Host's origin, and a browser only keeps it
                        // for a document of that origin. The first request
                        // therefore reaches the Host from the start page's
                        // site and leaves the application unauthorized.
                        // Repeating the request from the Host's own document
                        // completes the exchange, and the marker keeps it to
                        // one repetition.
                        let loaded = payload.url().clone();
                        let Ok(slot) = token_url.lock() else { return };
                        let Some(target) = slot.clone() else { return };
                        // Only the Host's own origin may receive the token.
                        if loaded.origin() != target.origin() {
                            return;
                        }
                        let carries_token = loaded
                            .query()
                            .is_some_and(|query| query.contains("token="));
                        if retried.swap(true, Ordering::SeqCst) {
                            // The repetition did not complete the exchange
                            // either, so the application has no session.
                            if carries_token {
                                let app = window.app_handle().clone();
                                let reported = Arc::clone(&reported);
                                report_host_failure(
                                    &app,
                                    &reported,
                                    "the Host did not accept the session token".to_owned(),
                                );
                            }
                            return;
                        }
                        if carries_token {
                            // The exchange did not complete, and the
                            // document is the Host's own, so repeating the
                            // request from here completes it.
                            let _ = window.navigate(target);
                            return;
                        }
                        let _ = window.navigate(target);
                    }
                })
                .title(PRODUCT)
                .inner_size(1280.0, 860.0)
                .min_inner_size(520.0, 600.0)
                .theme(None)
                .build()?;
            #[cfg(target_os = "linux")]
            {
                configure_webkit(&window);
                clear_session_cookies(&window);
            }
            report_startup("window-created");

            let handle = app.handle().clone();
            let failure_handle = handle.clone();
            let failure_reported = Arc::clone(&reported);
            let ready_window = window.clone();
            let token_url = Arc::clone(&token_url);
            let host = HostClient::spawn(
                &binary,
                &args,
                move |url| {
                    report_startup("host-url");
                    let Ok(target) = tauri::Url::parse(&url) else {
                        eprintln!("desktop shell: the Host URL is not a URL");
                        return;
                    };
                    if let Ok(mut slot) = token_url.lock() {
                        *slot = Some(target.clone());
                    }
                    if let Err(error) = ready_window.navigate(target) {
                        eprintln!("desktop shell: navigation to the Host failed: {error}");
                    }
                },
                move |reason| {
                    report_host_failure(&failure_handle, &failure_reported, reason);
                },
            )?;

            // A Host that never answers must not leave the window on the start
            // page forever.
            let watchdog_host = Arc::clone(&host);
            let watchdog_handle = handle.clone();
            let watchdog_reported = Arc::clone(&reported);
            std::thread::spawn(move || {
                std::thread::sleep(STARTUP_TIMEOUT);
                if !watchdog_host.ready() {
                    report_host_failure(
                        &watchdog_handle,
                        &watchdog_reported,
                        "the Host did not become ready in time".to_owned(),
                    );
                }
            });
            host.attach(handle.clone());
            app.manage(AppState { host });

            let hide_target = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if QUITTING.load(Ordering::SeqCst) {
                        return;
                    }
                    api.prevent_close();
                    close_to_background(hide_target.app_handle());
                }
            });

            // The tray is reported before the menu so the first close already knows
            // whether something stays visible.
            TRAY_VISIBLE.store(build_tray(app.handle()), Ordering::SeqCst);
            if let Ok(menu) = build_app_menu(app.handle()) {
                let _ = app.set_menu(menu);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the desktop shell")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.host.shutdown();
                }
            }
        });
}
