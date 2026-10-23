//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

mod dialog;
mod document_preview;
mod host;
mod settings_panel;
mod updater;

use document_preview::{DocumentPreview, TextDocument};
use dsh_client_core::{
    approvals::{ApprovalDecision, PendingApproval},
    chat::{ChatClient, PromptContentPart, PromptMode, PromptRequest},
    follow::{self, FollowFrame, FollowRequest},
    remote_events::{RemoteEvent, RemoteEventError, RemoteEvents},
    session::{SessionAddress, SessionAddressKind, SessionPage, SessionViewer, ToolCallStatus},
    sessions::{
        CreateSessionRequest, ForkSessionRequest, RenameSessionRequest, SessionSearchResponse,
        SessionsClient,
    },
    settings::{
        SettingsClient, SettingsDescribeValue, SettingsNamespaceView, SettingsUpdateRequest,
    },
    terminal::{
        self, TerminalClient, TerminalCloseRequest, TerminalCreateRequest, TerminalEnvironment,
        TerminalFrame, TerminalInput, TerminalRenameRequest, WebTerminalInfo,
    },
    transport::HttpTransport,
    workspace_files::{WorkspaceFileRange, WorkspaceFileScope, WorkspaceFilesClient},
    Error,
};
use eframe::egui;
use fs2::FileExt;
use host::Host;
use settings_panel::{SettingsAction, SettingsPanel};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

enum ApprovalCommand {
    Answer(PendingApproval, ApprovalDecision),
    Delegate(dsh_client_core::remote_events::RemoteEventInvocation),
}

const TERMINAL_VISIBLE_BYTES: usize = 128 * 1024;

/// Keeps the newest UTF-8 output, including when a single frame exceeds the visible budget.
fn append_terminal_output(output: &mut String, text: &str) {
    if text.len() >= TERMINAL_VISIBLE_BYTES {
        output.clear();
        let mut start = text.len() - TERMINAL_VISIBLE_BYTES;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        output.push_str(&text[start..]);
    } else {
        output.push_str(text);
        if output.len() > TERMINAL_VISIBLE_BYTES {
            let mut start = output.len() - TERMINAL_VISIBLE_BYTES;
            while !output.is_char_boundary(start) {
                start += 1;
            }
            output.drain(..start);
        }
    }
}

enum TerminalOperation {
    List,
    Create(TerminalCreateRequest),
    Rename(TerminalRenameRequest),
    Close(TerminalCloseRequest),
}

struct TerminalInventory {
    items: Vec<WebTerminalInfo>,
    environment: TerminalEnvironment,
    selected: Option<String>,
}

enum TerminalUpdate {
    Attached(mpsc::Sender<TerminalInput>),
    Frame(TerminalFrame),
    Error(String),
}

/// Owns workers until they settle; cancellation never joins a network worker on the egui frame.
#[derive(Default)]
struct TerminalPanel {
    session: Option<String>,
    items: Vec<WebTerminalInfo>,
    environment: Option<TerminalEnvironment>,
    selected: Option<String>,
    inventory: Option<mpsc::Receiver<Result<TerminalInventory, String>>>,
    updates: Option<mpsc::Receiver<TerminalUpdate>>,
    input: Option<mpsc::Sender<TerminalInput>>,
    stop: Option<mpsc::Sender<()>>,
    workers: Vec<std::thread::JoinHandle<()>>,
    output: String,
    text: String,
    title: String,
    status: String,
    snapshot_status: String,
    state_status: String,
    ack_status: String,
    error: Option<String>,
    writable: bool,
    sequence: u64,
    pending: Option<u64>,
    cols: u32,
    rows: u32,
}

impl TerminalPanel {
    fn detach(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.updates = None;
        self.input = None;
        self.writable = false;
        self.pending = None;
        self.status = "Detached (shell remains alive)".to_owned();
    }

    fn select_session(&mut self, session: Option<String>, transport: Option<HttpTransport>) {
        if self.session == session {
            return;
        }
        self.detach();
        self.session = session;
        self.items.clear();
        self.selected = None;
        self.inventory = None;
        self.environment = None;
        self.output.clear();
        self.text.clear();
        self.title.clear();
        self.snapshot_status.clear();
        self.state_status.clear();
        self.ack_status.clear();
        self.error = None;
        self.cols = 80;
        self.rows = 24;
        if let Some(transport) = transport {
            self.request(transport, TerminalOperation::List);
        }
    }

    fn request(&mut self, transport: HttpTransport, operation: TerminalOperation) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.inventory.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.workers.push(std::thread::spawn(move || {
            let client = TerminalClient::new(&transport, &session);
            let result = (|| {
                let selected = match operation {
                    TerminalOperation::List => None,
                    TerminalOperation::Create(request) => Some(client.create(request)?.id),
                    TerminalOperation::Rename(request) => {
                        client.rename(request)?;
                        None
                    }
                    TerminalOperation::Close(request) => {
                        client.close(request)?;
                        None
                    }
                };
                Ok::<_, Error>(TerminalInventory {
                    items: client.list(&session)?,
                    environment: client.environment()?,
                    selected,
                })
            })();
            let _ = sender.send(result.map_err(|error| error.to_string()));
        }));
        self.inventory = Some(receiver);
        self.status = "Loading terminal inventory / applying operation…".to_owned();
        self.error = None;
    }

    fn attach(&mut self, transport: HttpTransport) {
        let (Some(session), Some(id)) = (self.session.clone(), self.selected.clone()) else {
            return;
        };
        self.detach();
        self.output.clear();
        self.sequence = 0;
        self.snapshot_status.clear();
        self.state_status.clear();
        self.ack_status.clear();
        self.error = None;
        self.status = "Attaching…".to_owned();
        // The bounded handoff stops a stalled UI from retaining an unbounded frame history.
        let (sender, receiver) = mpsc::sync_channel(32);
        let (stop_sender, stop_receiver) = mpsc::channel();
        self.workers.push(std::thread::spawn(move || {
            let result = transport.credential().and_then(|credential| {
                terminal::attach(transport.base_url().as_str(), credential, session, id)
            });
            let (mut stream, input) = match result {
                Ok(result) => result,
                Err(error) => {
                    let _ = sender.send(TerminalUpdate::Error(error.to_string()));
                    return;
                }
            };
            let cancel = stream.cancel_handle().clone();
            let finished = AtomicBool::new(false);
            std::thread::scope(|scope| {
                let finished = &finished;
                scope.spawn(move || {
                    while !finished.load(Ordering::Acquire) {
                        match stop_receiver.recv_timeout(std::time::Duration::from_millis(30)) {
                            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        }
                    }
                    cancel.cancel();
                });
                if sender.send(TerminalUpdate::Attached(input)).is_ok() {
                    for frame in &mut stream {
                        let update = match frame {
                            Ok(frame) => TerminalUpdate::Frame(frame),
                            Err(error) => TerminalUpdate::Error(error.to_string()),
                        };
                        if sender.send(update).is_err() {
                            break;
                        }
                    }
                }
                // Dropping the stream cancels the socket even after a normal remote end.
                drop(stream);
                finished.store(true, Ordering::Release);
            });
        }));
        self.updates = Some(receiver);
        self.stop = Some(stop_sender);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.inventory {
            match receiver.try_recv() {
                Ok(result) => {
                    self.inventory = None;
                    match result {
                        Ok(inventory) => {
                            self.items = inventory.items;
                            self.cols = self.cols.clamp(2, inventory.environment.max_cols);
                            self.rows = self.rows.clamp(1, inventory.environment.max_rows);
                            self.environment = Some(inventory.environment);
                            if let Some(id) = inventory.selected {
                                self.detach();
                                self.selected = Some(id);
                                self.output.clear();
                                self.snapshot_status.clear();
                                self.state_status.clear();
                                self.ack_status.clear();
                                self.title = self
                                    .items
                                    .iter()
                                    .find(|item| Some(&item.id) == self.selected.as_ref())
                                    .map_or_else(String::new, |item| item.title.clone());
                            }
                            if self
                                .selected
                                .as_ref()
                                .is_some_and(|id| !self.items.iter().any(|item| &item.id == id))
                            {
                                self.detach();
                                self.selected = None;
                                self.output.clear();
                                self.snapshot_status.clear();
                                self.state_status.clear();
                                self.ack_status.clear();
                            }
                            self.status = format!("{} terminal(s)", self.items.len());
                        }
                        Err(error) => {
                            self.error = Some(format!(
                            "Terminal operation failed; refresh to confirm remote state: {error}"
                        ))
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.inventory = None;
                    self.error = Some("Terminal inventory worker stopped unexpectedly".to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        for _ in 0..64 {
            let Some(receiver) = &self.updates else {
                break;
            };
            match receiver.try_recv() {
                Ok(TerminalUpdate::Attached(input)) => {
                    self.input = Some(input);
                    self.status = "Attached — waiting for snapshot".to_owned();
                }
                Ok(TerminalUpdate::Error(error)) => {
                    self.detach();
                    self.error = Some(format!("Terminal stream failed: {error}"));
                }
                Ok(TerminalUpdate::Frame(frame)) => match frame {
                    TerminalFrame::Snapshot(frame) => {
                        self.output.clear();
                        append_terminal_output(&mut self.output, &frame.screen);
                        self.writable = frame.writable;
                        self.snapshot_status = format!("Snapshot #{}", frame.sequence);
                        self.state_status = format!(
                            "State: {:?} · exit: {:?} · writable: {}",
                            frame.info.state, frame.info.exit_code, frame.writable
                        );
                        self.status = "Attached".to_owned();
                        if let Some(error) = &frame.info.error {
                            self.error = Some(error.clone());
                        }
                        if let Some(item) =
                            self.items.iter_mut().find(|item| item.id == frame.info.id)
                        {
                            *item = frame.info;
                        }
                    }
                    TerminalFrame::Output(frame) => {
                        append_terminal_output(&mut self.output, &frame.data);
                        self.status =
                            format!("Output #{} · writable: {}", frame.sequence, self.writable);
                    }
                    TerminalFrame::State(frame) => {
                        self.writable = frame.writable;
                        self.state_status = format!(
                            "State: {:?} · exit: {:?} · writable: {}",
                            frame.info.state, frame.info.exit_code, frame.writable
                        );
                        if let Some(error) = &frame.info.error {
                            self.error = Some(error.clone());
                        }
                        if let Some(item) =
                            self.items.iter_mut().find(|item| item.id == frame.info.id)
                        {
                            *item = frame.info;
                        }
                    }
                    TerminalFrame::Ack(frame) => {
                        if self.pending == Some(frame.sequence) {
                            self.pending = None;
                        }
                        self.ack_status = format!("Ack #{} · {:?}", frame.sequence, frame.refusal);
                        self.status = "Attached".to_owned();
                        if let Some(refusal) = frame.refusal {
                            self.error =
                                Some(format!("Input #{} refused: {refusal:?}", frame.sequence));
                        }
                    }
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.detach();
                    self.status = "Terminal stream ended".to_owned();
                    break;
                }
            }
        }
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].is_finished() {
                if self.workers.swap_remove(index).join().is_err() {
                    self.error = Some("Terminal worker panicked".to_owned());
                }
            } else {
                index += 1;
            }
        }
        if self.inventory.is_some() || self.updates.is_some() || !self.workers.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    fn send(&mut self, resize: bool) {
        if !self.writable || self.pending.is_some() {
            return;
        }
        let Some(input) = &self.input else {
            return;
        };
        let sequence = self.sequence + 1;
        let command = if resize {
            TerminalInput::Resize {
                sequence,
                cols: self.cols,
                rows: self.rows,
            }
        } else {
            TerminalInput::Write {
                sequence,
                data: self.text.clone(),
            }
        };
        match input.send(command) {
            Ok(()) => {
                self.sequence = sequence;
                self.pending = Some(sequence);
                self.status = format!("Awaiting ack #{sequence}");
                if !resize {
                    self.text.clear();
                }
            }
            Err(error) => {
                self.error = Some(format!("Input was not queued: {error}"));
                self.detach();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, transport: Option<HttpTransport>) {
        ui.heading("Terminals");
        let Some(transport) = transport else {
            ui.label("Connect to use terminals");
            return;
        };
        let busy = self.inventory.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Refresh terminals"))
                .clicked()
            {
                self.request(transport.clone(), TerminalOperation::List);
            }
            if ui
                .add_enabled(
                    !busy && self.environment.is_some(),
                    egui::Button::new("Create terminal"),
                )
                .clicked()
            {
                let id = format!(
                    "native-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                );
                self.request(
                    transport.clone(),
                    TerminalOperation::Create(TerminalCreateRequest {
                        shell_path: None,
                        id,
                        cols: self.cols,
                        rows: self.rows,
                    }),
                );
            }
        });
        let mut selection = self.selected.clone();
        egui::ComboBox::from_id_salt("terminal-selection")
            .selected_text(
                self.items
                    .iter()
                    .find(|item| Some(&item.id) == self.selected.as_ref())
                    .map_or("Select terminal", |item| item.title.as_str()),
            )
            .show_ui(ui, |ui| {
                for item in &self.items {
                    ui.selectable_value(
                        &mut selection,
                        Some(item.id.clone()),
                        format!("{} · {:?}", item.title, item.state),
                    );
                }
            });
        if selection != self.selected {
            self.detach();
            self.selected = selection;
            self.output.clear();
            self.text.clear();
            self.snapshot_status.clear();
            self.state_status.clear();
            self.ack_status.clear();
            self.title = self
                .items
                .iter()
                .find(|item| Some(&item.id) == self.selected.as_ref())
                .map_or_else(String::new, |item| item.title.clone());
            if let Some(item) = self
                .items
                .iter()
                .find(|item| Some(&item.id) == self.selected.as_ref())
            {
                self.cols = item.cols;
                self.rows = item.rows;
            }
            self.error = None;
        }
        if let Some(id) = self.selected.clone() {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.updates.is_none() && !busy, egui::Button::new("Attach"))
                    .clicked()
                {
                    self.attach(transport.clone());
                }
                if ui
                    .add_enabled(self.updates.is_some(), egui::Button::new("Detach"))
                    .clicked()
                {
                    self.detach();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Close terminal (kills shell)"))
                    .clicked()
                {
                    self.detach();
                    self.request(
                        transport.clone(),
                        TerminalOperation::Close(TerminalCloseRequest { id: id.clone() }),
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.title)
                        .hint_text("Terminal title")
                        .char_limit(120),
                );
                if ui
                    .add_enabled(
                        !busy && !self.title.trim().is_empty(),
                        egui::Button::new("Rename"),
                    )
                    .clicked()
                {
                    self.request(
                        transport.clone(),
                        TerminalOperation::Rename(TerminalRenameRequest {
                            id,
                            title: self.title.trim().to_owned(),
                        }),
                    );
                }
            });
        }
        if busy {
            ui.spinner();
        }
        ui.label(&self.status);
        for text in [&self.snapshot_status, &self.state_status, &self.ack_status] {
            if !text.is_empty() {
                ui.small(text);
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        ui.small(
            "Plain-text terminal output (last 128 KiB); ANSI cursor positioning is not emulated.",
        );
        egui::ScrollArea::both()
            .id_salt("terminal-output")
            .max_height(180.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&self.output).monospace())
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
        let ready = self.input.is_some() && self.writable && self.pending.is_none();
        if let Some(environment) = &self.environment {
            let max_input = environment.max_input_bytes as usize;
            let (max_cols, max_rows) = (environment.max_cols, environment.max_rows);
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .desired_rows(2)
                    .hint_text("Exact text to write; include a newline to execute"),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        ready && !self.text.is_empty() && self.text.len() <= max_input,
                        egui::Button::new("Send exact text"),
                    )
                    .clicked()
                {
                    self.send(false);
                }
                ui.label(format!("{} / {max_input} input bytes", self.text.len()));
                ui.label("Columns");
                ui.add(egui::DragValue::new(&mut self.cols).range(2..=max_cols));
                ui.label("Rows");
                ui.add(egui::DragValue::new(&mut self.rows).range(1..=max_rows));
                if ui.add_enabled(ready, egui::Button::new("Resize")).clicked() {
                    self.send(true);
                }
            });
        }
    }
}

impl Drop for TerminalPanel {
    fn drop(&mut self) {
        self.detach();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod terminal_panel_tests {
    use super::{append_terminal_output, TerminalPanel, TERMINAL_VISIBLE_BYTES};
    use std::sync::mpsc;

    #[test]
    fn detach_requests_stop_and_releases_input() {
        let (stop, stopped) = mpsc::channel();
        let (input, inputs) = mpsc::channel();
        let mut panel = TerminalPanel::default();
        panel.stop = Some(stop);
        panel.input = Some(input);
        panel.writable = true;
        panel.pending = Some(1);
        panel.detach();
        assert_eq!(stopped.try_recv(), Ok(()));
        assert_eq!(inputs.try_recv(), Err(mpsc::TryRecvError::Disconnected));
        assert!(!panel.writable);
        assert_eq!(panel.pending, None);
    }

    #[test]
    fn output_preserves_exact_budget_and_newest_suffix() {
        let mut output = String::new();
        append_terminal_output(&mut output, &"x".repeat(TERMINAL_VISIBLE_BYTES));
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
        append_terminal_output(&mut output, "end");
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
        assert!(output.ends_with("end"));
    }

    #[test]
    fn oversized_and_incremental_output_preserve_utf8() {
        let mut output = "old output".to_owned();
        append_terminal_output(&mut output, &"界".repeat(TERMINAL_VISIBLE_BYTES));
        assert!(output.len() <= TERMINAL_VISIBLE_BYTES);
        assert!(output.chars().all(|character| character == '界'));
        append_terminal_output(&mut output, "🙂tail");
        assert!(output.len() <= TERMINAL_VISIBLE_BYTES);
        assert!(output.ends_with("🙂tail"));
    }
}

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

enum SessionOperation {
    List,
    Search(String),
    Create,
    Rename(RenameSessionRequest),
    Fork(ForkSessionRequest),
}

enum SessionUpdate {
    List(Vec<String>),
    Search(SessionSearchResponse),
    Created(String),
    Renamed(String),
    Forked(String),
}

/// Application state owns the transport and the last session-list result so UI rendering stays local.
struct DesktopApp {
    host: Option<Host>,
    host_receiver: Option<mpsc::Receiver<Host>>,
    connect_receiver: Option<mpsc::Receiver<Result<(String, HttpTransport), Error>>>,
    sessions_receiver: Option<mpsc::Receiver<Result<SessionUpdate, Error>>>,
    session_query: String,
    session_search: Option<SessionSearchResponse>,
    session_title: String,
    sessions_status: String,
    sessions_error: Option<String>,
    sessions_failure_context: &'static str,
    host_url: Option<String>,
    transport: Option<HttpTransport>,
    sessions: Vec<String>,
    selected_session: Option<String>,
    session_page: Option<SessionPage>,
    page_loading: bool,
    page_error: Option<String>,
    page_receiver: Option<mpsc::Receiver<Result<SessionPage, Error>>>,
    chat_text: String,
    chat_loading: bool,
    chat_receiver: Option<mpsc::Receiver<Result<FollowFrame, Error>>>,
    chat_frames: Vec<FollowFrame>,
    approval_events_receiver: Option<mpsc::Receiver<Result<RemoteEvent, RemoteEventError>>>,
    approval_commands: Option<mpsc::Sender<ApprovalCommand>>,
    pending_approvals: Vec<PendingApproval>,
    settings_panel: SettingsPanel,
    settings_description: Option<SettingsDescribeValue>,
    settings_loading: bool,
    settings_error: Option<String>,
    settings_receiver: Option<mpsc::Receiver<Result<SettingsDescribeValue, Error>>>,
    settings_update_receiver: Option<mpsc::Receiver<Result<SettingsNamespaceView, Error>>>,
    show_settings: bool,
    document_preview: DocumentPreview,
    preview_document: Option<TextDocument>,
    preview_path: String,
    preview_loading: bool,
    preview_error: Option<String>,
    preview_receiver: Option<mpsc::Receiver<Result<TextDocument, Error>>>,
    show_document: bool,
    terminal: TerminalPanel,
    status: String,
    quitting: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    hide_requested: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    viewport: Arc<Mutex<Option<egui::Context>>>,
    tray_available: bool,
}

impl DesktopApp {
    fn new(
        quitting: Arc<AtomicBool>,
        show_requested: Arc<AtomicBool>,
        hide_requested: Arc<AtomicBool>,
        hidden: Arc<AtomicBool>,
        viewport: Arc<Mutex<Option<egui::Context>>>,
        tray_available: bool,
    ) -> Self {
        let (host_sender, host_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = host_sender.send(Host::start());
        });
        Self {
            host: None,
            host_receiver: Some(host_receiver),
            connect_receiver: None,
            sessions_receiver: None,
            session_query: String::new(),
            session_search: None,
            session_title: String::new(),
            sessions_status: String::new(),
            sessions_error: None,
            sessions_failure_context: "",
            host_url: None,
            transport: None,
            sessions: Vec::new(),
            selected_session: None,
            session_page: None,
            page_loading: false,
            page_error: None,
            page_receiver: None,
            chat_text: String::new(),
            chat_loading: false,
            chat_receiver: None,
            chat_frames: Vec::new(),
            approval_events_receiver: None,
            approval_commands: None,
            pending_approvals: Vec::new(),
            settings_panel: SettingsPanel::default(),
            settings_description: None,
            settings_loading: false,
            settings_error: None,
            settings_receiver: None,
            settings_update_receiver: None,
            show_settings: false,
            document_preview: DocumentPreview,
            preview_document: None,
            preview_path: String::new(),
            preview_loading: false,
            preview_error: None,
            preview_receiver: None,
            show_document: false,
            terminal: TerminalPanel::default(),
            status: "Starting Host sidecar…".to_owned(),
            quitting,
            show_requested,
            hide_requested,
            hidden,
            viewport,
            tray_available,
        }
    }

    fn connect(&mut self) {
        let Some(launch_url) = self
            .host
            .as_ref()
            .and_then(Host::launch_url)
            .map(str::to_owned)
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(HttpTransport::authenticate(&launch_url));
        });
        self.connect_receiver = Some(receiver);
        self.status = "Authenticating with Host…".to_owned();
    }

    fn start_remote_events(&mut self, transport: HttpTransport) {
        let (event_sender, event_receiver) = mpsc::channel();
        let (command_sender, command_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let events = match RemoteEvents::connect(transport) {
                Ok(events) => events,
                Err(error) => {
                    let _ = event_sender.send(Err(error));
                    return;
                }
            };
            loop {
                while let Ok(command) = command_receiver.try_recv() {
                    let result = match command {
                        ApprovalCommand::Answer(pending, decision) => {
                            pending.answer(&events, decision)
                        }
                        ApprovalCommand::Delegate(invocation) => events.submit::<ApprovalDecision>(
                            &invocation,
                            dsh_client_core::remote_events::RemoteEventOutcome::Next,
                        ),
                    };
                    if let Err(error) = result {
                        let _ = event_sender.send(Err(error));
                    }
                }
                match events.recv_timeout(std::time::Duration::from_millis(25)) {
                    Ok(Some(event)) => {
                        if event_sender.send(Ok(event)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = event_sender.send(Err(error));
                        return;
                    }
                }
            }
        });
        self.approval_events_receiver = Some(event_receiver);
        self.approval_commands = Some(command_sender);
        self.pending_approvals.clear();
    }

    fn refresh_sessions(&mut self) {
        self.request_sessions(SessionOperation::List);
    }

    fn request_sessions(&mut self, operation: SessionOperation) {
        if self.sessions_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            return;
        };
        self.sessions_status = match &operation {
            SessionOperation::List => "Loading sessions…",
            SessionOperation::Search(_) => "Searching sessions…",
            SessionOperation::Create => "Creating session…",
            SessionOperation::Rename(_) => "Renaming session…",
            SessionOperation::Fork(_) => "Forking session…",
        }
        .to_owned();
        self.sessions_failure_context = match &operation {
            SessionOperation::List => "List refresh failed; previous inventory and selection retained. Refresh to retry",
            SessionOperation::Search(_) => "Search failed; previous results retained. Search again to retry",
            SessionOperation::Create => "Create outcome unknown; refresh/search to confirm remote state before retrying",
            SessionOperation::Rename(_) => "Rename outcome unknown; reload the selected session to confirm its title before retrying",
            SessionOperation::Fork(_) => "Fork outcome unknown; refresh/search to confirm remote state before retrying",
        };
        self.sessions_error = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = SessionsClient::new(&transport);
            let result = match operation {
                SessionOperation::List => SessionViewer::new(&transport).list(None).map(|items| {
                    SessionUpdate::List(items.into_iter().map(|item| item.session_id).collect())
                }),
                SessionOperation::Search(query) => client.search(query).map(SessionUpdate::Search),
                SessionOperation::Create => client
                    .create(CreateSessionRequest {
                        workspace_id: None,
                        cwd: None,
                        session_id: None,
                        agent_preset: None,
                    })
                    .map(|session| SessionUpdate::Created(session.session_id)),
                SessionOperation::Rename(request) => client
                    .rename(request)
                    .map(|response| SessionUpdate::Renamed(response.title)),
                SessionOperation::Fork(request) => client
                    .fork(request)
                    .map(|response| SessionUpdate::Forked(response.session_id)),
            };
            let _ = sender.send(result);
        });
        self.sessions_receiver = Some(receiver);
    }

    fn open_settings(&mut self) {
        self.show_settings = true;
        if self.settings_description.is_some() || self.settings_loading {
            return;
        }
        self.refresh_settings();
    }

    fn refresh_settings(&mut self) {
        if self.settings_loading || self.settings_update_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.settings_error = Some("Connect to load settings".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SettingsClient::new(&transport).describe());
        });
        self.settings_receiver = Some(receiver);
        self.settings_loading = true;
        self.settings_error = None;
    }

    fn update_settings(&mut self, request: SettingsUpdateRequest) {
        if self.settings_loading || self.settings_update_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.settings_panel.finish_update(Some(
                "Not connected. Refresh settings before editing again.".into(),
            ));
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SettingsClient::new(&transport).update(request));
        });
        self.settings_update_receiver = Some(receiver);
    }

    fn read_document(&mut self) {
        let (Some(session_id), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.preview_error = Some("Select a connected session before reading a file.".into());
            return;
        };
        let path = self.preview_path.trim().to_owned();
        if path.is_empty() {
            self.preview_error = Some("Enter a Host file path.".into());
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = WorkspaceFilesClient::new(&transport)
                .read(
                    WorkspaceFileScope { session_id },
                    path.clone(),
                    WorkspaceFileRange {
                        offset: None,
                        limit: Some(500),
                    },
                )
                .map(|page| {
                    let name = page
                        .stat
                        .absolute_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&path)
                        .to_owned();
                    TextDocument {
                        name,
                        host_path: page.stat.absolute_path,
                        text: page.text,
                    }
                });
            let _ = sender.send(result);
        });
        self.preview_receiver = Some(receiver);
        self.preview_loading = true;
        self.preview_error = None;
    }

    fn load_session(&mut self, session_id: String) {
        self.session_title.clear();
        self.show_settings = false;
        self.selected_session = Some(session_id.clone());
        self.terminal
            .select_session(Some(session_id.clone()), self.transport.clone());
        self.session_page = None;
        self.page_error = None;
        self.chat_receiver = None;
        self.chat_loading = false;
        self.chat_frames.clear();
        self.preview_document = None;
        self.preview_error = None;
        self.show_document = false;
        self.page_loading = true;
        let Some(transport) = self.transport.clone() else {
            self.page_loading = false;
            self.page_error = Some("Connect to load this session".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = SessionViewer::new(&transport).page(&session_id, i64::MAX);
            let _ = sender.send(result);
        });
        self.page_receiver = Some(receiver);
    }

    fn send_prompt(&mut self) {
        let Some(session_id) = self.selected_session.clone() else {
            return;
        };
        let text = self.chat_text.trim().to_owned();
        if text.is_empty() || self.chat_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.page_error = Some("Connect before sending a prompt".to_owned());
            return;
        };
        self.chat_text.clear();
        self.chat_frames.clear();
        self.chat_loading = true;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let chat = ChatClient::new(&transport);
            let request = PromptRequest {
                request_id: chat.next_request_id(),
                session_id: session_id.clone(),
                mode: PromptMode::Queue,
                content: vec![PromptContentPart::Text { text }],
                client_time_zone: None,
            };
            if let Err(error) = chat.prompt(request) {
                let _ = sender.send(Err(error));
                return;
            }
            let credential = match transport.credential() {
                Ok(credential) => credential.clone(),
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            let request = FollowRequest {
                address: SessionAddress {
                    kind: SessionAddressKind::Session,
                    session_id,
                },
                max_messages: Some(200),
                turn_window: None,
                assistant_stream: Some(true),
            };
            match follow::follow(transport.base_url().as_str(), &credential, request) {
                Ok(stream) => {
                    for frame in stream {
                        if sender.send(frame).is_err() {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                }
            }
        });
        self.chat_receiver = Some(receiver);
    }
}

impl eframe::App for DesktopApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(mut viewport) = self.viewport.lock() {
            *viewport = Some(ctx.clone());
        }
        if self.quitting.load(Ordering::Acquire) {
            self.terminal.detach();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.terminal
            .select_session(self.selected_session.clone(), self.transport.clone());
        self.terminal.poll(ctx);
        if let Some(receiver) = &self.host_receiver {
            match receiver.try_recv() {
                Ok(host) => {
                    self.host_url = host.host_url().map(str::to_owned);
                    self.status = host.error().map_or_else(
                        || {
                            self.host_url.as_ref().map_or_else(
                                || "Disconnected — Host launch configuration is not set".to_owned(),
                                |_| "Host ready — click Connect".to_owned(),
                            )
                        },
                        str::to_owned,
                    );
                    self.host = Some(host);
                    self.host_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Host startup worker stopped unexpectedly".to_owned();
                    self.host_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30))
                }
            }
        }
        if let Some(receiver) = &self.connect_receiver {
            match receiver.try_recv() {
                Ok(Ok((base, transport))) => {
                    self.host_url = Some(base);
                    self.start_remote_events(transport.clone());
                    self.transport = Some(transport);
                    self.connect_receiver = None;
                    self.status = "Connected — loading sessions…".to_owned();
                    self.refresh_sessions();
                }
                Ok(Err(error)) => {
                    self.connect_receiver = None;
                    self.status = format!("Authentication failed: {error}");
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.connect_receiver = None;
                    self.status = "Authentication worker stopped unexpectedly".to_owned();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30))
                }
            }
        }
        if let Some(receiver) = &self.sessions_receiver {
            match receiver.try_recv() {
                Ok(Ok(update)) => {
                    self.sessions_receiver = None;
                    match update {
                        SessionUpdate::List(sessions) => {
                            self.sessions = sessions;
                            self.session_search = None;
                            self.sessions_status = format!("{} session(s)", self.sessions.len());
                        }
                        SessionUpdate::Search(results) => {
                            self.sessions_status = format!("{} match(es)", results.items.len());
                            self.session_search = Some(results);
                        }
                        SessionUpdate::Created(id) | SessionUpdate::Forked(id) => {
                            self.load_session(id.clone());
                            self.session_query.clear();
                            self.session_search = None;
                            self.refresh_sessions();
                            self.sessions_status = format!("Session {id} ready; refreshing list…");
                        }
                        SessionUpdate::Renamed(title) => {
                            self.session_title = title.clone();
                            self.session_query.clear();
                            self.session_search = None;
                            self.refresh_sessions();
                            self.sessions_status = format!("Renamed to {title}; refreshing list…");
                        }
                    }
                }
                Ok(Err(error)) => {
                    self.sessions_receiver = None;
                    self.sessions_status = "Session request failed".to_owned();
                    self.sessions_error =
                        Some(format!("{}: {error}", self.sessions_failure_context));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sessions_receiver = None;
                    self.sessions_status = "Session worker stopped unexpectedly".to_owned();
                    self.sessions_error = Some(self.sessions_failure_context.to_owned());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30))
                }
            }
        }
        if let Some(receiver) = &self.page_receiver {
            match receiver.try_recv() {
                Ok(Ok(page)) => {
                    self.session_page = Some(page);
                    self.page_loading = false;
                    self.page_error = None;
                    self.page_receiver = None;
                }
                Ok(Err(error)) => {
                    self.page_loading = false;
                    self.page_error = Some(format!("Could not load session: {error}"));
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.page_loading = false;
                    self.page_error = Some("Session request stopped unexpectedly".to_owned());
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.settings_update_receiver {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result.map(|_| ())),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(Error::InvalidEnvelope("settings worker stopped")))
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    None
                }
            };
            if let Some(result) = result {
                self.settings_update_receiver = None;
                match result {
                    Ok(()) => {
                        self.settings_panel.finish_update(None);
                        self.refresh_settings();
                    }
                    Err(error) => self
                        .settings_panel
                        .finish_update(Some(settings_write_error(&error))),
                }
            }
        }
        if let Some(receiver) = &self.settings_receiver {
            match receiver.try_recv() {
                Ok(Ok(description)) => {
                    self.settings_panel.refreshed();
                    self.settings_description = Some(description);
                    self.settings_error = None;
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Ok(Err(error)) => {
                    let _ = error; // Raw server messages may include submitted settings or credentials.
                    self.settings_error =
                        Some("Could not load settings. Refresh to try again.".into());
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.settings_error = Some("Settings request stopped unexpectedly".to_owned());
                    self.settings_loading = false;
                    self.settings_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.preview_receiver {
            match receiver.try_recv() {
                Ok(Ok(document)) => {
                    self.preview_document = Some(document);
                    self.preview_loading = false;
                    self.preview_error = None;
                    self.preview_receiver = None;
                }
                Ok(Err(error)) => {
                    self.preview_loading = false;
                    self.preview_error = Some(format!("Could not read Host file: {error}"));
                    self.preview_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.preview_loading = false;
                    self.preview_error = Some("Document read worker stopped unexpectedly".into());
                    self.preview_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.chat_receiver {
            match receiver.try_recv() {
                Ok(Ok(frame)) => self.chat_frames.push(frame),
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Chat stream failed: {error}"));
                    self.chat_loading = false;
                    self.chat_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.chat_loading = false;
                    self.chat_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.approval_events_receiver {
            match receiver.try_recv() {
                Ok(Ok(RemoteEvent::Ready { .. })) => {
                    self.status = "Connected — permissions are listening".to_owned();
                }
                Ok(Ok(RemoteEvent::Waterfall(invocation))) => {
                    if invocation.event == "approval/request" {
                        match PendingApproval::from_invocation(invocation) {
                            Ok(pending) => self.pending_approvals.push(pending),
                            Err(error) => {
                                self.page_error = Some(format!("Invalid approval request: {error}"))
                            }
                        }
                    } else if let Some(commands) = &self.approval_commands {
                        let _ = commands.send(ApprovalCommand::Delegate(invocation));
                    }
                }
                Ok(Ok(RemoteEvent::Cancel { event_id })) => {
                    self.pending_approvals
                        .retain(|pending| pending.invocation.event_id != event_id);
                }
                Ok(Ok(RemoteEvent::Emit { .. })) => {}
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Permission event stream failed: {error}"));
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Permission event stream closed".to_owned();
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if self
            .show_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.hidden.store(false, Ordering::Release);
        }
        if self
            .hide_requested
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden.store(true, Ordering::Release);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|i| i.viewport().close_requested())
            && self.tray_available
            && !self.quitting.load(std::sync::atomic::Ordering::Acquire)
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_requested
                .store(true, std::sync::atomic::Ordering::Release);
            self.hidden.store(true, Ordering::Release);
        }
        egui::Panel::top("title_bar").show(ui, |ui| {
            ui.heading("DeepSeek Harness");
        });
        egui::Panel::left("sessions").show(ui, |ui| {
            if ui
                .selectable_label(!self.show_settings, "Sessions")
                .clicked()
            {
                self.show_settings = false;
                self.show_document = false;
            }
            if ui
                .selectable_label(self.show_settings, "Settings")
                .clicked()
            {
                self.open_settings();
                self.show_document = false;
            }
            if ui
                .selectable_label(self.show_document, "Document")
                .clicked()
            {
                self.show_document = true;
                self.show_settings = false;
            }
            ui.heading("Sessions");
            let connected = self.transport.is_some();
            let busy = self.sessions_receiver.is_some();
            ui.add_enabled_ui(connected && !busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Refresh").clicked() {
                        self.session_query.clear();
                        self.refresh_sessions();
                    }
                    if ui.button("New Session").clicked() {
                        self.request_sessions(SessionOperation::Create);
                    }
                });
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.session_query)
                        .hint_text("Search sessions")
                        .desired_width(170.0),
                );
                if ui.button("Search").clicked()
                    || (search.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                {
                    if self.session_query.trim().is_empty() {
                        self.refresh_sessions();
                    } else {
                        self.request_sessions(SessionOperation::Search(self.session_query.clone()));
                    }
                }
                if let Some(session_id) = self.selected_session.clone() {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.session_title)
                            .hint_text("New session title")
                            .desired_width(170.0),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                !self.session_title.trim().is_empty(),
                                egui::Button::new("Rename selected"),
                            )
                            .clicked()
                        {
                            self.request_sessions(SessionOperation::Rename(RenameSessionRequest {
                                session_id: session_id.clone(),
                                title: self.session_title.trim().to_owned(),
                            }));
                        }
                        if ui.button("Fork selected").clicked() {
                            self.request_sessions(SessionOperation::Fork(ForkSessionRequest {
                                session_id,
                                at_seq: None,
                                allow_migration: None,
                            }));
                        }
                    });
                }
            });
            if !connected {
                ui.label("Connect to enable sessions");
            }
            if busy {
                ui.spinner();
            }
            if !self.sessions_status.is_empty() {
                ui.small(&self.sessions_status);
            }
            if let Some(error) = &self.sessions_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.add_enabled_ui(!busy, |ui| {
                let mut selected = None;
                egui::ScrollArea::vertical()
                    .id_salt("session-inventory")
                    .max_height(300.0)
                    .show(ui, |ui| {
                        if let Some(results) = &self.session_search {
                            for item in &results.items {
                                if ui
                                    .selectable_label(
                                        self.selected_session.as_ref() == Some(&item.session_id),
                                        &item.session_id,
                                    )
                                    .on_hover_text(&item.snippet)
                                    .clicked()
                                {
                                    selected = Some(item.session_id.clone());
                                }
                            }
                            if results.has_more {
                                ui.small("More matches exist; refine your search.");
                            }
                        } else {
                            for session_id in &self.sessions {
                                if ui
                                    .selectable_label(
                                        self.selected_session.as_ref() == Some(session_id),
                                        session_id,
                                    )
                                    .clicked()
                                {
                                    selected = Some(session_id.clone());
                                }
                            }
                        }
                    });
                if let Some(session_id) = selected {
                    self.load_session(session_id);
                }
            });
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
        if self.selected_session.is_some() {
            egui::Panel::bottom("terminal-panel").show(ui, |ui| {
                egui::CollapsingHeader::new("Session terminals")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.terminal.ui(ui, self.transport.clone());
                    });
            });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.show_settings {
                let action = self.settings_panel.ui(
                    ui,
                    self.settings_description.as_ref(),
                    self.settings_loading,
                    self.settings_error.as_deref(),
                    self.settings_update_receiver.is_some(),
                );
                match action {
                    Some(SettingsAction::Refresh) => self.refresh_settings(),
                    Some(SettingsAction::Update(request)) => self.update_settings(request),
                    None => {}
                }
            } else if self.show_document {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preview_path)
                            .hint_text("Host file path")
                            .desired_width(360.0),
                    );
                    if ui
                        .add_enabled(!self.preview_loading, egui::Button::new("Preview text"))
                        .clicked()
                    {
                        self.read_document();
                    }
                });
                self.document_preview.ui(
                    ui,
                    self.preview_document.as_ref(),
                    self.preview_loading,
                    self.preview_error.as_deref(),
                );
            } else if self.selected_session.is_none() {
                ui.heading("Welcome");
                if let Some(url) = &self.host_url {
                    ui.label(format!("Host: {url}"));
                }
                if self.transport.is_none()
                    && self.host.as_ref().and_then(Host::launch_url).is_some()
                    && ui.button("Connect").clicked()
                {
                    self.connect();
                }
                ui.label(&self.status);
            } else {
                ui.heading(self.selected_session.as_deref().unwrap_or("Session"));
                if self.page_loading {
                    ui.label("Loading session…");
                } else if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                } else if let Some(page) = &self.session_page {
                    if page.records.is_empty() {
                        ui.label("This session has no events");
                    } else {
                        let calls = SessionViewer::tool_calls(page);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for record in &page.records {
                                let event = &record.event;
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(format!("{}", event.seq));
                                    ui.label(&event.event_type);
                                    ui.label(event_text(&event.data));
                                });
                                ui.separator();
                            }
                            if !calls.is_empty() {
                                ui.heading("Tool calls");
                                for call in calls {
                                    ui.label(format!(
                                        "{} — {} ({})",
                                        call.name,
                                        call.call_id,
                                        match call.status {
                                            ToolCallStatus::Pending => "pending",
                                            ToolCallStatus::Succeeded => "succeeded",
                                            ToolCallStatus::Failed => "failed",
                                            ToolCallStatus::Unknown => "unknown",
                                        }
                                    ));
                                }
                            }
                        });
                        if page.has_more {
                            ui.label("More events are available");
                        }
                    }
                } else {
                    ui.label("Select a session to view its events");
                }
                ui.separator();
                ui.heading("Chat");
                ui.add_enabled(
                    !self.chat_loading,
                    egui::TextEdit::multiline(&mut self.chat_text)
                        .hint_text("Message")
                        .desired_rows(3),
                );
                if ui
                    .add_enabled(
                        !self.chat_loading && !self.chat_text.trim().is_empty(),
                        egui::Button::new("Send"),
                    )
                    .clicked()
                {
                    self.send_prompt();
                }
                if self.chat_loading {
                    ui.label("Waiting for assistant…");
                }
                for frame in &self.chat_frames {
                    match frame {
                        FollowFrame::Event(frame) => {
                            ui.label(format!("{} · {}", frame.event.seq, frame.event.event_type));
                        }
                        FollowFrame::AssistantStream { frame } => {
                            ui.label(format!("Assistant stream: {frame:?}"));
                        }
                        FollowFrame::Snapshot(snapshot) => {
                            ui.label(format!("Stream snapshot at {}", snapshot.cursor));
                        }
                    }
                }
                if let Some(pending) = self.pending_approvals.first().cloned() {
                    ui.separator();
                    ui.heading(format!("Permission: {}", pending.request.tool_name));
                    if let Some(reason) = &pending.request.reason {
                        ui.label(reason);
                    }
                    if let Some(commands) = &self.approval_commands {
                        let mut decision = None;
                        ui.horizontal(|ui| {
                            if ui.button("Allow once").clicked() {
                                decision = Some(ApprovalDecision::AllowedOnce);
                            }
                            if ui.button("Reject").clicked() {
                                decision = Some(ApprovalDecision::Rejected);
                            }
                        });
                        if let Some(decision) = decision {
                            let event_id = pending.invocation.event_id.clone();
                            let _ = commands.send(ApprovalCommand::Answer(pending, decision));
                            self.pending_approvals
                                .retain(|item| item.invocation.event_id != event_id);
                        }
                    }
                }
            }
        });
    }
}

fn settings_write_error(error: &Error) -> String {
    match error {
        Error::Remote { code, .. } if code == "SETTINGS_CONFLICT" =>
            "Settings conflict: another edit changed this namespace. Draft retained; refresh settings, discard the draft, and review current values before editing again.".into(),
        Error::Remote { .. } =>
            "Save failed. Draft retained; refresh settings to confirm remote values and review the schema before editing again. No automatic retry was made.".into(),
        _ => "Save outcome unknown. Draft retained; refresh settings to confirm remote values before discarding the draft and editing again. No automatic retry was made.".into(),
    }
}

fn event_text(data: &serde_json::Value) -> String {
    for key in ["text", "message", "content", "name"] {
        if let Some(text) = data.get(key).and_then(serde_json::Value::as_str) {
            return text.to_owned();
        }
    }
    let text = data.to_string();
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

/// Starts the native window after enforcing single-instance ownership.
fn main() -> eframe::Result {
    let Some(_instance_lock) = acquire_instance_lock() else {
        return Ok(());
    };
    std::thread::spawn(updater::check);
    let quitting = Arc::new(AtomicBool::new(false));
    let tray = setup_tray();
    let tray_available = tray.is_some();
    let (show_id, quit_id) = tray.as_ref().map_or_else(
        || (None, None),
        |(_, show, quit)| (Some(show.clone()), Some(quit.clone())),
    );
    let hidden = Arc::new(AtomicBool::new(false));
    let viewport = Arc::new(Mutex::new(None::<egui::Context>));
    let show_requested = Arc::new(AtomicBool::new(false));
    let hide_requested = Arc::new(AtomicBool::new(false));
    let options = eframe::NativeOptions::default();
    let event_quitting = Arc::clone(&quitting);
    let event_show = Arc::clone(&show_requested);
    let event_hide = Arc::clone(&hide_requested);
    let event_hidden = Arc::clone(&hidden);
    let event_viewport = Arc::clone(&viewport);
    let _menu_listener =
        MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            if Some(event.id.clone()) == quit_id {
                event_quitting.store(true, Ordering::Release);
                if let Ok(viewport) = event_viewport.lock() {
                    if let Some(ctx) = viewport.as_ref() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            } else if Some(event.id) == show_id {
                let currently_hidden = event_hidden.load(Ordering::Acquire);
                if let Ok(viewport) = event_viewport.lock() {
                    if let Some(ctx) = viewport.as_ref() {
                        if currently_hidden {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        } else {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                        }
                        ctx.request_repaint();
                    }
                }
                event_show.store(currently_hidden, Ordering::Release);
                event_hide.store(!currently_hidden, Ordering::Release);
                event_hidden.store(!currently_hidden, Ordering::Release);
            }
        }));
    let tray_show = Arc::clone(&show_requested);
    let tray_viewport = Arc::clone(&viewport);
    let _tray_listener = TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click { .. } = event {
            tray_show.store(true, Ordering::Release);
            if let Ok(viewport) = tray_viewport.lock() {
                if let Some(ctx) = viewport.as_ref() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
        }
    }));
    let app_quitting = Arc::clone(&quitting);
    let app_show = Arc::clone(&show_requested);
    let app_hide = Arc::clone(&hide_requested);
    let app_hidden = Arc::clone(&hidden);
    let app_viewport = Arc::clone(&viewport);
    let app_tray_available = tray_available;
    eframe::run_native(
        "DeepSeek Harness",
        options,
        Box::new(move |_creation_context| {
            Ok(Box::new(DesktopApp::new(
                app_quitting,
                app_show,
                app_hide,
                app_hidden,
                app_viewport,
                app_tray_available,
            )))
        }),
    )
}
