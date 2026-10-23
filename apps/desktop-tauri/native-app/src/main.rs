//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

mod dialog;
mod document_preview;
mod host;
mod settings_panel;
mod terminal_ansi;
mod updater;

use document_preview::{
    DocumentPreview, ImageDocument, ImagePreview, PreviewDocument, TextDocument,
};
use dsh_client_core::{
    approvals::{ApprovalDecision, PendingApproval},
    chat::{AcceptedResponse, ChatClient, PromptContentPart, PromptMode, PromptRequest},
    follow::{self, FollowFrame, FollowRequest},
    models::{
        ModelCatalog, ModelCatalogModel, ModelClient, ModelSelection, SelectModelRequest,
        SelectModelResponse,
    },
    plugins::{PluginInventorySnapshot, PluginsClient},
    schedules::{ScheduleDeleteRequest, ScheduleListRequest, SchedulesClient},
    remote_events::{RemoteEvent, RemoteEventError, RemoteEventOutcome, RemoteEvents},
    remote_stream::RemoteStreamMux,
    streams::{self, JobFollowFrame, JobView},
    session::{
        SessionAddress, SessionAddressKind, SessionPage, SessionSummary, SessionViewer,
        ToolCallStatus,
    },
    stream::FrameStream,
    sessions::{
        CreateSessionRequest, ForkSessionRequest, RenameSessionRequest, SessionSearchResponse,
        SessionsClient,
    },
    settings::{
        SettingsClient, SettingsDescribeValue, SettingsNamespaceView, SettingsUpdateRequest,
    },
    workspace_files::WorkspaceByteReadOptions,
    terminal::{
        self, TerminalClient, TerminalCloseRequest, TerminalCreateRequest,
        TerminalEnvironment, TerminalFrame, TerminalInput, TerminalInputLimits,
        TerminalRenameRequest, TerminalWriter, WebTerminalInfo,
    },
    transport::HttpTransport,
    upload::{FileUploadClient, FileUploadValue},
    user_questions::{AskUserQuestionAnswer, AskUserQuestionAnswerItem},
    workspace_files::{
        WorkspaceByteRange, WorkspaceFileRange, WorkspaceFileScope, WorkspaceFilesClient,
    },
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
fn append_bounded_output(output: &mut String, text: &str) {
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
    input: Option<TerminalWriter>,
    stop: Option<mpsc::Sender<()>>,
    workers: Vec<std::thread::JoinHandle<()>>,
    output: String,
    output_revision: u64,
    parsed: Vec<terminal_ansi::AnsiLine>,
    parsed_revision: u64,
    text: String,
    title: String,
    status: String,
    snapshot_status: String,
    state_status: String,
    ack_status: String,
    error: Option<String>,
    writable: bool,
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

    fn clear_output(&mut self) {
        self.output.clear();
        self.output_revision = self.output_revision.wrapping_add(1);
    }

    fn push_output(&mut self, text: &str) {
        append_bounded_output(&mut self.output, text);
        self.output_revision = self.output_revision.wrapping_add(1);
    }

    fn select_session(
        &mut self,
        session: Option<String>,
        transport: Option<HttpTransport>,
        streams: Option<RemoteStreamMux>,
    ) {
        if self.session == session {
            return;
        }
        self.detach();
        self.session = session;
        self.items.clear();
        self.selected = None;
        self.inventory = None;
        self.environment = None;
        self.clear_output();
        self.text.clear();
        self.title.clear();
        self.snapshot_status.clear();
        self.state_status.clear();
        self.ack_status.clear();
        self.error = None;
        self.cols = 80;
        self.rows = 24;
        if let Some(transport) = transport {
            let _ = streams;
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

    fn attach(&mut self, streams: RemoteStreamMux) {
        let (Some(session), Some(id)) = (self.session.clone(), self.selected.clone()) else {
            return;
        };
        self.detach();
        self.clear_output();
        self.snapshot_status.clear();
        self.state_status.clear();
        self.ack_status.clear();
        self.error = None;
        self.status = "Attaching…".to_owned();
        // The bounded handoff stops a stalled UI from retaining an unbounded frame history.
        let (sender, receiver) = mpsc::sync_channel(32);
        let (stop_sender, stop_receiver) = mpsc::channel();
        self.workers.push(std::thread::spawn(move || {
            let result = terminal::attach(&streams, session, id);
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
                                self.clear_output();
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
                                self.clear_output();
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
                    let limits = self.environment.as_ref().map(|environment| TerminalInputLimits {
                        max_input_bytes: environment.max_input_bytes as usize,
                        max_buffered_bytes: environment.max_buffered_bytes as usize,
                        max_cols: environment.max_cols,
                        max_rows: environment.max_rows,
                    });
                    match limits {
                        Some(limits) => {
                            self.input = Some(TerminalWriter::new(input, limits));
                            self.status = "Attached — waiting for snapshot".to_owned();
                        }
                        None => {
                            self.error =
                                Some("Terminal limits are unavailable; refresh first".to_owned());
                            self.detach();
                        }
                    }
                }
                Ok(TerminalUpdate::Error(error)) => {
                    self.detach();
                    self.error = Some(format!("Terminal stream failed: {error}"));
                }
                Ok(TerminalUpdate::Frame(frame)) => match frame {
                    TerminalFrame::Snapshot(frame) => {
                        self.clear_output();
                        self.push_output(&frame.screen);
                        self.writable = frame.writable;
                        self.snapshot_status = format!("Snapshot #{}", frame.sequence);
                        self.state_status = format!(
                            "State: {:?} · exit: {:?} · writable: {}",
                            frame.info.state, frame.info.exit_code, frame.writable
                        );
                        self.status = "Attached".to_owned();
                        if let (Some(writer), Some(window)) = (&mut self.input, frame.input_window)
                        {
                            writer.set_window(window);
                        }
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
                        self.push_output(&frame.data);
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
        let Some(writer) = &mut self.input else {
            return;
        };
        let result = if resize {
            writer.resize(self.cols, self.rows)
        } else {
            writer.write(&self.text)
        };
        match result {
            Ok(sequence) => {
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

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        transport: Option<HttpTransport>,
        streams: Option<RemoteStreamMux>,
    ) {
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
            self.clear_output();
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
                    if let Some(streams) = streams.clone() {
                        self.attach(streams);
                    }
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
            "ANSI SGR colors and carriage returns are rendered; cursor positioning is not emulated.",
        );
        if self.parsed_revision != self.output_revision {
            self.parsed = terminal_ansi::parse(&self.output);
            self.parsed_revision = self.output_revision;
        }
        let job = terminal_ansi::layout(&self.parsed, ui.available_width());
        egui::ScrollArea::both()
            .id_salt("terminal-output")
            .max_height(180.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(job)
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
    use super::{append_bounded_output, TerminalPanel, TERMINAL_VISIBLE_BYTES};
    use dsh_client_core::terminal::{TerminalInputLimits, TerminalWriter};
    use std::sync::mpsc;

    #[test]
    fn detach_requests_stop_and_releases_input() {
        let (stop, stopped) = mpsc::channel();
        let (input, inputs) = mpsc::channel();
        let mut panel = TerminalPanel::default();
        panel.stop = Some(stop);
        panel.input = Some(TerminalWriter::new(
            input,
            TerminalInputLimits {
                max_input_bytes: 1024,
                max_buffered_bytes: 4096,
                max_cols: 80,
                max_rows: 24,
            },
        ));
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
        append_bounded_output(&mut output, &"x".repeat(TERMINAL_VISIBLE_BYTES));
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
        append_bounded_output(&mut output, "end");
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
        assert!(output.ends_with("end"));
    }

    #[test]
    fn oversized_and_incremental_output_preserve_utf8() {
        let mut output = "old output".to_owned();
        append_bounded_output(&mut output, &"界".repeat(TERMINAL_VISIBLE_BYTES));
        assert!(output.len() <= TERMINAL_VISIBLE_BYTES);
        assert!(output.chars().all(|character| character == '界'));
        append_bounded_output(&mut output, "🙂tail");
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
    List(Vec<SessionSummary>),
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
    streams: Option<RemoteStreamMux>,
    sessions: Vec<SessionSummary>,
    selected_session: Option<String>,
    session_page: Option<SessionPage>,
    page_loading: bool,
    page_error: Option<String>,
    page_receiver: Option<mpsc::Receiver<Result<SessionPage, Error>>>,
    chat_text: String,
    cancel_receiver: Option<mpsc::Receiver<Result<AcceptedResponse, Error>>>,
    attachment: Option<FileUploadValue>,
    attachment_name: String,
    attachment_receiver: Option<mpsc::Receiver<Result<FileUploadValue, String>>>,
    attachment_error: Option<String>,
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
    image_preview: ImagePreview,
    preview_document: Option<TextDocument>,
    preview_image: Option<ImageDocument>,
    preview_path: String,
    preview_loading: bool,
    preview_error: Option<String>,
    preview_receiver: Option<mpsc::Receiver<Result<PreviewDocument, String>>>,
    show_document: bool,
    show_plugins: bool,
    model_catalog: Option<ModelCatalog>,
    model_selection: Option<ModelSelection>,
    model_error: Option<String>,
    model_receiver: Option<mpsc::Receiver<Result<ModelCatalog, Error>>>,
    model_select_receiver: Option<mpsc::Receiver<Result<SelectModelResponse, Error>>>,
    plugins_snapshot: Option<PluginInventorySnapshot>,
    plugins_loading: bool,
    plugins_error: Option<String>,
    plugins_receiver: Option<mpsc::Receiver<Result<PluginInventorySnapshot, Error>>>,
    terminal: TerminalPanel,
    schedules: Vec<serde_json::Value>,
    schedules_session: Option<String>,
    schedules_receiver: Option<mpsc::Receiver<Result<Vec<serde_json::Value>, String>>>,
    schedules_error: Option<String>,
    schedule_delete_receiver: Option<mpsc::Receiver<Result<(), String>>>,
    jobs: Vec<JobView>,
    jobs_session: Option<String>,
    jobs_receiver: Option<mpsc::Receiver<Result<Vec<JobView>, String>>>,
    jobs_error: Option<String>,
    job_selected: Option<String>,
    job_output: String,
    job_status: String,
    job_receiver: Option<mpsc::Receiver<Result<JobFollowFrame, String>>>,
    status: String,
    quitting: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    hide_requested: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    viewport: Arc<Mutex<Option<egui::Context>>>,
    tray_available: bool,
    auto_connect: bool,
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
            streams: None,
            sessions: Vec::new(),
            selected_session: None,
            session_page: None,
            page_loading: false,
            page_error: None,
            page_receiver: None,
            chat_text: String::new(),
            cancel_receiver: None,
            attachment: None,
            attachment_name: String::new(),
            attachment_receiver: None,
            attachment_error: None,
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
            image_preview: ImagePreview::default(),
            preview_document: None,
            preview_image: None,
            preview_path: String::new(),
            preview_loading: false,
            preview_error: None,
            preview_receiver: None,
            show_document: false,
            show_plugins: false,
            model_catalog: None,
            model_selection: None,
            model_error: None,
            model_receiver: None,
            model_select_receiver: None,
            plugins_snapshot: None,
            plugins_loading: false,
            plugins_error: None,
            plugins_receiver: None,
            terminal: TerminalPanel::default(),
            schedules: Vec::new(),
            schedules_session: None,
            schedules_receiver: None,
            schedules_error: None,
            schedule_delete_receiver: None,
            jobs: Vec::new(),
            jobs_session: None,
            jobs_receiver: None,
            jobs_error: None,
            job_selected: None,
            job_output: String::new(),
            job_status: String::new(),
            job_receiver: None,
            status: "Starting Host sidecar…".to_owned(),
            quitting,
            show_requested,
            hide_requested,
            hidden,
            viewport,
            tray_available,
            auto_connect: std::env::var("DSH_DESKTOP_AUTO_CONNECT").as_deref() == Ok("1"),
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

    fn start_remote_events(&mut self, transport: HttpTransport, streams: RemoteStreamMux) {
        let (event_sender, event_receiver) = mpsc::channel();
        let (command_sender, command_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let events = match RemoteEvents::connect(transport, &streams) {
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
                        if matches!(event, RemoteEvent::Ready { .. }) {
                            eprintln!("dsh: native events=ready");
                        }
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
                    SessionUpdate::List(items)
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

    fn refresh_models(&mut self) {
        if self.model_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).catalog());
        });
        self.model_receiver = Some(receiver);
        self.model_error = None;
    }

    fn select_model(&mut self, selection: ModelSelection) {
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.model_error = Some("Select a Session before choosing a model".to_owned());
            return;
        };
        let request = SelectModelRequest {
            session_id: session,
            selection,
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).select(request));
        });
        self.model_select_receiver = Some(receiver);
        self.model_error = None;
    }

    fn refresh_plugins(&mut self) {
        if self.plugins_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.plugins_error = Some("Connect to load plugins".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(PluginsClient::new(&transport).list());
        });
        self.plugins_receiver = Some(receiver);
        self.plugins_loading = true;
        self.plugins_error = None;
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
        let image_path = is_image_path(&path);
        std::thread::spawn(move || {
            let client = WorkspaceFilesClient::new(&transport);
            let scope = WorkspaceFileScope { session_id };
            let result = if image_path {
                read_image(&client, scope, &path)
            } else {
                client
                    .read(
                        scope,
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
                        PreviewDocument::Text(TextDocument {
                            name,
                            host_path: page.stat.absolute_path,
                            text: page.text,
                        })
                    })
                    .map_err(|error| error.to_string())
            };
            let _ = sender.send(result);
        });
        self.preview_receiver = Some(receiver);
        self.preview_loading = true;
        self.preview_error = None;
    }

    fn load_session(&mut self, session_id: String) {
        self.session_title.clear();
        self.show_settings = false;
        self.show_plugins = false;
        self.selected_session = Some(session_id.clone());
        self.terminal.select_session(
            Some(session_id.clone()),
            self.transport.clone(),
            self.streams.clone(),
        );
        self.jobs.clear();
        self.jobs_session = None;
        self.jobs_error = None;
        self.job_selected = None;
        self.job_output.clear();
        self.job_status.clear();
        self.job_receiver = None;
        self.schedules.clear();
        self.schedules_session = None;
        self.schedules_error = None;
        self.schedule_delete_receiver = None;
        self.start_job_list();
        self.start_schedules();
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

    fn pick_attachment(&mut self) {
        let Some(path) = dialog::open_file() else {
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.attachment_error = Some("Connect before attaching a file".to_owned());
            return;
        };
        let Some(session_id) = self.selected_session.clone() else {
            self.attachment_error = Some("Select a Session before attaching a file".to_owned());
            return;
        };
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let (sender, receiver) = mpsc::channel();
        self.attachment_receiver = Some(receiver);
        self.attachment_error = None;
        self.attachment = None;
        self.attachment_name = name.clone();
        std::thread::spawn(move || {
            let result = std::fs::read(&path)
                .map_err(|error| format!("reading {name} failed: {error}"))
                .and_then(|bytes| {
                    FileUploadClient::new(&transport)
                        .upload(&session_id, Some(&name), bytes)
                        .map_err(|error| error.to_string())
                });
            let _ = sender.send(result);
        });
    }

    fn poll_attachment(&mut self) {
        let Some(receiver) = &self.attachment_receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(value)) => {
                self.attachment = Some(value);
                self.attachment_receiver = None;
            }
            Ok(Err(error)) => {
                self.attachment_receiver = None;
                self.attachment_error = Some(format!("Upload failed: {error}"));
                self.attachment_name.clear();
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.attachment_receiver = None;
                self.attachment_error = Some("Upload worker stopped unexpectedly".to_owned());
            }
        }
    }

    fn start_schedules(&mut self) {
        let (Some(session), Some(transport)) = (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        if self.schedules_session.as_deref() == Some(session.as_str()) {
            return;
        }
        self.schedules_session = Some(session.clone());
        let (sender, receiver) = mpsc::channel();
        self.schedules_receiver = Some(receiver);
        self.schedules_error = None;
        std::thread::spawn(move || {
            let request = ScheduleListRequest {
                session_id: session,
            };
            let result = SchedulesClient::new(&transport)
                .list(request)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
    }

    fn delete_schedule(&mut self, id: String) {
        let (Some(session), Some(transport)) = (self.selected_session.clone(), self.transport.clone())
        else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let request = ScheduleDeleteRequest {
                session_id: session,
                id,
            };
            let result = SchedulesClient::new(&transport)
                .delete(request)
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.schedule_delete_receiver = Some(receiver);
    }

    fn poll_schedules(&mut self) {
        loop {
            let update = self
                .schedules_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(records))) => self.schedules = records,
                Some(Ok(Err(error))) => {
                    self.schedules_error = Some(format!("Schedule list failed: {error}"));
                    self.schedules_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.schedules_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
        loop {
            let update = self
                .schedule_delete_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(()))) => {
                    self.schedule_delete_receiver = None;
                    self.schedules_session = None;
                    self.start_schedules();
                    self.status = "Schedule deleted".to_owned();
                }
                Some(Ok(Err(error))) => {
                    self.schedules_error = Some(format!("Delete failed: {error}"));
                    self.schedule_delete_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.schedule_delete_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
    }

    fn start_job_list(&mut self) {
        let (Some(session), Some(mux)) = (self.selected_session.clone(), self.streams.clone())
        else {
            return;
        };
        if self.jobs_session.as_deref() == Some(session.as_str()) {
            return;
        }
        self.jobs_session = Some(session.clone());
        let (sender, receiver) = mpsc::channel();
        self.jobs_receiver = Some(receiver);
        self.jobs_error = None;
        std::thread::spawn(move || match streams::job_list(&mux, &session) {
            Ok(stream) => {
                for frame in stream {
                    let update = match frame {
                        Ok(rows) => Ok(rows.jobs),
                        Err(error) => Err(error.to_string()),
                    };
                    let failed = update.is_err();
                    if sender.send(update).is_err() || failed {
                        break;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
            }
        });
    }

    fn start_job_follow(&mut self, job_id: String) {
        let Some(mux) = self.streams.clone() else {
            return;
        };
        self.job_selected = Some(job_id.clone());
        self.job_output.clear();
        self.job_status = "Opening job…".to_owned();
        let (sender, receiver) = mpsc::channel();
        self.job_receiver = Some(receiver);
        std::thread::spawn(move || {
            let request = streams::JobFollowRequest {
                session_id: None,
                job_id,
                from: None,
            };
            match streams::job_follow(&mux, request) {
                Ok(stream) => {
                    for frame in stream {
                        let update = frame.map_err(|error| error.to_string());
                        let failed = update.is_err();
                        if sender.send(update).is_err() || failed {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error.to_string()));
                }
            }
        });
    }

    fn poll_jobs(&mut self) {
        loop {
            let update = self.jobs_receiver.as_ref().map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(rows))) => self.jobs = rows,
                Some(Ok(Err(error))) => {
                    self.jobs_error = Some(format!("Job list failed: {error}"));
                    self.jobs_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.jobs_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
        loop {
            let update = self.job_receiver.as_ref().map(|receiver| receiver.try_recv());
            match update {
                Some(Ok(Ok(frame))) => match frame {
                    JobFollowFrame::Opened { job, from } => {
                        self.job_status = format!("{} · from byte {from}", job.status);
                    }
                    JobFollowFrame::Output { chunks, next, .. } => {
                        for chunk in chunks {
                            append_bounded_output(&mut self.job_output, &chunk.text);
                        }
                        self.job_status = format!("output to byte {next}");
                    }
                    JobFollowFrame::Status { job } => {
                        self.job_status = job.status;
                    }
                },
                Some(Ok(Err(error))) => {
                    self.job_status = format!("Job output failed: {error}");
                    self.job_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    self.job_receiver = None;
                }
                Some(Err(mpsc::TryRecvError::Empty)) | None => break,
            }
        }
    }

    fn cancel_turn(&mut self) {
        if self.cancel_receiver.is_some() {
            return;
        }
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
        else {
            self.page_error = Some("Select a connected Session before cancelling".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ChatClient::new(&transport).cancel(session));
        });
        self.cancel_receiver = Some(receiver);
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
        let Some(streams) = self.streams.clone() else {
            self.page_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let attachment = self.attachment.clone();
        self.chat_text.clear();
        self.chat_frames.clear();
        self.attachment = None;
        self.attachment_name.clear();
        self.chat_loading = true;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let chat = ChatClient::new(&transport);
            let mut content = vec![PromptContentPart::Text { text }];
            if let Some(attachment) = attachment {
                content.push(PromptContentPart::File {
                    receipt_id: attachment.receipt_id,
                });
            }
            let request = PromptRequest {
                request_id: chat.next_request_id(),
                session_id: session_id.clone(),
                mode: PromptMode::Queue,
                content,
                client_time_zone: None,
            };
            if let Err(error) = chat.prompt(request) {
                let _ = sender.send(Err(error));
                return;
            }
            let request = FollowRequest {
                address: SessionAddress {
                    kind: SessionAddressKind::Session,
                    session_id,
                },
                max_messages: Some(200),
                turn_window: None,
                assistant_stream: Some(true),
            };
            match follow::follow(&streams, request) {
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
        self.poll_attachment();
        self.poll_jobs();
        self.poll_schedules();
        if self.jobs_receiver.is_some()
            || self.job_receiver.is_some()
            || self.schedules_receiver.is_some()
            || self.schedule_delete_receiver.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if self.quitting.load(Ordering::Acquire) {
            self.terminal.detach();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.terminal.select_session(
            self.selected_session.clone(),
            self.transport.clone(),
            self.streams.clone(),
        );
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
                    eprintln!("dsh: native host=ready");
                    if self.auto_connect && self.host_url.is_some() {
                        self.connect();
                    }
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
                    eprintln!("dsh: native auth=ok");
                    let streams = match transport.credential().and_then(|credential| {
                        RemoteStreamMux::connect(transport.base_url().as_str(), credential)
                    }) {
                        Ok(streams) => streams,
                        Err(error) => {
                            eprintln!("dsh: native stream mux failed: {error}");
                            self.connect_receiver = None;
                            self.status = format!("Remote streams unavailable: {error}");
                            return;
                        }
                    };
                    self.host_url = Some(base);
                    self.refresh_models();
                    let approval_probe = matches!(
                        std::env::var("DSH_DESKTOP_PROBE_APPROVAL").as_deref(),
                        Ok("1" | "allow" | "reject" | "delegate")
                    );
                    if approval_probe {
                        // The probe owns the only `$events` reader so one waterfall
                        // answer cannot race a second connected client.
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_approval(probe_transport, probe_mux));
                    } else {
                        self.start_remote_events(transport.clone(), streams.clone());
                    }
                    if std::env::var("DSH_DESKTOP_PROBE_STREAMS").as_deref() == Ok("1") {
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_streams(probe_transport, probe_mux));
                    }
                    self.streams = Some(streams);
                    self.transport = Some(transport);
                    self.connect_receiver = None;
                    self.status = "Connected — loading sessions…".to_owned();
                    self.refresh_sessions();
                }
                Ok(Err(error)) => {
                    self.connect_receiver = None;
                    eprintln!("dsh: native auth=failed: {error}");
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
                    match document {
                        PreviewDocument::Text(document) => {
                            self.preview_document = Some(document);
                            self.preview_image = None;
                        }
                        PreviewDocument::Image(image) => {
                            self.preview_image = Some(image);
                            self.preview_document = None;
                        }
                    }
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
        if let Some(receiver) = &self.cancel_receiver {
            match receiver.try_recv() {
                Ok(Ok(_)) => {
                    self.status = "Turn cancellation requested".to_owned();
                    self.cancel_receiver = None;
                }
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Cancel failed: {error}"));
                    self.cancel_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.page_error = Some("Cancel worker stopped unexpectedly".into());
                    self.cancel_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.model_receiver {
            match receiver.try_recv() {
                Ok(Ok(catalog)) => {
                    if self.model_selection.is_none() {
                        self.model_selection = Some(catalog.default.clone());
                    }
                    self.model_catalog = Some(catalog);
                    self.model_error = None;
                    self.model_receiver = None;
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Could not load models: {error}"));
                    self.model_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Model catalog worker stopped unexpectedly".into());
                    self.model_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.model_select_receiver {
            match receiver.try_recv() {
                Ok(Ok(response)) => {
                    self.model_selection = Some(response.selected);
                    self.model_error = None;
                    self.model_select_receiver = None;
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Model selection failed: {error}"));
                    self.model_select_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Model selection worker stopped unexpectedly".into());
                    self.model_select_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.plugins_receiver {
            match receiver.try_recv() {
                Ok(Ok(snapshot)) => {
                    self.plugins_snapshot = Some(snapshot);
                    self.plugins_loading = false;
                    self.plugins_error = None;
                    self.plugins_receiver = None;
                }
                Ok(Err(error)) => {
                    self.plugins_loading = false;
                    self.plugins_error = Some(format!("Could not load plugins: {error}"));
                    self.plugins_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plugins_loading = false;
                    self.plugins_error = Some("Plugin inventory worker stopped unexpectedly".into());
                    self.plugins_receiver = None;
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
                .selectable_label(!self.show_settings && !self.show_plugins, "Sessions")
                .clicked()
            {
                self.show_settings = false;
                self.show_document = false;
                self.show_plugins = false;
            }
            if ui
                .selectable_label(self.show_settings, "Settings")
                .clicked()
            {
                self.open_settings();
                self.show_document = false;
                self.show_plugins = false;
            }
            if ui
                .selectable_label(self.show_document, "Document")
                .clicked()
            {
                self.show_document = true;
                self.show_settings = false;
                self.show_plugins = false;
            }
            if ui
                .selectable_label(self.show_plugins, "Plugins")
                .clicked()
            {
                self.show_plugins = true;
                self.show_settings = false;
                self.show_document = false;
                if self.plugins_snapshot.is_none() {
                    self.refresh_plugins();
                }
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
                            let now = now_millis();
                            for summary in &self.sessions {
                                let label = session_label(summary);
                                let running = if summary.running { " · running" } else { "" };
                                let blank = if summary.blank { " (blank)" } else { "" };
                                if ui
                                    .selectable_label(
                                        self.selected_session.as_ref()
                                            == Some(&summary.session_id),
                                        format!(
                                            "{label}{running}{blank} · {}",
                                            relative_time(summary.updated_at, now)
                                        ),
                                    )
                                    .on_hover_text(&summary.session_id)
                                    .clicked()
                                {
                                    selected = Some(summary.session_id.clone());
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
            egui::Panel::bottom("schedules-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Schedules ({})", self.schedules.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(error) = &self.schedules_error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        let mut deleted = None;
                        egui::ScrollArea::vertical()
                            .id_salt("schedule-list")
                            .max_height(120.0)
                            .show(ui, |ui| {
                                for record in &self.schedules {
                                    let id = record
                                        .get("id")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or_default()
                                        .to_owned();
                                    let title = record
                                        .get("title")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or(&id);
                                    let kind = record
                                        .get("kind")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("schedule");
                                    let next = record
                                        .get("scheduledAt")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("");
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(format!("{title} · {kind} · {next}"));
                                        if ui
                                            .add_enabled(
                                                self.schedule_delete_receiver.is_none(),
                                                egui::Button::new("Delete"),
                                            )
                                            .clicked()
                                        {
                                            deleted = Some(id.clone());
                                        }
                                    });
                                }
                                if self.schedules.is_empty() && self.schedules_receiver.is_none() {
                                    ui.label("No schedules for this Session.");
                                }
                            });
                        if let Some(id) = deleted {
                            self.delete_schedule(id);
                        }
                    });
            });
        }
        if self.selected_session.is_some() {
            egui::Panel::bottom("jobs-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Jobs ({})", self.jobs.len()))
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(error) = &self.jobs_error {
                            ui.colored_label(egui::Color32::LIGHT_RED, error);
                        }
                        let mut next = None;
                        egui::ScrollArea::vertical()
                            .id_salt("jobs-list")
                            .max_height(90.0)
                            .show(ui, |ui| {
                                for job in &self.jobs {
                                    let selected =
                                        self.job_selected.as_deref() == Some(job.id.as_str());
                                    if ui
                                        .selectable_label(
                                            selected,
                                            format!(
                                                "{} · {} · {}",
                                                job.label, job.kind, job.status
                                            ),
                                        )
                                        .clicked()
                                    {
                                        next = Some(job.id.clone());
                                    }
                                }
                            });
                        if let Some(id) = next {
                            self.start_job_follow(id);
                        }
                        if !self.job_status.is_empty() {
                            ui.small(&self.job_status);
                        }
                        if !self.job_output.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("job-output")
                                .max_height(140.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&self.job_output).monospace(),
                                        )
                                        .selectable(true)
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                    );
                                });
                        }
                    });
            });
        }
        if self.selected_session.is_some() {
            egui::Panel::bottom("terminal-panel").show(ui, |ui| {
                egui::CollapsingHeader::new("Session terminals")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.terminal
            .ui(ui, self.transport.clone(), self.streams.clone());
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
            } else if self.show_plugins {
                ui.horizontal(|ui| {
                    ui.heading("Plugins");
                    if ui
                        .add_enabled(!self.plugins_loading, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh_plugins();
                    }
                    if self.plugins_loading {
                        ui.spinner();
                    }
                });
                if let Some(error) = &self.plugins_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(snapshot) = &self.plugins_snapshot {
                    if snapshot.management_available == Some(false) {
                        ui.label("Plugin management is unavailable for this Host.");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("plugin-inventory")
                        .max_height(360.0)
                        .show(ui, |ui| {
                            if snapshot.entries.is_empty() {
                                ui.label("No configured plugins.");
                            }
                            for entry in &snapshot.entries {
                                let phase = entry.fiber_phase.map_or_else(
                                    || "unknown".to_owned(),
                                    |phase| format!("{phase:?}").to_lowercase(),
                                );
                                let state = if entry.enabled { "enabled" } else { "disabled" };
                                ui.label(format!(
                                    "{} · {state} · {phase}",
                                    entry.module_name
                                ));
                            }
                            if let Some(presets) = &snapshot.agent_presets {
                                ui.separator();
                                ui.heading("Agent presets");
                                for preset in presets {
                                    let name = preset.name.as_deref().unwrap_or(&preset.id);
                                    ui.label(format!("{name} · {} row(s)", preset.rows.len()));
                                }
                            }
                        });
                } else if !self.plugins_loading {
                    ui.label("Open this tab to load the plugin inventory.");
                }
                if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            } else if self.show_document {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preview_path)
                            .hint_text("Host file path")
                            .desired_width(360.0),
                    );
                    if ui
                        .add_enabled(!self.preview_loading, egui::Button::new("Preview"))
                        .clicked()
                    {
                        self.read_document();
                    }
                });
                if self.preview_image.is_some() {
                    self.image_preview.ui(
                        ui,
                        self.preview_image.as_ref(),
                        self.preview_loading,
                        self.preview_error.as_deref(),
                    );
                } else {
                    self.document_preview.ui(
                        ui,
                        self.preview_document.as_ref(),
                        self.preview_loading,
                        self.preview_error.as_deref(),
                    );
                }
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
                ui.horizontal_wrapped(|ui| {
                    let selected = self.model_selection.as_ref().map_or_else(
                        || "Model: loading…".to_owned(),
                        |selection| format!("{} / {}", selection.provider, selection.model),
                    );
                    egui::ComboBox::from_id_salt("model-selection")
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            let mut next = None;
                            if let Some(catalog) = &self.model_catalog {
                                for group in &catalog.groups {
                                    ui.label(egui::RichText::new(&group.name).strong());
                                    for model in &group.models {
                                        let selection = ModelSelection {
                                            provider: group.id.clone(),
                                            model: model.id.clone(),
                                            reasoning_effort: model
                                                .reasoning
                                                .as_ref()
                                                .and_then(|reasoning| {
                                                    reasoning.default_effort.clone()
                                                }),
                                        };
                                        let current = self
                                            .model_selection
                                            .as_ref()
                                            .is_some_and(|current| {
                                                current.provider == selection.provider
                                                    && current.model == selection.model
                                            });
                                        if ui
                                            .selectable_label(
                                                current,
                                                format!("{} · {}", model.name, model.id),
                                            )
                                            .clicked()
                                        {
                                            next = Some(selection);
                                        }
                                    }
                                }
                            }
                            if let Some(selection) = next {
                                self.select_model(selection);
                            }
                        });
                    if ui
                        .add_enabled(self.model_receiver.is_none(), egui::Button::new("Reload"))
                        .clicked()
                    {
                        self.refresh_models();
                    }
                    let selected_model = match (&self.model_catalog, &self.model_selection) {
                        (Some(catalog), Some(selection)) => {
                            catalog_model(catalog, selection).cloned()
                        }
                        _ => None,
                    };
                    if let (Some(selection), Some(reasoning)) = (
                        self.model_selection.clone(),
                        selected_model.and_then(|model| model.reasoning),
                    ) {
                        let current = selection
                            .reasoning_effort
                            .clone()
                            .or_else(|| reasoning.default_effort.clone())
                            .unwrap_or_else(|| "default".to_owned());
                        let mut next_effort = None;
                        egui::ComboBox::from_id_salt("model-effort")
                            .selected_text(format!("Effort: {current}"))
                            .show_ui(ui, |ui| {
                                for effort in &reasoning.efforts {
                                    if ui
                                        .selectable_label(current == effort.id, &effort.name)
                                        .on_hover_text(
                                            effort.description.as_deref().unwrap_or_default(),
                                        )
                                        .clicked()
                                    {
                                        next_effort = Some(effort.id.clone());
                                    }
                                }
                            });
                        if let Some(effort) = next_effort {
                            let mut next = selection;
                            next.reasoning_effort = Some(effort);
                            self.select_model(next);
                        }
                    }
                });
                if let Some(error) = &self.model_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.add_enabled(
                    !self.chat_loading,
                    egui::TextEdit::multiline(&mut self.chat_text)
                        .hint_text("Message")
                        .desired_rows(3),
                );
                ui.horizontal_wrapped(|ui| {
                    let uploading = self.attachment_receiver.is_some();
                    if ui
                        .add_enabled(
                            !uploading && !self.chat_loading,
                            egui::Button::new("Attach file…"),
                        )
                        .clicked()
                    {
                        self.pick_attachment();
                    }
                    if !self.attachment_name.is_empty() {
                        ui.label(&self.attachment_name);
                    }
                    if uploading {
                        ui.spinner();
                    }
                    if self.attachment.is_some() && ui.button("Remove").clicked() {
                        self.attachment = None;
                        self.attachment_name.clear();
                    }
                });
                if let Some(error) = &self.attachment_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.chat_loading
                                && !self.chat_text.trim().is_empty()
                                && self.attachment_receiver.is_none(),
                            egui::Button::new("Send"),
                        )
                        .clicked()
                    {
                        self.send_prompt();
                    }
                    if ui
                        .add_enabled(
                            self.chat_loading && self.cancel_receiver.is_none(),
                            egui::Button::new("Cancel turn"),
                        )
                        .clicked()
                    {
                        self.cancel_turn();
                    }
                });
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

/// Finds the catalog model named by a selection.
fn catalog_model<'a>(
    catalog: &'a ModelCatalog,
    selection: &ModelSelection,
) -> Option<&'a ModelCatalogModel> {
    catalog
        .groups
        .iter()
        .filter(|group| group.id == selection.provider)
        .flat_map(|group| group.models.iter())
        .find(|model| model.id == selection.model)
}

/// Current Unix time in milliseconds.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Display label for a session: its projected title, else the identity.
fn session_label(summary: &SessionSummary) -> String {
    summary
        .projections
        .as_ref()
        .and_then(|projections| projections.get("title"))
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.is_empty())
        .map_or_else(|| summary.session_id.clone(), str::to_owned)
}

/// Relative age of a session update, tolerant of second- or millisecond-scale times.
fn relative_time(updated_at: i64, now: i64) -> String {
    let updated_at = if updated_at < 1_000_000_000_000 {
        updated_at.saturating_mul(1000)
    } else {
        updated_at
    };
    let seconds = now.saturating_sub(updated_at) / 1000;
    match seconds {
        ..=59 => "just now".to_owned(),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        86_400..=2_591_999 => format!("{}d ago", seconds / 86_400),
        _ => format!("{}mo ago", seconds / 2_592_000),
    }
}

/// Largest image preview read; larger files are rejected instead of fetched.
const PREVIEW_IMAGE_LIMIT: usize = 16 * 1024 * 1024;
/// Largest decoded image side accepted by the preview.
const PREVIEW_IMAGE_MAX_DIMENSION: u32 = 8192;

/// True for extensions the preview decodes as an image.
fn is_image_path(path: &str) -> bool {
    let Some((_, extension)) = path.rsplit_once('.') else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
    )
}

/// Reads and decodes one workspace image for display.
fn read_image(
    client: &WorkspaceFilesClient<'_>,
    scope: WorkspaceFileScope,
    path: &str,
) -> Result<PreviewDocument, String> {
    let bytes = client
        .read_bytes(
            scope,
            path,
            WorkspaceByteReadOptions {
                range: Some(WorkspaceByteRange {
                    offset: None,
                    length: Some(PREVIEW_IMAGE_LIMIT as u64),
                }),
                base_file: None,
            },
        )
        .map_err(|error| error.to_string())?;
    if !bytes.eof {
        return Err(format!(
            "image is larger than {} MiB",
            PREVIEW_IMAGE_LIMIT / (1024 * 1024)
        ));
    }
    let name = bytes
        .stat
        .absolute_path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned();
    decode_preview_image(name, bytes.stat.absolute_path, &bytes.data)
}

/// Decodes exact image bytes into a preview document.
fn decode_preview_image(
    name: String,
    host_path: String,
    data: &[u8],
) -> Result<PreviewDocument, String> {
    let decoded =
        image::load_from_memory(data).map_err(|error| format!("image decoding failed: {error}"))?;
    if decoded.width() > PREVIEW_IMAGE_MAX_DIMENSION
        || decoded.height() > PREVIEW_IMAGE_MAX_DIMENSION
    {
        return Err(format!(
            "image exceeds {PREVIEW_IMAGE_MAX_DIMENSION} pixels on a side"
        ));
    }
    let rgba = decoded.to_rgba8();
    Ok(PreviewDocument::Image(ImageDocument {
        name,
        host_path,
        size: [rgba.width() as usize, rgba.height() as usize],
        rgba: rgba.into_raw(),
    }))
}

/// Model the probe requests: the DeepSeek API-key route when it exists, else the catalog default.
fn probe_model_selection(catalog: &ModelCatalog) -> Option<ModelSelection> {
    catalog
        .groups
        .iter()
        .find(|group| group.id == "deepseek-official")
        .and_then(|group| group.models.first())
        .map(|model| ModelSelection {
            provider: "deepseek-official".to_owned(),
            model: model.id.clone(),
            reasoning_effort: model
                .reasoning
                .as_ref()
                .and_then(|reasoning| reasoning.default_effort.clone()),
        })
        .or_else(|| {
            catalog.groups.iter().find_map(|group| {
                group.models.first().map(|model| ModelSelection {
                    provider: group.id.clone(),
                    model: model.id.clone(),
                    reasoning_effort: model
                        .reasoning
                        .as_ref()
                        .and_then(|reasoning| reasoning.default_effort.clone()),
                })
            })
        })
}

/// Drives one live approval round-trip: prompt the mock model, allow the
/// escalated bash call through `$events`, and wait for the marker it writes.
///
/// Runs only when `DSH_DESKTOP_PROBE_APPROVAL=1`; `DSH_APPROVAL_PROBE_MARKER`
/// names the file the approved command writes.
fn probe_approval(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::{Duration, Instant};
    let marker = match std::env::var("DSH_APPROVAL_PROBE_MARKER") {
        Ok(value) if !value.is_empty() => value,
        _ => {
            eprintln!("dsh: native approval=failed: DSH_APPROVAL_PROBE_MARKER is required");
            return;
        }
    };
    let mode = std::env::var("DSH_DESKTOP_PROBE_APPROVAL").unwrap_or_default();
    let decision = match mode.as_str() {
        "reject" => Some(ApprovalDecision::Rejected),
        "delegate" => None,
        _ => Some(ApprovalDecision::AllowedOnce),
    };
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native approval=failed: session create: {error}");
            return;
        }
    };
    let model = match ModelClient::new(&transport).catalog() {
        Ok(catalog) => match probe_model_selection(&catalog) {
            Some(selection) => selection,
            None => {
                eprintln!("dsh: native approval=failed: empty model catalog");
                return;
            }
        },
        Err(error) => {
            eprintln!("dsh: native approval=failed: model catalog: {error}");
            return;
        }
    };
    eprintln!(
        "dsh: native approval=model {} / {}",
        model.provider, model.model
    );
    if let Err(error) = ModelClient::new(&transport).select(SelectModelRequest {
        session_id: session.clone(),
        selection: model,
    }) {
        eprintln!("dsh: native approval=failed: select model: {error}");
        return;
    }
    let events = match RemoteEvents::connect(transport.clone(), &mux) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("dsh: native approval=failed: events: {error}");
            return;
        }
    };
    let chat = ChatClient::new(&transport);
    let request = PromptRequest {
        request_id: chat.next_request_id(),
        session_id: session.clone(),
        mode: PromptMode::Queue,
        content: vec![PromptContentPart::Text {
            text: "Run the escalated approval probe command.".to_owned(),
        }],
        client_time_zone: None,
    };
    if let Err(error) = chat.prompt(request) {
        eprintln!("dsh: native approval=failed: prompt: {error}");
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut asked = false;
    let mut answered = false;
    let mut roundtrip_reported = false;
    while Instant::now() < deadline {
        if !roundtrip_reported && std::path::Path::new(&marker).exists() {
            roundtrip_reported = true;
            eprintln!("dsh: native approval=roundtrip-ok");
            probe_job_follow(&mux, &session);
        }
        match events.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(RemoteEvent::Waterfall(invocation))) => {
                if invocation.event == "user-questions/request" {
                    let answer = user_question_answer(&invocation.request);
                    match events.submit(
                        &invocation,
                        RemoteEventOutcome::Result { value: answer },
                    ) {
                        Ok(()) => {
                            eprintln!("dsh: native question=answered");
                            if !roundtrip_reported
                                && std::path::Path::new(&marker).exists()
                            {
                                eprintln!("dsh: native approval=roundtrip-ok");
                                probe_job_follow(&mux, &session);
                            }
                            if probe_question_result(&mux, &session) {
                                eprintln!("dsh: native question=answer-ok");
                            } else {
                                eprintln!("dsh: native question=failed: no answered result");
                            }
                        }
                        Err(error) => {
                            eprintln!("dsh: native question=failed: {error}");
                        }
                    }
                    return;
                }
                if invocation.event != "approval/request" {
                    continue;
                }
                let pending = match PendingApproval::from_invocation(invocation) {
                    Ok(pending) => pending,
                    Err(error) => {
                        eprintln!("dsh: native approval=failed: decode request: {error}");
                        continue;
                    }
                };
                if !asked {
                    asked = true;
                    eprintln!(
                        "dsh: native approval=asked tool={}",
                        pending.request.tool_name
                    );
                }
                let submitted = match decision {
                    Some(decision) => pending.answer(&events, decision),
                    None => pending.delegate(&events),
                };
                match submitted {
                    Ok(()) => {
                        if !answered {
                            answered = true;
                            match decision {
                                Some(ApprovalDecision::AllowedOnce) => {
                                    eprintln!("dsh: native approval=allowed-once")
                                }
                                Some(ApprovalDecision::Rejected) => {
                                    eprintln!("dsh: native approval=rejected")
                                }
                                None => eprintln!("dsh: native approval=delegated"),
                            }
                        }
                        if decision != Some(ApprovalDecision::AllowedOnce) {
                            let (expected, label) = match decision {
                                Some(ApprovalDecision::Rejected) => ("rejected", "rejected-ok"),
                                _ => ("unavailable", "delegated-ok"),
                            };
                            if probe_decided(&mux, &session, expected) {
                                eprintln!("dsh: native approval={label}");
                            } else {
                                eprintln!("dsh: native approval=failed: no {expected} decision logged");
                            }
                            if std::path::Path::new(&marker).exists() {
                                eprintln!(
                                    "dsh: native approval=failed: command ran despite {expected}"
                                );
                            }
                            return;
                        }
                    }
                    Err(error) => eprintln!("dsh: native approval=answer-failed: {error}"),
                }
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => {
                eprintln!("dsh: native approval=failed: events closed: {error}");
                return;
            }
        }
    }
    eprintln!(
        "dsh: native approval=failed: timeout asked={asked} answered={answered} marker={}",
        std::path::Path::new(&marker).exists()
    );
}

/// Reads the current session snapshot, retrying after a failed open.
fn probe_snapshot(
    mux: &RemoteStreamMux,
    session: &str,
) -> Option<follow::SnapshotFrame> {
    let request = FollowRequest {
        address: SessionAddress {
            kind: SessionAddressKind::Session,
            session_id: session.to_owned(),
        },
        max_messages: Some(200),
        turn_window: None,
        assistant_stream: None,
    };
    let stream = follow::follow(mux, request).ok()?;
    for frame in stream {
        match frame {
            Ok(FollowFrame::Snapshot(snapshot)) => return Some(snapshot),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Waits for the audit event the Host appends after a decision.
fn probe_decided(mux: &RemoteStreamMux, session: &str, expected: &str) -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(snapshot) = probe_snapshot(mux, session) {
            for entry in &snapshot.records {
                let event = &entry.event;
                if event.event_type == "approval/decided"
                    && event.data.get("outcome").and_then(serde_json::Value::as_str)
                        == Some(expected)
                {
                    return true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Builds an answer that selects the first option of each asked question.
fn user_question_answer(request: &serde_json::Map<String, serde_json::Value>) -> AskUserQuestionAnswer {
    let answers = request
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| {
                    let id = question
                        .get("id")
                        .and_then(serde_json::Value::as_str)?
                        .to_owned();
                    let selected = question
                        .get("options")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|options| options.first())
                        .and_then(|option| option.get("label"))
                        .and_then(serde_json::Value::as_str)
                        .map(|label| vec![label.to_owned()])
                        .unwrap_or_default();
                    Some(AskUserQuestionAnswerItem {
                        id,
                        selected,
                        custom: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    AskUserQuestionAnswer { answers }
}

/// Waits for the answered tool result in the durable log.
fn probe_question_result(mux: &RemoteStreamMux, session: &str) -> bool {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(snapshot) = probe_snapshot(mux, session) {
            for entry in &snapshot.records {
                let event = &entry.event;
                if event.event_type == "tool/result" && event.data.to_string().contains("Yes") {
                    return true;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Follows the background job the mock model starts after the approved turn./// Follows the background job the mock model starts after the approved turn.
fn probe_job_follow(mux: &RemoteStreamMux, session: &str) {
    use std::time::{Duration, Instant};
    let mut list = match streams::job_list(mux, session) {
        Ok(list) => list,
        Err(error) => {
            eprintln!("dsh: native job=failed: list open: {error}");
            return;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut job_id = None;
    while Instant::now() < deadline {
        match list.recv_timeout(Duration::from_millis(250)) {
            Some(Ok(frame)) => {
                if let Some(job) = frame.jobs.first() {
                    job_id = Some(job.id.clone());
                    break;
                }
            }
            Some(Err(error)) => {
                eprintln!("dsh: native job=failed: list: {error}");
                return;
            }
            None => {}
        }
    }
    let Some(job_id) = job_id else {
        eprintln!("dsh: native job=failed: no job listed");
        return;
    };
    let request = streams::JobFollowRequest {
        session_id: Some(session.to_owned()),
        job_id,
        from: Some(0),
    };
    let mut follow = match streams::job_follow(mux, request) {
        Ok(follow) => follow,
        Err(error) => {
            eprintln!("dsh: native job=failed: follow open: {error}");
            return;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut output = String::new();
    while Instant::now() < deadline {
        match follow.recv_timeout(Duration::from_millis(250)) {
            Some(Ok(JobFollowFrame::Output { chunks, .. })) => {
                for chunk in chunks {
                    output.push_str(&chunk.text);
                }
                if output.contains("job-output-ok") {
                    eprintln!("dsh: native job=follow-ok");
                    return;
                }
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                eprintln!("dsh: native job=failed: follow: {error}");
                return;
            }
            None => {}
        }
    }
    eprintln!("dsh: native job=failed: output marker missing");
}

/// Reports one probed stream: its opening frame, its failure, or a timeout.
fn probe_first<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    opened: Result<FrameStream<T>, Error>,
    timeout: std::time::Duration,
) {
    match opened {
        Ok(mut stream) => match stream.recv_timeout(timeout) {
            Some(Ok(_)) => eprintln!("dsh: native stream {endpoint}=ok"),
            Some(Err(error)) => eprintln!("dsh: native stream {endpoint}=failed: {error}"),
            None => {
                eprintln!("dsh: native stream {endpoint}=failed: no frame before probe timeout")
            }
        },
        Err(error) => eprintln!("dsh: native stream {endpoint}=failed: {error}"),
    }
}

/// Opens the stream endpoints that need no live id and reports their first frame.
///
/// Runs only when `DSH_DESKTOP_PROBE_STREAMS=1`; the lane's GUI probe reads the
/// `dsh: native stream` lines to prove the endpoints on a real Host.
fn probe_streams(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::Duration;
    let timeout = Duration::from_secs(20);
    probe_first("workspace/follow", streams::workspace_follow(&mux), timeout);
    probe_first("session/control", streams::session_control(&mux), timeout);
    probe_first("account/watch", streams::account_watch(&mux), timeout);
    probe_first(
        "remoteAccess/follow",
        streams::remote_access_follow(&mux),
        timeout,
    );
    // The expiry stream carries no opening frame; opening it is the whole check.
    match streams::account_watch_expiry(&mux) {
        Ok(_) => eprintln!("dsh: native stream account/watchExpiry=ok"),
        Err(error) => eprintln!("dsh: native stream account/watchExpiry=failed: {error}"),
    }
    let session_id = match SessionViewer::new(&transport).list(None) {
        Ok(items) if !items.is_empty() => Some(items[0].session_id.clone()),
        Ok(_) => match SessionsClient::new(&transport).create(CreateSessionRequest {
            workspace_id: None,
            cwd: None,
            session_id: None,
            agent_preset: None,
        }) {
            Ok(session) => Some(session.session_id),
            Err(error) => {
                eprintln!("dsh: native stream job/list=failed: session create: {error}");
                None
            }
        },
        Err(error) => {
            eprintln!("dsh: native stream job/list=failed: {error}");
            None
        }
    };
    match &session_id {
        Some(id) => {
            probe_first("job/list", streams::job_list(&mux, id), timeout);
            let scope = WorkspaceFileScope {
                session_id: id.clone(),
            };
            probe_first(
                "workspaceFiles/changes",
                streams::workspace_file_changes(&mux, &scope, "cordis.yml"),
                timeout,
            );
            let request = ScheduleListRequest {
                session_id: id.clone(),
            };
            match SchedulesClient::new(&transport).list(request) {
                Ok(records) => eprintln!("dsh: native schedules={}", records.len()),
                Err(error) => eprintln!("dsh: native schedules=failed: {error}"),
            }
        }
        None => {
            eprintln!("dsh: native stream job/list=skipped: no sessions");
            eprintln!("dsh: native stream workspaceFiles/changes=skipped: no sessions");
        }
    }
    for endpoint in ["job/follow", "userQuestions/attachWait"] {
        eprintln!("dsh: native stream {endpoint}=skipped: needs a live id");
    }
    match PluginsClient::new(&transport).list() {
        Ok(snapshot) => eprintln!("dsh: native plugins={}", snapshot.entries.len()),
        Err(error) => eprintln!("dsh: native plugins=failed: {error}"),
    }
    match ModelClient::new(&transport).catalog() {
        Ok(catalog) => eprintln!(
            "dsh: native models={} failures={}",
            catalog.groups.iter().map(|group| group.models.len()).sum::<usize>(),
            catalog.failures.len()
        ),
        Err(error) => eprintln!("dsh: native models=failed: {error}"),
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

#[cfg(test)]
mod preview_path_tests {
    use super::{decode_preview_image, is_image_path};
    use crate::document_preview::PreviewDocument;
    use std::io::Cursor;

    #[test]
    fn recognizes_image_extensions_case_insensitively() {
        for path in [
            "shot.png",
            "photo.JPG",
            "scan.jpeg",
            "anim.GIF",
            "wide.webp",
            "old.bmp",
            "icon.ico",
        ] {
            assert!(is_image_path(path), "{path}");
        }
        for path in ["notes.md", "render.png.txt", "png", "a.pngx", ""] {
            assert!(!is_image_path(path), "{path}");
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_png_bytes_into_rgba_pixels() {
        let PreviewDocument::Image(document) =
            decode_preview_image("shot.png".into(), "/host/shot.png".into(), &png(3, 2)).unwrap()
        else {
            panic!("expected image preview")
        };
        assert_eq!(document.size, [3, 2]);
        assert_eq!(document.rgba.len(), 3 * 2 * 4);
        assert_eq!(&document.rgba[..4], &[1, 2, 3, 255]);
        assert_eq!(document.host_path, "/host/shot.png");
    }

    #[test]
    fn rejects_bytes_that_are_not_images() {
        let error = decode_preview_image("notes.txt".into(), "/host/notes.txt".into(), b"text")
            .unwrap_err();
        assert!(error.starts_with("image decoding failed: "), "{error}");
    }
}

#[cfg(test)]
mod session_label_tests {
    use super::{relative_time, session_label};
    use dsh_client_core::session::SessionSummary;

    fn summary(projections: Option<serde_json::Value>) -> SessionSummary {
        SessionSummary {
            agent_available: true,
            format_status: None,
            session_id: "session-1".into(),
            updated_at: 1_760_000_000_000,
            running: false,
            blank: false,
            parent_session_id: None,
            origin: None,
            cwd: None,
            projections,
        }
    }

    #[test]
    fn prefers_the_projected_title_and_falls_back_to_the_id() {
        assert_eq!(
            session_label(&summary(Some(serde_json::json!({"title": "Roadmap"})))),
            "Roadmap"
        );
        for projections in [
            Some(serde_json::json!({"title": null})),
            Some(serde_json::json!({"title": ""})),
            Some(serde_json::json!({})),
            None,
        ] {
            assert_eq!(session_label(&summary(projections)), "session-1");
        }
    }

    #[test]
    fn formats_relative_age_in_seconds_and_milliseconds() {
        let now = 1_760_000_000_000;
        for (updated, expected) in [
            (now, "just now"),
            (now - 30_000, "just now"),
            (now - 90_000, "1m ago"),
            (now - 7_200_000, "2h ago"),
            (now - 172_800_000, "2d ago"),
            (now - 5_184_000_000, "2mo ago"),
        ] {
            assert_eq!(relative_time(updated, now), expected);
        }
        // Second-scale timestamps convert before differencing.
        assert_eq!(relative_time(1_760_000_000 - 120, now), "2m ago");
        // Clock skew stays "just now" instead of underflowing.
        assert_eq!(relative_time(now + 60_000, now), "just now");
    }
}

#[cfg(test)]
mod model_catalog_tests {
    use super::catalog_model;
    use dsh_client_core::models::{
        ModelCatalog, ModelCatalogModel, ModelProviderGroup, ModelSelection,
    };

    fn catalog() -> ModelCatalog {
        ModelCatalog {
            default: ModelSelection {
                provider: "deepseek".into(),
                model: "chat".into(),
                reasoning_effort: None,
            },
            routable_providers: vec!["deepseek".into()],
            groups: vec![
                ModelProviderGroup {
                    id: "deepseek".into(),
                    name: "DeepSeek".into(),
                    models: vec![ModelCatalogModel {
                        id: "chat".into(),
                        name: "Chat".into(),
                        description: None,
                        reasoning: None,
                    }],
                },
                ModelProviderGroup {
                    id: "other".into(),
                    name: "Other".into(),
                    models: vec![ModelCatalogModel {
                        id: "chat".into(),
                        name: "Other Chat".into(),
                        description: None,
                        reasoning: None,
                    }],
                },
            ],
            failures: Vec::new(),
        }
    }

    #[test]
    fn matches_the_model_within_the_selected_provider() {
        let catalog = catalog();
        let selection = ModelSelection {
            provider: "other".into(),
            model: "chat".into(),
            reasoning_effort: None,
        };
        assert_eq!(
            catalog_model(&catalog, &selection).map(|model| model.name.as_str()),
            Some("Other Chat")
        );
        let missing = ModelSelection {
            provider: "other".into(),
            model: "absent".into(),
            reasoning_effort: None,
        };
        assert!(catalog_model(&catalog, &missing).is_none());
    }
}

#[cfg(test)]
mod probe_model_tests {
    use super::probe_model_selection;
    use dsh_client_core::models::{
        ModelCatalog, ModelCatalogModel, ModelProviderGroup, ModelSelection,
    };

    fn model(id: &str) -> ModelCatalogModel {
        ModelCatalogModel {
            id: id.into(),
            name: id.into(),
            description: None,
            reasoning: None,
        }
    }

    fn catalog_with(groups: Vec<ModelProviderGroup>) -> ModelCatalog {
        ModelCatalog {
            default: ModelSelection {
                provider: "openai-codex".into(),
                model: "gpt".into(),
                reasoning_effort: None,
            },
            routable_providers: Vec::new(),
            groups,
            failures: Vec::new(),
        }
    }

    #[test]
    fn prefers_the_deepseek_api_key_route() {
        let catalog = catalog_with(vec![
            ModelProviderGroup {
                id: "openai-codex".into(),
                name: "Codex".into(),
                models: vec![model("gpt")],
            },
            ModelProviderGroup {
                id: "deepseek-official".into(),
                name: "DeepSeek".into(),
                models: vec![model("deepseek-flash")],
            },
        ]);
        let selection = probe_model_selection(&catalog).unwrap();
        assert_eq!(selection.provider, "deepseek-official");
        assert_eq!(selection.model, "deepseek-flash");
    }

    #[test]
    fn falls_back_to_the_first_catalog_model() {
        let catalog = catalog_with(vec![ModelProviderGroup {
            id: "other".into(),
            name: "Other".into(),
            models: vec![model("first")],
        }]);
        assert_eq!(
            probe_model_selection(&catalog).map(|selection| selection.model),
            Some("first".to_owned())
        );
        assert!(probe_model_selection(&catalog_with(Vec::new())).is_none());
    }
}
