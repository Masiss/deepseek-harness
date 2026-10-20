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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use host::HostClient;

/// Product name used by shell-owned copy: window title, tray tooltip, dialogs.
const PRODUCT: &str = "DeepSeek Harness";

/// Interface name of the StatusNotifier watcher a tray icon registers with.
#[cfg(target_os = "linux")]
const STATUS_NOTIFIER_WATCHER: &str = "org.kde.StatusNotifierWatcher";

/// Set once a quit has been approved, so close requests stop hiding the window.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Start of the shell's own run, the reference every startup milestone reports.
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

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
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hide the main window, leaving the process and the Host running.
///
/// @param app - handle that owns the window.
fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
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
fn host_has_active_tasks(state: &AppState) -> bool {
    match state.host.request("quit-inspection") {
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
    if !host_has_active_tasks(&state) {
        app.exit(0);
        return;
    }

    let handle = app.clone();
    app.dialog()
        .message("Tasks are still running. Quit DeepSeek Harness anyway?")
        .title(format!("Quit {PRODUCT}"))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Quit".to_owned(),
            "Cancel".to_owned(),
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
    let Some(window) = app.get_webview_window("main") else { return };
    match command {
        // The page owns the session; reloading only re-fetches the shell the Host serves.
        "reload-page" => {
            let _ = window.eval("location.reload()");
        }
        "toggle-devtools" => {
            if window.is_devtools_open() {
                window.close_devtools();
            } else {
                window.open_devtools();
            }
        }
        _ => {}
    }
}

fn main() {
    // The startup milestones measure the shell's own run, so the reference
    // starts here rather than at the first report.
    LazyLock::force(&STARTED);
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

            // The window shows the bundled start page first, so the application
            // is on screen while the Host boots. The Host's URL arrives later
            // and replaces the start page.
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App(PathBuf::from("index.html")))
                .on_page_load(move |_, payload| {
                    report_startup(match payload.event() {
                        tauri::webview::PageLoadEvent::Started => "page-load-started",
                        tauri::webview::PageLoadEvent::Finished => "page-load-finished",
                    });
                })
                .title(PRODUCT)
                .inner_size(1280.0, 860.0)
                .min_inner_size(520.0, 600.0)
                .theme(None)
                .build()?;
            report_startup("window-created");

            let handle = app.handle().clone();
            let splash = window.clone();
            let host = HostClient::spawn(&binary, &args, move |url| {
                report_startup("host-url");
                match tauri::Url::parse(&url) {
                    Ok(target) => {
                        if let Err(error) = splash.navigate(target) {
                            eprintln!("desktop shell: navigation to the Host failed: {error}");
                        }
                    }
                    Err(error) => eprintln!("desktop shell: the Host URL is not a URL: {error}"),
                }
            })?;
            host.attach(handle.clone());
            app.manage(AppState { host });

            let window = window.clone();
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
