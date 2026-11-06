//! Native gpui desktop shell: one GPU-rendered window over the Host API.
//!
//! The shared protocol, session projection, and preference modules are the
//! same ones the retired egui shell used; only the presentation layer moved.

#[path = "command_registry.rs"]
mod command_registry;
#[path = "composer_input.rs"]
mod composer_input;
#[path = "dialog.rs"]
mod dialog;
#[path = "document_preview.rs"]
mod document_preview;
#[path = "explorer.rs"]
mod explorer;
#[path = "fonts.rs"]
mod fonts;
#[path = "handoff.rs"]
mod handoff;
#[path = "host.rs"]
mod host;
#[path = "layout.rs"]
mod layout;
#[path = "question_card.rs"]
mod question_card;
#[path = "restart.rs"]
mod restart;
#[path = "session_list.rs"]
mod session_list;
#[path = "terminal_ansi.rs"]
mod terminal_ansi;
#[path = "terminal_input.rs"]
mod terminal_input;
#[path = "transcript.rs"]
mod transcript;
#[path = "ui_preferences.rs"]
mod ui_preferences;
#[path = "updater.rs"]
mod updater;

mod shell;

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};
use ui_preferences::UiPreferences;

fn lock_path() -> PathBuf {
    let home = std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".deepseek-harness")))
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".native-app.lock")
}

fn acquire_instance_lock() -> Option<File> {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("single-instance directory creation failed: {error}");
            return None;
        }
    }
    let file = match OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("single-instance lock open failed: {error}");
            return None;
        }
    };
    if let Err(error) = file.try_lock_exclusive() {
        eprintln!("native app is already running: {error}");
        return None;
    }
    Some(file)
}

fn setup_tray() -> Option<(
    tray_icon::TrayIcon,
    tray_icon::menu::MenuId,
    tray_icon::menu::MenuId,
)> {
    let menu = Menu::new();
    let show = MenuItem::new("Show/Hide", true, None);
    let quit = MenuItem::new("Quit", true, None);
    if let Err(error) = menu.append(&show) {
        eprintln!("tray menu setup failed: {error}");
        return None;
    }
    if let Err(error) = menu.append(&quit) {
        eprintln!("tray menu setup failed: {error}");
        return None;
    }
    let icon_pixels =
        match image::load_from_memory(include_bytes!("../../src-tauri/icons/icon.png")) {
            Ok(image) => image
                .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
                .into_rgba8()
                .into_raw(),
            Err(error) => {
                eprintln!("tray icon decode failed: {error}");
                return None;
            }
        };
    let icon = match Icon::from_rgba(icon_pixels, 32, 32) {
        Ok(icon) => icon,
        Err(error) => {
            eprintln!("tray icon setup failed: {error}");
            return None;
        }
    };
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    match TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("DeepSeek Harness")
        .with_icon(icon)
        .build()
    {
        Ok(tray) => Some((tray, show_id, quit_id)),
        Err(error) => {
            eprintln!("tray setup failed: {error}");
            None
        }
    }
}

fn main() {
    let Some(_instance_lock) = acquire_instance_lock() else {
        return;
    };
    std::thread::spawn(updater::check);

    let quitting = Arc::new(AtomicBool::new(false));
    let tray = setup_tray();
    let tray_available = tray.is_some();
    let (show_id, quit_id) = tray.as_ref().map_or_else(
        || (None, None),
        |(_, show, quit)| (Some(show.clone()), Some(quit.clone())),
    );
    let show_requested = Arc::new(AtomicBool::new(false));
    let hide_requested = Arc::new(AtomicBool::new(false));
    let hidden = Arc::new(AtomicBool::new(false));

    {
        let event_quitting = Arc::clone(&quitting);
        let event_show = Arc::clone(&show_requested);
        let event_hide = Arc::clone(&hide_requested);
        let event_hidden = Arc::clone(&hidden);
        MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            if Some(event.id.clone()) == quit_id {
                event_quitting.store(true, Ordering::Release);
            } else if Some(event.id) == show_id {
                let currently_hidden = event_hidden.load(Ordering::Acquire);
                if currently_hidden {
                    event_show.store(true, Ordering::Release);
                } else {
                    event_hide.store(true, Ordering::Release);
                }
            }
        }));
    }
    {
        let tray_show = Arc::clone(&show_requested);
        TrayIconEvent::set_event_handler(Some(move |event| {
            if let TrayIconEvent::Click { .. } = event {
                tray_show.store(true, Ordering::Release);
            }
        }));
    }

    shell::run(shell::ShellServices {
        quitting,
        show_requested,
        hide_requested,
        hidden,
        tray_available,
        ui_preferences_path: UiPreferences::config_path(),
        spawn_host: true,
        commands: Default::default(),
    });
}
