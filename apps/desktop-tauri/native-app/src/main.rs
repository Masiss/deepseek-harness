//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

use dsh_client_core::{session::SessionViewer, transport::HttpTransport};
use eframe::egui;

/// Application state owns the transport and the last session-list result so UI rendering stays local.
struct DesktopApp {
    transport: Option<HttpTransport>,
    sessions: Vec<String>,
    status: String,
}

impl DesktopApp {
    /// Constructs the shell; an absent Host URL leaves the app usable in disconnected mode.
    fn new() -> Self {
        let Some(host_url) = std::env::var_os("DSH_HOST_URL") else {
            return Self {
                transport: None,
                sessions: Vec::new(),
                status: "Disconnected — set DSH_HOST_URL to connect".to_owned(),
            };
        };
        let host_url = host_url.to_string_lossy().into_owned();
        match HttpTransport::new(&host_url) {
            Ok(transport) => Self {
                transport: Some(transport),
                sessions: Vec::new(),
                status: "Connected — loading sessions".to_owned(),
            },
            Err(error) => Self {
                transport: None,
                sessions: Vec::new(),
                status: format!("Host URL error: {error}"),
            },
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
            if ui.button("Refresh").clicked() {
                self.refresh_sessions();
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
            ui.label("Native egui shell — host sidecar and authentication are TODO.");
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
