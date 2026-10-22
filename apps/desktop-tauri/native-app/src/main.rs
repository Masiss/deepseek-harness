//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

mod host;

use dsh_client_core::{session::SessionViewer, transport::HttpTransport};
use eframe::egui;
use host::Host;

/// Application state owns the transport and the last session-list result so UI rendering stays local.
struct DesktopApp {
    host: Host,
    host_url: Option<String>,
    transport: Option<HttpTransport>,
    sessions: Vec<String>,
    status: String,
}

impl DesktopApp {
    /// Constructs the shell; an absent Host URL leaves the app usable in disconnected mode.
    fn new() -> Self {
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

    /// Refreshes the visible inventory through client-core, preserving typed errors for the status line.
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
    /// Logic runs before painting; nothing to do here yet.
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {}

    /// Renders the title bar, session list, and connection status each frame.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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

/// Starts the pure-egui native window; no webview is created.
fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "DeepSeek Harness",
        options,
        Box::new(|_creation_context| Ok(Box::new(DesktopApp::new()))),
    )
}
