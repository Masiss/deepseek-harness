//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

mod dialog;
mod host;
mod updater;

use dsh_client_core::{session::SessionViewer, transport::HttpTransport};
use eframe::egui;
use fs2::FileExt;
use host::Host;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

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
    let file = match OpenOptions::new().create(true).write(true).open(path) {
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

fn setup_tray() -> Option<(tray_icon::TrayIcon, tray_icon::menu::MenuId, tray_icon::menu::MenuId)> {
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
    let icon = match Icon::from_rgba(vec![0x33, 0x66, 0xcc, 0xff], 1, 1) {
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

/// Application state owns the transport and the last session-list result so UI rendering stays local.
struct DesktopApp {
    host: Host,
    host_url: Option<String>,
    transport: Option<HttpTransport>,
    sessions: Vec<String>,
    status: String,
    quitting: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    hide_requested: Arc<AtomicBool>,
}

impl DesktopApp {
    fn new(quitting: Arc<AtomicBool>, show_requested: Arc<AtomicBool>, hide_requested: Arc<AtomicBool>) -> Self {
        let host = Host::start();
        let host_url = host.host_url().map(str::to_owned);
        let status = host.error().map_or_else(
            || {
                if host_url.is_some() {
                    "Host ready — click Connect".to_owned()
                } else {
                    "Disconnected — Host launch configuration is not set".to_owned()
                }
            },
            str::to_owned,
        );
        Self {
            host,
            host_url,
            transport: None,
            sessions: Vec::new(),
            status,
            quitting,
            show_requested,
            hide_requested,
        }
    }

    fn connect(&mut self) {
        let Some(launch_url) = self.host.launch_url() else {
            return;
        };
        match HttpTransport::authenticate(launch_url) {
            Ok((base, transport)) => {
                self.host_url = Some(base);
                self.transport = Some(transport);
                self.status = "Connected — loading sessions".to_owned();
                self.refresh_sessions();
            }
            Err(error) => self.status = format!("Authentication failed: {error}"),
        }
    }

    fn refresh_sessions(&mut self) {
        let Some(transport) = self.transport.as_ref() else {
            return;
        };
        match SessionViewer::new(transport).list(None) {
            Ok(items) => {
                self.sessions = items.into_iter().map(|item| item.session_id).collect();
                self.status = format!("{} session(s)", self.sessions.len());
            }
            Err(error) => self.status = format!("Session list failed: {error}"),
        }
    }
}

impl eframe::App for DesktopApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.show_requested.swap(false, std::sync::atomic::Ordering::AcqRel) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.hide_requested.swap(false, std::sync::atomic::Ordering::AcqRel) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|i| i.viewport().close_requested()) && !self.quitting.load(std::sync::atomic::Ordering::Acquire) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_requested.store(true, std::sync::atomic::Ordering::Release);
        }
        egui::Panel::top("title_bar").show(ui, |ui| {
            ui.heading("DeepSeek Harness");
        });
        egui::Panel::left("sessions").show(ui, |ui| {
            ui.heading("Sessions");
            if self.transport.is_some() {
                if ui.button("Refresh").clicked() {
                    self.refresh_sessions();
                }
            } else {
                ui.label("Connect to enable sessions");
            }
            if self.sessions.is_empty() {
                ui.label("No sessions loaded");
            } else {
                for session_id in &self.sessions {
                    let _ = ui.selectable_label(false, session_id);
                }
            }
            if ui.button("Open file").clicked() {
                let _ = dialog::open_file();
            }
            if ui.button("Save file").clicked() {
                let _ = dialog::save_file();
            }
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.status);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Welcome");
            if let Some(url) = &self.host_url {
                ui.label(format!("Host: {url}"));
            }
            if self.transport.is_none()
                && self.host.launch_url().is_some()
                && ui.button("Connect").clicked()
            {
                self.connect();
            }
            ui.label(&self.status);
        });
    }
}

/// Starts the native window after enforcing single-instance ownership.
fn main() -> eframe::Result {
    let Some(_instance_lock) = acquire_instance_lock() else {
        return Ok(());
    };
    updater::check();
    let quitting = Arc::new(AtomicBool::new(false));
    let tray = setup_tray();
    let (show_id, quit_id) = tray.as_ref().map_or_else(
        || (None, None),
        |(_, show, quit)| (Some(show.clone()), Some(quit.clone())),
    );
    let show_requested = Arc::new(AtomicBool::new(false));
    let hide_requested = Arc::new(AtomicBool::new(false));
    let options = eframe::NativeOptions::default();
    let event_quitting = Arc::clone(&quitting);
    let event_show = Arc::clone(&show_requested);
    let event_hide = Arc::clone(&hide_requested);
    let _menu_listener = MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
        if Some(event.id.clone()) == quit_id {
            event_quitting.store(true, Ordering::Release);
        } else if Some(event.id) == show_id {
            let currently_hidden = event_hide.load(Ordering::Acquire);
            if currently_hidden {
                event_show.store(true, Ordering::Release);
            } else {
                event_hide.store(true, Ordering::Release);
            }
        }
    }));
    let tray_show = Arc::clone(&show_requested);
    let tray_quitting2 = Arc::clone(&quitting);
    let _tray_listener = TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click { .. } = event {
            tray_show.store(true, Ordering::Release);
        }
        if tray_quitting2.load(Ordering::Acquire) {
            std::process::exit(0);
        }
    }));
    let app_quitting = Arc::clone(&quitting);
    let app_show = Arc::clone(&show_requested);
    let app_hide = Arc::clone(&hide_requested);
    eframe::run_native(
        "DeepSeek Harness",
        options,
        Box::new(move |_creation_context| Ok(Box::new(DesktopApp::new(app_quitting, app_show, app_hide)))),
    )
}
