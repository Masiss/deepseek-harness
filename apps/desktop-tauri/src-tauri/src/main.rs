//! Tauri shell for the DeepSeek Harness desktop application.
//!
//! The shell supervises the Desktop Host as a sidecar and hands its
//! authenticated Web URL to a system WebView. Closing the window hides it: the
//! page and the Host keep running, and the next show resumes the same document.
//! Quitting asks the Host whether work is still running.

mod host;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use host::HostClient;

/// Set once a quit has been approved, so close requests stop hiding the window.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Application state shared by window, tray, and exit handlers.
struct AppState {
    host: Arc<HostClient>,
}

/// Resolve the Host command from the environment.
///
/// @returns the executable and its positional arguments.
fn host_command() -> (String, Vec<String>) {
    let binary = std::env::var("DSH_DESKTOP_HOST_BIN").unwrap_or_else(|_| "dsh-host".to_owned());
    let mut args = Vec::new();
    if let (Ok(runtime), Ok(project)) = (
        std::env::var("DSH_DESKTOP_RUNTIME_DIR"),
        std::env::var("DSH_DESKTOP_PROJECT_DIR"),
    ) {
        args.push(runtime);
        args.push(project);
    }
    (binary, args)
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

/// Whether the Host reports interruptible work.
///
/// An unreachable Host counts as active: the shell has no proof that a quit is
/// safe, so it asks instead of discarding work.
fn host_has_active_tasks(state: &AppState) -> bool {
    match state.host.request("quit-inspection") {
        Ok(frame) => frame
            .get("activeTasks")
            .and_then(Value::as_bool)
            .unwrap_or(true),
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
        .title("Quit DeepSeek Harness")
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

/// Create the tray icon, degrading to no tray when the platform rejects it.
///
/// @param app - handle that owns the menu and tray lifetime.
fn build_tray(app: &AppHandle) {
    let build = || -> Result<(), Box<dyn std::error::Error>> {
        let show = MenuItem::with_id(app, "show", "Show DeepSeek Harness", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let separator = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&show, &separator, &quit])?;

        let mut builder = TrayIconBuilder::with_id("main")
            .menu(&menu)
            .tooltip("DeepSeek Harness")
            .on_menu_event(|app, event| match event.id().as_ref() {
                "show" => show_main(app),
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
    };
    if let Err(error) = build() {
        // A desktop without an appindicator host still runs; only the tray is missing.
        eprintln!("desktop tray unavailable: {error}");
    }
}

fn main() {
    tauri::Builder::default()
        // Registered first so a second launch focuses the first instance and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
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
        .setup(|app| {
            let (binary, args) = host_command();
            let (host, url) = HostClient::spawn(&binary, &args)?;
            app.manage(AppState { host });

            let target = tauri::Url::parse(&url)?;
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(target))
                .title("DeepSeek Harness")
                .inner_size(1280.0, 860.0)
                .build()?;
            let hide_target = window.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if QUITTING.load(Ordering::SeqCst) {
                        return;
                    }
                    // Closing hides: the page and the Host keep running, and the
                    // next show resumes the same document.
                    api.prevent_close();
                    let _ = hide_target.hide();
                }
            });

            build_tray(app.handle());
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
