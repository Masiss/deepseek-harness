//! Native desktop shell: a pure egui window that reads the Host session inventory through client-core.

mod composer_input;
mod dialog;
mod document_preview;
mod fonts;
mod host;
mod layout;
mod question_card;
mod restart;
mod session_list;
mod settings_panel;
mod terminal_ansi;
mod terminal_input;
mod transcript;
mod transcript_view;
mod ui_preferences;
mod updater;

use document_preview::{
    DocumentPreview, ImageDocument, ImagePreview, PreviewDocument, TextDocument,
};
use dsh_client_core::{
    approvals::{ApprovalDecision, PendingApproval},
    chat::{AcceptedResponse, ChatClient, PromptContentPart, PromptMode, PromptRequest},
    commands::{CommandExecution, CommandResult, CommandsClient},
    follow::{self, FollowFrame, FollowRequest},
    goals::{
        CreateGoalRequest, EditGoalRequest, GoalActivation, GoalPhase, GoalRef, GoalView,
        GoalsClient,
    },
    marketplace::MarketplaceClient,
    message_feedback::{MessageFeedbackClient, MessageFeedbackItem, MessageFeedbackPutRequest},
    models::{
        ModelCatalog, ModelCatalogModel, ModelClient, ModelSelection, SelectModelRequest,
        SelectModelResponse,
    },
    office_to_pdf::OfficeToPdfClient,
    plugins::{BundleInfo, ChangeResult, ManagedPlugin, PluginInventorySnapshot, PluginsClient},
    remote_events::{RemoteEvent, RemoteEventError, RemoteEventOutcome, RemoteEvents},
    remote_stream::RemoteStreamMux,
    schedules::{ScheduleDeleteRequest, ScheduleListRequest, SchedulesClient},
    session::{
        SessionAddress, SessionAddressKind, SessionPage, SessionPageRequest, SessionSummary,
        SessionTurnWindow, SessionViewer, MAX_SAFE_SEQUENCE,
    },
    sessions::{
        CreateSessionRequest, ForkSessionRequest, RenameSessionRequest, SessionSearchResponse,
        SessionsClient,
    },
    settings::{SettingsClient, SettingsDescribeValue, SettingsNamespaceView},
    skills::{SkillListResponse, SkillsClient},
    stream::FrameStream,
    streams::{self, JobFollowFrame, JobView},
    terminal::{
        self, TerminalClient, TerminalCloseRequest, TerminalCreateRequest, TerminalEnvironment,
        TerminalFrame, TerminalInput, TerminalInputLimits, TerminalRenameRequest, TerminalWriter,
        WebTerminalInfo,
    },
    transport::HttpTransport,
    upload::{FileUploadClient, FileUploadValue},
    user_questions::{
        AskUserQuestionAnswer, AskUserQuestionAnswerItem, PendingQuestion, UserQuestionsClient,
    },
    workspace_files::WorkspaceByteReadOptions,
    workspace_files::{
        WorkspaceByteRange, WorkspaceFileRange, WorkspaceFileScope, WorkspaceFilesClient,
    },
    Error,
};
use eframe::egui;
use fs2::FileExt;
use host::Host;
use question_card::{QuestionAction, QuestionCard};
use settings_panel::{SettingsAction, SettingsPanel};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use transcript::{RowKind, Transcript};
use transcript_view::render_transcript;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};
use ui_preferences::{UiPreferences, UiPreferencesAction, UiPreferencesPanel};

enum PluginAction {
    SetPlugin { id: String, enabled: bool },
    SetBundle { name: String, enabled: bool },
}

enum ApprovalCommand {
    Answer(PendingApproval, ApprovalDecision),
    Delegate(dsh_client_core::remote_events::RemoteEventInvocation),
    AnswerQuestion(PendingQuestion, AskUserQuestionAnswer),
}

enum ApprovalChoice {
    Answer(ApprovalDecision),
    Delegate,
}

enum GoalMutation {
    Create(CreateGoalRequest),
    Edit(GoalRef, EditGoalRequest),
    Pause(GoalRef),
    Resume(GoalRef),
    Complete(GoalRef),
    Clear(GoalRef),
}

const TERMINAL_VISIBLE_BYTES: usize = 128 * 1024;

/// One file the model declared as a final deliverable.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeliverableFile {
    path: String,
    description: Option<String>,
}

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

/// What the running file picker is collecting.
#[derive(Clone, Copy)]
enum DialogPurpose {
    Attach,
    Open,
    Save,
}

enum TerminalUpdate {
    Attached(mpsc::Sender<TerminalInput>),
    Frame(Box<TerminalFrame>),
    Error(String),
}

/// Snapshot, bundles, and managed plugins returned by the inventory workers.
type PluginInventoryResult =
    Result<(PluginInventorySnapshot, Vec<BundleInfo>, Vec<ManagedPlugin>), Error>;

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
    parsed_width: u32,
    text: String,
    title: String,
    status: String,
    snapshot_status: String,
    state_status: String,
    ack_status: String,
    error: Option<String>,
    writable: bool,
    pending: Option<u64>,
    queued_input: std::collections::VecDeque<String>,
    queued_input_bytes: usize,
    cols: u32,
    rows: u32,
    /// Input modes the attached program enabled.
    modes: terminal_ansi::InputModes,
    mode_tracker: terminal_ansi::InputModeTracker,
    /// Last painted grid rectangle, used to map pointer positions.
    grid_rect: Option<egui::Rect>,
}

/// Removes one Office preview's PDF and raster files when conversion finishes.
struct PreviewScratchDirectory(PathBuf);

impl Drop for PreviewScratchDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("dsh: could not remove Office preview scratch directory: {error}");
            }
        }
    }
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
        self.queued_input.clear();
        self.queued_input_bytes = 0;
        self.modes = terminal_ansi::InputModes::default();
        self.mode_tracker.reset();
        self.grid_rect = None;
        self.status = "Detached (shell remains alive)".to_owned();
    }

    /// Answers a program's device or cursor query through the same uplink.
    fn reply_to_program(&mut self, replies: &str) {
        if !self.writable || self.pending.is_some() {
            self.status = "A terminal reply was dropped while input was unacknowledged".to_owned();
            return;
        }
        let Some(writer) = &mut self.input else {
            return;
        };
        match writer.write(replies) {
            Ok(sequence) => {
                self.pending = Some(sequence);
                self.status = format!("Awaiting ack #{sequence} (terminal reply)");
            }
            Err(error) => {
                self.error = Some(format!("A terminal reply was not queued: {error}"));
            }
        }
    }

    fn clear_output(&mut self) {
        self.output.clear();
        self.output_revision = self.output_revision.wrapping_add(1);
        self.mode_tracker.reset();
        self.modes = terminal_ansi::InputModes::default();
    }

    fn push_output(&mut self, text: &str) {
        self.modes = self.mode_tracker.feed(text);
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
                            Ok(frame) => TerminalUpdate::Frame(Box::new(frame)),
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
                    let limits = self
                        .environment
                        .as_ref()
                        .map(|environment| TerminalInputLimits {
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
                Ok(TerminalUpdate::Frame(frame)) => match *frame {
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
                        if !frame.writable {
                            self.queued_input.clear();
                            self.queued_input_bytes = 0;
                        }
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
                        if let Some(writer) = &mut self.input {
                            writer.acknowledge(frame.sequence);
                        }
                        if self.pending == Some(frame.sequence) {
                            self.pending = None;
                        }
                        self.ack_status = format!("Ack #{} · {:?}", frame.sequence, frame.refusal);
                        self.status = "Attached".to_owned();
                        if let Some(refusal) = frame.refusal {
                            self.error =
                                Some(format!("Input #{} refused: {refusal:?}", frame.sequence));
                            self.queued_input.clear();
                            self.queued_input_bytes = 0;
                        } else {
                            self.flush_queued_input();
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

    /// Writes already-encoded input through the same uplink and ack tracking.
    ///
    /// @param data - Bytes to send exactly as encoded.
    /// @returns Whether the write was queued.
    fn send_encoded(&mut self, data: &str) -> bool {
        if !self.writable || data.is_empty() {
            return false;
        }
        if self.pending.is_some() {
            let limit = self
                .environment
                .as_ref()
                .map_or(0, |environment| environment.max_buffered_bytes as usize);
            if self.queued_input_bytes.saturating_add(data.len()) > limit {
                self.error = Some("Terminal input queue is full; wait for acknowledgement.".into());
                return false;
            }
            self.queued_input_bytes += data.len();
            self.queued_input.push_back(data.to_owned());
            return true;
        }
        self.write_encoded(data)
    }

    fn write_encoded(&mut self, data: &str) -> bool {
        if !self.writable {
            return false;
        }
        let result = {
            let Some(writer) = &mut self.input else {
                return false;
            };
            writer.write(data).map(|sequence| (sequence, writer.is_closed()))
        };
        match result {
            Ok((_, true)) => {
                self.error = Some("Terminal input stream closed before the write was sent.".into());
                self.detach();
                false
            }
            Ok((sequence, false)) => {
                self.pending = Some(sequence);
                self.status = format!("Awaiting ack #{sequence}");
                true
            }
            Err(error) => {
                self.error = Some(format!("Input was not queued: {error}"));
                self.detach();
                false
            }
        }
    }

    fn flush_queued_input(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(data) = self.queued_input.pop_front() {
            self.queued_input_bytes = self.queued_input_bytes.saturating_sub(data.len());
            self.write_encoded(&data);
        }
    }

    /// Sends the command text framed as a paste when the program asked for it.
    fn send_paste(&mut self) {
        let data = terminal_input::paste(&self.text, self.modes.bracketed_paste);
        if self.send_encoded(&data) {
            self.text.clear();
        }
    }

    /// Sends one or more pointer gestures at a one-based grid cell.
    fn send_mouse(&mut self, actions: &[terminal_input::MouseAction], cell: (usize, usize)) {
        let mut data = String::new();
        for action in actions {
            let Some(encoded) = terminal_input::mouse(self.modes, *action, cell.0, cell.1) else {
                continue;
            };
            data.push_str(&encoded);
        }
        if !data.is_empty() {
            self.send_encoded(&data);
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
            "ANSI SGR colors, cursor addressing, and line erases are rendered from the stream.",
        );
        if self.parsed_revision != self.output_revision || self.parsed_width != self.cols {
            let screen = terminal_ansi::parse_screen(&self.output, self.cols as usize);
            self.parsed = screen.lines;
            self.parsed_revision = self.output_revision;
            self.parsed_width = self.cols;
            if !screen.replies.is_empty() {
                self.reply_to_program(&screen.replies);
            }
        }
        let mouse_mode = self.modes.mouse_reporting();
        let mouse_active = mouse_mode != terminal_ansi::MouseReporting::Off;
        if mouse_active || self.modes.bracketed_paste {
            let mut enabled = Vec::new();
            if self.modes.bracketed_paste {
                enabled.push("bracketed paste");
            }
            if mouse_active {
                enabled.push("mouse reporting");
            }
            ui.small(format!("Program enabled {}", enabled.join(" and ")));
        }
        // Wheel ticks belong to a program that tracks the pointer, so they are
        // consumed before the scroll area sees them.
        if mouse_active {
            if let Some(rect) = self.grid_rect {
                if let Some(position) = ui
                    .input(|input| input.pointer.hover_pos())
                    .filter(|position| rect.contains(*position))
                {
                    let delta =
                        ui.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta.y));
                    if delta != 0.0 {
                        if let Some(cell) = terminal_cell(ui, rect, position) {
                            self.send_mouse(
                                &[terminal_input::MouseAction::Wheel(delta > 0.0)],
                                cell,
                            );
                        }
                    }
                }
            }
        }
        let job = terminal_ansi::layout(&self.parsed, ui.available_width());
        let mut grid_response = None;
        egui::ScrollArea::both()
            .id_salt("terminal-output")
            .max_height(180.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let font = egui::FontId::monospace(12.0);
                let (cell_width, cell_height) = ui
                    .painter()
                    .fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
                let size = egui::vec2(
                    ui.available_width().max(cell_width * self.cols as f32),
                    (cell_height * self.rows as f32).max(cell_height),
                );
                let sense = if mouse_active {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::hover()
                };
                grid_response = Some(
                    ui.add_sized(
                        size,
                        egui::Label::new(job)
                            .selectable(!mouse_active)
                            .wrap_mode(egui::TextWrapMode::Extend)
                            .sense(sense),
                    ),
                );
            });
        if let Some(response) = grid_response {
            self.grid_rect = Some(response.rect);
            response.output_event(egui::output::OutputEvent::ValueChanged(
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Label,
                    response.enabled(),
                    "Terminal screen",
                ),
            ));
            if mouse_active {
                let pointer = response
                    .interact_pointer_pos()
                    .or_else(|| response.clicked().then_some(response.rect.center()));
                if let Some(position) = pointer {
                    if let Some(cell) = terminal_cell(ui, response.rect, position) {
                        for (pointer_button, button) in [
                            (egui::PointerButton::Primary, 0),
                            (egui::PointerButton::Middle, 1),
                            (egui::PointerButton::Secondary, 2),
                        ] {
                            if response.drag_started_by(pointer_button) {
                                self.send_mouse(
                                    &[terminal_input::MouseAction::Press(button)],
                                    cell,
                                );
                            }
                            if response.dragged_by(pointer_button) {
                                self.send_mouse(&[terminal_input::MouseAction::Drag(button)], cell);
                            }
                            if response.drag_stopped_by(pointer_button) {
                                self.send_mouse(
                                    &[terminal_input::MouseAction::Release(button)],
                                    cell,
                                );
                            } else if response.clicked_by(pointer_button) {
                                self.send_mouse(
                                    &[
                                        terminal_input::MouseAction::Press(button),
                                        terminal_input::MouseAction::Release(button),
                                    ],
                                    cell,
                                );
                            }
                        }
                        if mouse_mode == terminal_ansi::MouseReporting::AnyMotion
                            && response.hovered()
                            && ui.input(|input| input.pointer.delta() != egui::Vec2::ZERO)
                        {
                            self.send_mouse(&[terminal_input::MouseAction::Move], cell);
                        }
                    }
                }
            }
        }
        let ready = self.input.is_some() && self.writable && self.pending.is_none();
        if let Some(environment) = &self.environment {
            let max_input = environment.max_input_bytes as usize;
            let (max_cols, max_rows) = (environment.max_cols, environment.max_rows);
            let command_field = ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .desired_rows(2)
                    .hint_text("Exact text to write; include a newline to execute"),
            );
            accessible_label(&command_field, "Terminal command");
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
                if self.modes.bracketed_paste
                    && ui
                        .add_enabled(
                            ready && !self.text.is_empty() && self.text.len() + 12 <= max_input,
                            egui::Button::new("Send as paste"),
                        )
                        .clicked()
                {
                    self.send_paste();
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
mod ui_harness;

#[cfg(test)]
mod terminal_panel_tests {
    use super::{append_bounded_output, TerminalPanel, TerminalUpdate, TERMINAL_VISIBLE_BYTES};
    use crate::terminal_ansi::InputModes;
    use crate::terminal_input::MouseAction;
    use dsh_client_core::terminal::{
        TerminalAckFrame, TerminalEnvironment, TerminalFrame, TerminalInput, TerminalInputLimits,
        TerminalRefusal, TerminalWriter,
    };
    use std::sync::mpsc;

    const LIMITS: TerminalInputLimits = TerminalInputLimits {
        max_input_bytes: 4096,
        max_buffered_bytes: 16384,
        max_cols: 200,
        max_rows: 60,
    };

    fn attached() -> (TerminalPanel, mpsc::Receiver<TerminalInput>) {
        let (input, inputs) = mpsc::channel();
        let mut panel = TerminalPanel::default();
        panel.input = Some(TerminalWriter::new(input, LIMITS));
        panel.writable = true;
        (panel, inputs)
    }

    #[test]
    fn paste_is_framed_only_when_the_program_enables_bracketed_paste() {
        let (mut panel, inputs) = attached();
        panel.text = "line one\nline two".to_owned();
        panel.modes = InputModes {
            bracketed_paste: true,
            ..InputModes::default()
        };
        panel.send_paste();
        assert_eq!(
            inputs.try_recv(),
            Ok(TerminalInput::Write {
                sequence: 1,
                data: "\u{1b}[200~line one\nline two\u{1b}[201~".to_owned(),
            })
        );
        assert!(panel.text.is_empty());

        panel.pending = None;
        panel.text = "raw".to_owned();
        panel.modes.bracketed_paste = false;
        panel.send_paste();
        assert_eq!(
            inputs.try_recv(),
            Ok(TerminalInput::Write {
                sequence: 2,
                data: "raw".to_owned(),
            })
        );
    }

    #[test]
    fn mouse_gestures_are_encoded_for_the_enabled_protocol() {
        let (mut panel, inputs) = attached();
        panel.modes = InputModes {
            bracketed_paste: false,
            mouse_press: true,
            sgr_mouse: true,
            ..InputModes::default()
        };
        panel.send_mouse(&[MouseAction::Press(0), MouseAction::Release(0)], (3, 2));
        assert_eq!(
            inputs.try_recv(),
            Ok(TerminalInput::Write {
                sequence: 1,
                data: "\u{1b}[<0;3;2M\u{1b}[<0;3;2m".to_owned(),
            })
        );

        // A press-only program gets the wheel (as in xterm) but never drags.
        panel.pending = None;
        panel.send_mouse(&[MouseAction::Drag(0)], (3, 2));
        assert_eq!(inputs.try_recv(), Err(mpsc::TryRecvError::Empty));
        panel.send_mouse(&[MouseAction::Wheel(true)], (3, 2));
        assert_eq!(
            inputs.try_recv(),
            Ok(TerminalInput::Write {
                sequence: 2,
                data: "\u{1b}[<64;3;2M".to_owned(),
            })
        );

        panel.pending = None;
        panel.modes.mouse_press = false;
        panel.modes.mouse_drag = true;
        panel.send_mouse(&[MouseAction::Drag(0)], (4, 2));
        assert_eq!(
            inputs.try_recv(),
            Ok(TerminalInput::Write {
                sequence: 3,
                data: "\u{1b}[<32;4;2M".to_owned(),
            })
        );
    }

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
    fn input_modes_survive_output_retention_trimming() {
        let mut panel = TerminalPanel::default();
        panel.push_output("\u{1b}[?2004h\u{1b}[?1002h\u{1b}[?1006h");
        panel.push_output(&"x".repeat(TERMINAL_VISIBLE_BYTES));
        assert!(panel.modes.bracketed_paste);
        assert_eq!(
            panel.modes.mouse_reporting(),
            crate::terminal_ansi::MouseReporting::ButtonDrag
        );
        assert!(panel.modes.sgr_mouse);
        panel.clear_output();
        assert_eq!(panel.modes, InputModes::default());
    }

    #[test]
    fn refusal_discards_queued_terminal_input_with_a_visible_error() {
        let (mut panel, _inputs) = attached();
        panel.environment = Some(TerminalEnvironment {
            cwd: "/tmp".into(),
            max_input_bytes: 4096,
            max_buffered_bytes: 16384,
            max_cols: 80,
            max_rows: 24,
            scrollback: 1000,
        });
        panel.pending = Some(1);
        panel.queued_input.push_back("queued mouse".into());
        panel.queued_input_bytes = "queued mouse".len();
        let (updates, receiver) = mpsc::channel();
        panel.updates = Some(receiver);
        updates
            .send(TerminalUpdate::Frame(Box::new(TerminalFrame::Ack(
                TerminalAckFrame {
                    sequence: 1,
                    refusal: Some(TerminalRefusal::ReadOnly),
                },
            ))))
            .expect("ack update");

        panel.poll(&eframe::egui::Context::default());
        assert_eq!(panel.pending, None);
        assert!(panel.queued_input.is_empty());
        assert_eq!(panel.queued_input_bytes, 0);
        assert!(panel
            .error
            .as_deref()
            .is_some_and(|error| error.contains("ReadOnly")));
    }

    #[test]
    fn terminal_mouse_queue_limit_reports_input_instead_of_growing_unbounded() {
        let (mut panel, inputs) = attached();
        panel.environment = Some(TerminalEnvironment {
            cwd: "/tmp".into(),
            max_input_bytes: 4096,
            max_buffered_bytes: 4,
            max_cols: 80,
            max_rows: 24,
            scrollback: 1000,
        });
        panel.pending = Some(1);
        panel.modes.mouse_press = true;
        panel.modes.sgr_mouse = true;
        panel.send_mouse(&[MouseAction::Wheel(true)], (3, 2));
        assert!(panel.queued_input.is_empty());
        assert_eq!(panel.queued_input_bytes, 0);
        assert!(panel.error.as_deref().is_some_and(|error| error.contains("queue is full")));
        assert_eq!(inputs.try_recv(), Err(mpsc::TryRecvError::Empty));
    }

    #[test]
    fn closed_terminal_uplink_detaches_after_a_write_fails() {
        let (mut panel, inputs) = attached();
        drop(inputs);
        assert!(!panel.send_encoded("paste"));
        assert!(!panel.writable);
        assert_eq!(panel.pending, None);
        assert!(panel.input.is_none());
        assert!(panel
            .error
            .as_deref()
            .is_some_and(|error| error.contains("stream closed")));
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

/// Follow backlog plus history page a Session load resolves to.
type PageLoad = (i64, SessionPage, Vec<dsh_client_core::follow::EventFrame>);

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
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    Sessions,
    Settings,
    Document,
    Plugins,
    Skills,
    Marketplace,
    Goals,
}

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
    page_has_more: bool,
    page_loading: bool,
    page_before_seq: Option<i64>,
    page_error: Option<String>,
    page_receiver: Option<mpsc::Receiver<Result<PageLoad, Error>>>,
    page_through_seq: Option<i64>,
    chat_text: String,
    focus_composer: bool,
    cancel_receiver: Option<mpsc::Receiver<Result<AcceptedResponse, Error>>>,
    dialog: Option<(DialogPurpose, mpsc::Receiver<Option<PathBuf>>)>,
    attachment: Option<FileUploadValue>,
    attachment_name: String,
    attachment_receiver: Option<mpsc::Receiver<Result<FileUploadValue, String>>>,
    attachment_error: Option<String>,
    chat_loading: bool,
    chat_receiver: Option<mpsc::Receiver<Result<FollowFrame, Error>>>,
    transcript: Transcript,
    assistant_streaming: bool,
    assistant_draft: String,
    approval_events_receiver: Option<mpsc::Receiver<Result<RemoteEvent, RemoteEventError>>>,
    approval_commands: Option<mpsc::Sender<ApprovalCommand>>,
    pending_approvals: Vec<PendingApproval>,
    pending_questions: Vec<QuestionCard>,
    deliverables: Vec<DeliverableFile>,
    market_indexes: Vec<serde_json::Value>,
    market_results: Vec<serde_json::Value>,
    market_url: Option<String>,
    market_query: String,
    market_receiver: Option<mpsc::Receiver<Result<serde_json::Value, Error>>>,
    market_status: Option<String>,
    market_loading: bool,
    feedback_items: Vec<MessageFeedbackItem>,
    feedback_receiver: Option<mpsc::Receiver<Result<Vec<MessageFeedbackItem>, Error>>>,
    feedback_put_receiver: Option<mpsc::Receiver<Result<MessageFeedbackItem, Error>>>,
    feedback_status: Option<String>,
    settings_panel: SettingsPanel,
    ui_preferences_path: PathBuf,
    font_catalog: fonts::SharedCatalog,
    font_catalog_snapshot: Option<fonts::FontCatalog>,
    applied_fonts: Option<ui_preferences::FontSettings>,
    applied_cjk: Option<bool>,
    ui_preferences: UiPreferences,
    applied_ui_preferences: Option<UiPreferences>,
    ui_preferences_panel: UiPreferencesPanel,
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
    show_skills: bool,
    show_market: bool,
    show_goals: bool,
    goal: Option<GoalView>,
    goal_loading: bool,
    goal_mutation_loading: bool,
    goal_receiver: Option<mpsc::Receiver<Result<Option<GoalView>, Error>>>,
    goal_mutation_receiver: Option<mpsc::Receiver<Result<Option<GoalView>, Error>>>,
    goal_objective: String,
    goal_max_rounds: String,
    clear_goal_confirmation: Option<GoalRef>,
    goal_error: Option<String>,
    goal_status: Option<String>,
    plan_command_receiver: Option<mpsc::Receiver<Result<Option<CommandExecution>, Error>>>,
    plan_command_loading: bool,
    plan_command_status: Option<String>,
    active_view: ActiveView,
    skills: Option<SkillListResponse>,
    skills_receiver: Option<mpsc::Receiver<Result<SkillListResponse, Error>>>,
    skills_loading: bool,
    skills_error: Option<String>,
    model_catalog: Option<ModelCatalog>,
    model_selection: Option<ModelSelection>,
    model_error: Option<String>,
    model_receiver: Option<mpsc::Receiver<Result<ModelCatalog, Error>>>,
    model_requested_at: Option<std::time::Instant>,
    model_select_receiver: Option<mpsc::Receiver<Result<SelectModelResponse, Error>>>,
    model_init_receiver: Option<mpsc::Receiver<Result<(), Error>>>,
    plugins_snapshot: Option<PluginInventorySnapshot>,
    plugins_loading: bool,
    plugins_error: Option<String>,
    plugins_receiver: Option<mpsc::Receiver<PluginInventoryResult>>,
    plugin_bundles: Vec<BundleInfo>,
    managed_plugins: Vec<ManagedPlugin>,
    plugin_write_receiver: Option<mpsc::Receiver<Result<ChangeResult, Error>>>,
    plugin_write_status: Option<String>,
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
    update_prompt: Option<host::UpdatePrompt>,
    update_error: Option<String>,
    quitting: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    hide_requested: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    viewport: Arc<Mutex<Option<egui::Context>>>,
    tray_available: bool,
    auto_connect: bool,
}

/// Process-level services the shell is handed before it can draw.
struct ShellServices {
    quitting: Arc<AtomicBool>,
    show_requested: Arc<AtomicBool>,
    hide_requested: Arc<AtomicBool>,
    hidden: Arc<AtomicBool>,
    viewport: Arc<Mutex<Option<egui::Context>>>,
    tray_available: bool,
    ui_preferences_path: PathBuf,
    /// Whether to start the Desktop Host; a windowless harness brings its own.
    spawn_host: bool,
}

impl DesktopApp {
    fn new(services: ShellServices) -> Self {
        let ShellServices {
            quitting,
            show_requested,
            hide_requested,
            hidden,
            viewport,
            tray_available,
            ui_preferences_path,
            spawn_host,
        } = services;
        let (ui_preferences, ui_preferences_load_error) =
            match UiPreferences::load(&ui_preferences_path) {
                Ok(preferences) => (preferences, None),
                Err(error) => {
                    eprintln!("dsh: UI preferences unavailable; defaults are active: {error}");
                    (UiPreferences::default(), Some(error.to_string()))
                }
            };
        let ui_preferences_panel =
            UiPreferencesPanel::new(ui_preferences.clone(), ui_preferences_load_error);
        // A windowless harness supplies its own transport and must not start a Host.
        let host_receiver = if spawn_host {
            let (host_sender, host_receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = host_sender.send(Host::start());
            });
            Some(host_receiver)
        } else {
            None
        };
        Self {
            host: None,
            host_receiver,
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
            page_has_more: false,
            page_loading: false,
            page_before_seq: None,
            page_error: None,
            page_receiver: None,
            page_through_seq: None,
            chat_text: String::new(),
            focus_composer: false,
            cancel_receiver: None,
            dialog: None,
            attachment: None,
            attachment_name: String::new(),
            attachment_receiver: None,
            attachment_error: None,
            chat_loading: false,
            chat_receiver: None,
            transcript: Transcript::default(),
            assistant_streaming: false,
            assistant_draft: String::new(),
            approval_events_receiver: None,
            approval_commands: None,
            pending_approvals: Vec::new(),
            pending_questions: Vec::new(),
            deliverables: Vec::new(),
            market_indexes: Vec::new(),
            market_results: Vec::new(),
            market_url: None,
            market_query: String::new(),
            market_receiver: None,
            market_status: None,
            market_loading: false,
            feedback_items: Vec::new(),
            feedback_receiver: None,
            feedback_put_receiver: None,
            feedback_status: None,
            settings_panel: SettingsPanel::default(),
            ui_preferences_path,
            ui_preferences,
            applied_ui_preferences: None,
            font_catalog: fonts::shared_catalog(),
            font_catalog_snapshot: None,
            applied_fonts: None,
            applied_cjk: None,
            ui_preferences_panel,
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
            show_skills: false,
            show_market: false,
            show_goals: false,
            goal: None,
            goal_loading: false,
            goal_mutation_loading: false,
            goal_receiver: None,
            goal_mutation_receiver: None,
            goal_objective: String::new(),
            goal_max_rounds: String::new(),
            clear_goal_confirmation: None,
            goal_error: None,
            goal_status: None,
            plan_command_receiver: None,
            plan_command_loading: false,
            plan_command_status: None,
            active_view: ActiveView::Sessions,
            skills: None,
            skills_receiver: None,
            skills_loading: false,
            skills_error: None,
            model_catalog: None,
            model_selection: None,
            model_error: None,
            model_receiver: None,
            model_requested_at: None,
            model_select_receiver: None,
            model_init_receiver: None,
            plugins_snapshot: None,
            plugins_loading: false,
            plugins_error: None,
            plugins_receiver: None,
            plugin_bundles: Vec::new(),
            managed_plugins: Vec::new(),
            plugin_write_receiver: None,
            plugin_write_status: None,
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
            update_prompt: None,
            update_error: None,
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
                        ApprovalCommand::AnswerQuestion(pending, answer) => {
                            pending.answer(&events, answer)
                        }
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
        self.pending_questions.clear();
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
                SessionOperation::List => SessionViewer::new(&transport)
                    .list(None)
                    .map(SessionUpdate::List),
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

    fn handle_ui_preferences_action(&mut self, action: UiPreferencesAction, ctx: &egui::Context) {
        match action {
            UiPreferencesAction::Preview(preferences) => {
                self.ui_preferences = preferences;
                self.apply_ui_preferences(ctx);
            }
            UiPreferencesAction::Save(preferences) => {
                match preferences.save(&self.ui_preferences_path) {
                    Ok(()) => {
                        self.ui_preferences = preferences.clone();
                        self.ui_preferences_panel.saved(preferences);
                        self.apply_ui_preferences(ctx);
                    }
                    Err(error) => {
                        eprintln!("dsh: UI preferences save failed: {error}");
                        let disposition = if self.ui_preferences == preferences {
                            "Preview remains active for this session"
                        } else {
                            "The previous appearance remains active"
                        };
                        self.ui_preferences_panel.failed(format!(
                            "Could not save UI preferences: {error}. {disposition}; draft retained. Fix the file location or permissions and retry."
                        ));
                    }
                }
            }
            UiPreferencesAction::Reload => match UiPreferences::load(&self.ui_preferences_path) {
                Ok(preferences) => {
                    self.ui_preferences = preferences.clone();
                    self.ui_preferences_panel.reloaded(preferences);
                    self.apply_ui_preferences(ctx);
                }
                Err(error) => {
                    eprintln!("dsh: UI preferences reload failed: {error}");
                    self.ui_preferences_panel.failed(format!(
                            "Could not reload UI preferences: {error}. Current appearance remains active."
                        ));
                }
            },
            UiPreferencesAction::Reset => {
                let preferences = UiPreferences::default();
                match preferences.save(&self.ui_preferences_path) {
                    Ok(()) => {
                        self.ui_preferences = preferences.clone();
                        self.ui_preferences_panel.saved(preferences);
                        self.apply_ui_preferences(ctx);
                    }
                    Err(error) => {
                        eprintln!("dsh: UI preferences reset failed: {error}");
                        self.ui_preferences_panel.failed(format!(
                            "Could not reset UI preferences: {error}. Current appearance remains active."
                        ));
                    }
                }
            }
        }
    }

    fn apply_ui_preferences(&mut self, ctx: &egui::Context) {
        ui_preferences::apply(ctx, &self.ui_preferences);
        self.applied_ui_preferences = Some(self.ui_preferences.clone());
        self.apply_fonts(ctx);
    }

    /// Rebuilds the egui font stack once the installed-family scan is ready.
    ///
    /// The scan runs off the first frame, so startup applies the bundled faces
    /// and this installs the chosen families as soon as the catalog arrives.
    /// Whether no catalog request is running and no provider offers a model.
    fn model_catalog_empty(&self) -> bool {
        self.model_receiver.is_none()
            && self
                .model_catalog
                .as_ref()
                .is_none_or(|catalog| catalog.groups.iter().all(|group| group.models.is_empty()))
    }

    /// Labels the model picker with the chosen model or the reason there is none.
    fn model_picker_label(&self) -> String {
        model_picker_label(
            self.model_selection.as_ref(),
            self.model_error.as_deref(),
            self.model_catalog.as_ref(),
            self.model_receiver.is_some(),
        )
    }

    /// Whether the interface currently needs a CJK fallback face.
    fn cjk_required(&self) -> bool {
        self.transcript.has_cjk()
            || fonts::contains_cjk(&self.chat_text)
            || fonts::contains_cjk(&self.session_title)
            || self.sessions.iter().any(|session| {
                session
                    .projections
                    .as_ref()
                    .and_then(|projections| projections.get("title"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(fonts::contains_cjk)
            })
    }

    fn apply_fonts(&mut self, ctx: &egui::Context) {
        let Some(catalog) = self.font_catalog_snapshot.clone() else {
            return;
        };
        let include_cjk = self.cjk_required();
        if self.applied_fonts.as_ref() == Some(&self.ui_preferences.fonts)
            && self.applied_cjk == Some(include_cjk)
        {
            return;
        }
        for warning in fonts::install(ctx, &self.ui_preferences.fonts, &catalog, include_cjk) {
            eprintln!("dsh: {warning}");
        }
        self.applied_fonts = Some(self.ui_preferences.fonts.clone());
        self.applied_cjk = Some(include_cjk);
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
        self.model_requested_at = Some(std::time::Instant::now());
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

    fn initialize_default_model(&mut self) {
        if self.model_init_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.model_error = Some("Connect before selecting the default model".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ModelClient::new(&transport).initialize_default_model());
        });
        self.model_init_receiver = Some(receiver);
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
            let client = PluginsClient::new(&transport);
            let result = client
                .list()
                .map(|snapshot| (snapshot, client.list_bundles(), client.list_plugins()))
                .map(|(snapshot, bundles, plugins)| {
                    (
                        snapshot,
                        bundles.unwrap_or_default(),
                        plugins.unwrap_or_default(),
                    )
                });
            let _ = sender.send(result);
        });
        self.plugins_receiver = Some(receiver);
        self.plugins_loading = true;
        self.plugins_error = None;
    }

    fn record_deliverables(&mut self, event: &dsh_client_core::follow::SessionWireEvent) {
        record_deliverables(&mut self.deliverables, event);
    }

    fn write_plugin(&mut self, action: PluginAction) {
        if self.plugin_write_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.plugins_error = Some("Connect to change plugins".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = PluginsClient::new(&transport);
            let result = match action {
                PluginAction::SetPlugin { id, enabled } => client.set_plugin_enabled(&id, enabled),
                PluginAction::SetBundle { name, enabled } => {
                    client.set_bundle_enabled(&name, enabled)
                }
            };
            let _ = sender.send(result);
        });
        self.plugin_write_receiver = Some(receiver);
        self.plugin_write_status = Some("Applying plugin change…".to_owned());
    }

    fn refresh_feedback(&mut self) {
        if self.feedback_receiver.is_some() {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.feedback_status = Some("Connect to load feedback".to_owned());
            return;
        };
        let Some(session) = self.selected_session.clone() else {
            self.feedback_items.clear();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MessageFeedbackClient::new(&transport).list(session));
        });
        self.feedback_receiver = Some(receiver);
        self.feedback_status = None;
    }

    fn refresh_market_indexes(&mut self) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to load marketplace".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MarketplaceClient::new(&transport).list_indexes());
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = None;
    }

    fn search_market(&mut self) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to search".to_owned());
            return;
        };
        let Some(url) = self.market_url.clone() else {
            self.market_status = Some("No market index configured".to_owned());
            return;
        };
        let query = self.market_query.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(MarketplaceClient::new(&transport).search(&url, &query));
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = None;
    }

    fn install_market_entry(&mut self, spec: String) {
        if self.market_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.market_status = Some("Connect to install".to_owned());
            return;
        };
        let status_spec = spec.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = MarketplaceClient::new(&transport);
            let result = (|| -> Result<String, Error> {
                let inspect = client.inspect(&spec)?;
                let status = inspect
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if status != "accepted" {
                    return Err(Error::Remote {
                        code: "inspect-rejected".into(),
                        message: format!("inspect status: {status}"),
                        details: serde_json::Map::new(),
                    });
                }
                let install = client.install_bundle(&spec)?;
                let application = install
                    .get("application")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| Error::InvalidEnvelope("missing application result"))?;
                Ok(format!("{spec} -> {application}"))
            })();
            let _ = sender.send(result.map(serde_json::Value::String));
        });
        self.market_receiver = Some(receiver);
        self.market_loading = true;
        self.market_status = Some(format!("Installing {status_spec}…"));
    }

    fn refresh_skills(&mut self) {
        if self.skills_loading {
            return;
        }
        let Some(transport) = self.transport.clone() else {
            self.skills_error = Some("Connect to load skills".to_owned());
            return;
        };
        let Some(session) = self.selected_session.clone() else {
            self.skills_error = Some("Select a session to list its skills".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(SkillsClient::new(&transport).list(session));
        });
        self.skills_receiver = Some(receiver);
        self.skills_loading = true;
        self.skills_error = None;
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

    fn mutate_settings(&mut self, request: dsh_client_core::settings::SettingsMutateRequest) {
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
            let _ = sender.send(SettingsClient::new(&transport).mutate(request));
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

    fn read_office_document(&mut self) {
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
            let scope = WorkspaceFileScope { session_id };
            let result = OfficeToPdfClient::new(&transport)
                .render(scope, path.clone(), "foreground")
                .map_err(|error| error.to_string())
                .and_then(|preview| {
                    // Keep parallel previews from overwriting the same temporary PDF.
                    let fingerprint = path.bytes().fold(0_u64, |hash, byte| {
                        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
                    });
                    let directory = std::env::temp_dir().join(format!(
                        "dsh-office-preview-{}-{fingerprint:x}",
                        std::process::id()
                    ));
                    let _scratch = PreviewScratchDirectory(directory.clone());
                    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                    let pdf_path = directory.join("document.pdf");
                    std::fs::write(&pdf_path, &preview.pdf).map_err(|error| error.to_string())?;
                    let prefix = directory.join("page");
                    let output = std::process::Command::new("pdftoppm")
                        .args([
                            "-png",
                            "-r",
                            "110",
                            "-f",
                            "1",
                            "-l",
                            "1",
                            pdf_path.to_str().unwrap_or_default(),
                            prefix.to_str().unwrap_or_default(),
                        ])
                        .output()
                        .map_err(|error| error.to_string())?;
                    if !output.status.success() {
                        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
                    }
                    let png_path = format!("{}-1.png", prefix.display());
                    let bytes = std::fs::read(&png_path).map_err(|error| error.to_string())?;
                    let image =
                        image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
                    let rgba = image.to_rgba8();
                    let (width, height) = rgba.dimensions();
                    let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                    let mut document_name = name;
                    if !preview.missing_fonts.is_empty() {
                        document_name = format!(
                            "{document_name} (missing fonts: {})",
                            preview.missing_fonts.join(", ")
                        );
                    }
                    Ok(PreviewDocument::Image(ImageDocument {
                        name: document_name,
                        host_path: path,
                        size: [width as usize, height as usize],
                        rgba: rgba.into_raw(),
                    }))
                });
            let _ = sender.send(result);
        });
        self.preview_receiver = Some(receiver);
        self.preview_loading = true;
        self.preview_error = None;
    }

    fn is_office_path(path: &str) -> bool {
        matches!(
            path.rsplit('.').next().map(str::to_lowercase).as_deref(),
            Some("doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx")
        )
    }

    fn load_session(&mut self, session_id: String) {
        self.session_title.clear();
        self.show_settings = false;
        self.show_plugins = false;
        self.selected_session = Some(session_id.clone());
        self.goal = None;
        self.goal_receiver = None;
        self.goal_mutation_receiver = None;
        self.goal_loading = false;
        self.goal_mutation_loading = false;
        self.goal_error = None;
        self.goal_status = None;
        self.clear_goal_confirmation = None;
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
        self.page_has_more = false;
        self.page_receiver = None;
        self.page_through_seq = None;
        self.page_error = None;
        self.page_before_seq = None;
        self.chat_receiver = None;
        self.chat_loading = false;
        self.transcript.reset();
        self.assistant_streaming = false;
        self.assistant_draft.clear();
        self.deliverables.clear();
        self.feedback_items.clear();
        self.refresh_feedback();
        self.preview_document = None;
        self.preview_error = None;
        self.show_document = false;
        self.page_loading = true;
        let Some(transport) = self.transport.clone() else {
            self.page_loading = false;
            self.page_error = Some("Connect to load this session".to_owned());
            return;
        };
        let Some(streams) = self.streams.clone() else {
            self.page_loading = false;
            self.page_error = Some("Remote streams are not connected".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let mut stream = follow::follow(
                    &streams,
                    FollowRequest {
                        address: SessionAddress {
                            kind: SessionAddressKind::Session,
                            session_id: session_id.clone(),
                        },
                        max_messages: Some(1),
                        turn_window: None,
                        assistant_stream: false,
                    },
                )?;
                let frame = stream.next().ok_or(Error::InvalidEnvelope(
                    "session follow ended before snapshot",
                ))??;
                let FollowFrame::Snapshot(snapshot) = frame else {
                    return Err(Error::InvalidEnvelope(
                        "session follow did not start with snapshot",
                    ));
                };
                let through_seq = snapshot.cursor;
                // The snapshot is the newest backlog; the page holds older
                // history. Both project into the same transcript.
                let live = snapshot.records;
                drop(stream);
                let page = SessionViewer::new(&transport).page(&session_id, through_seq)?;
                Ok((through_seq, page, live))
            })();
            let _ = sender.send(result);
        });
        self.page_receiver = Some(receiver);
        if self.show_goals {
            self.refresh_goal();
        }
    }

    fn load_older_session_page(&mut self) {
        if self.page_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.page_error = Some("Select a Session before loading older events".to_owned());
            return;
        };
        let Some(before_seq) = self.transcript.oldest_seq() else {
            self.page_error = Some("No earlier session events are available".to_owned());
            return;
        };
        let Some(through_seq) = self.page_through_seq else {
            self.page_error = Some("Session history cursor is unavailable".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.page_error = Some("Connect before loading older events".to_owned());
            return;
        };
        let request = SessionPageRequest {
            address: SessionAddress {
                kind: SessionAddressKind::Session,
                session_id,
            },
            through_seq,
            before_seq: Some(before_seq),
            max_messages: Some(200),
            turn_window: Some(SessionTurnWindow {
                min_messages: 100,
                min_turns: 2,
            }),
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = SessionViewer::new(&transport)
                .page_request(request)
                .map(|page| (through_seq, page, Vec::new()));
            let _ = sender.send(result);
        });
        self.page_receiver = Some(receiver);
        self.page_loading = true;
        self.page_before_seq = Some(before_seq);
        self.page_error = None;
    }

    fn refresh_goal(&mut self) {
        if self.goal_loading || self.goal_mutation_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.goal_error = Some("Select a Session to view its goal".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.goal_error = Some("Connect before loading the goal".to_owned());
            return;
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(GoalsClient::new(&transport).get(&session_id));
        });
        self.goal_receiver = Some(receiver);
        self.goal_loading = true;
        self.goal_error = None;
    }

    fn mutate_goal(&mut self, mutation: GoalMutation) {
        if self.goal_mutation_loading || self.goal_loading {
            return;
        }
        let Some(session_id) = self.selected_session.clone() else {
            self.goal_error = Some("Select a Session before changing its goal".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.goal_error = Some("Connect before changing the goal".to_owned());
            return;
        };
        let status = match &mutation {
            GoalMutation::Create(_) => "Creating goal…",
            GoalMutation::Edit(_, _) => "Saving goal…",
            GoalMutation::Pause(_) => "Pausing goal…",
            GoalMutation::Resume(_) => "Resuming goal…",
            GoalMutation::Complete(_) => "Completing goal…",
            GoalMutation::Clear(_) => "Clearing goal…",
        }
        .to_owned();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let client = GoalsClient::new(&transport);
            let result = match mutation {
                GoalMutation::Create(request) => {
                    client.create(&session_id, request).and_then(|created| {
                        let goal = client
                            .get(&session_id)?
                            .ok_or(Error::InvalidEnvelope("created goal is not readable"))?;
                        if goal.id != created.r#ref.id || goal.revision != created.r#ref.revision {
                            return Err(Error::InvalidEnvelope("created goal identity changed"));
                        }
                        Ok(Some(goal))
                    })
                }
                GoalMutation::Edit(reference, request) => {
                    client.edit(&session_id, &reference, request).map(Some)
                }
                GoalMutation::Pause(reference) => client.pause(&session_id, &reference).map(Some),
                GoalMutation::Resume(reference) => client.resume(&session_id, &reference).map(Some),
                GoalMutation::Complete(reference) => {
                    client.complete(&session_id, &reference).map(Some)
                }
                GoalMutation::Clear(reference) => {
                    client.clear(&session_id, &reference).map(|_| None)
                }
            };
            let _ = sender.send(result);
        });
        self.goal_mutation_receiver = Some(receiver);
        self.goal_mutation_loading = true;
        self.goal_status = Some(status);
        self.goal_error = None;
    }

    fn run_plan_command(&mut self, active: bool) {
        if self.plan_command_loading || self.chat_loading || self.assistant_streaming {
            return;
        }
        let Some(agent_id) = self.selected_session.clone() else {
            self.plan_command_status =
                Some("Select a Session before changing plan mode".to_owned());
            return;
        };
        let Some(transport) = self.transport.clone() else {
            self.plan_command_status = Some("Connect before changing plan mode".to_owned());
            return;
        };
        let line = if active { "/plan" } else { "/plan off" };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = CommandsClient::new(&transport).execute(agent_id.as_str(), line, vec![]);
            let _ = sender.send(result);
        });
        self.plan_command_receiver = Some(receiver);
        self.plan_command_loading = true;
        self.plan_command_status = Some(if active {
            "Entering plan mode…".to_owned()
        } else {
            "Leaving plan mode…".to_owned()
        });
    }

    fn render_goals_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Goal");
            if ui
                .add_enabled(
                    !self.goal_loading && !self.goal_mutation_loading,
                    egui::Button::new("Refresh"),
                )
                .clicked()
            {
                self.refresh_goal();
            }
        });
        let Some(session_id) = self.selected_session.as_deref() else {
            ui.label("Select a Session to manage its goal.");
            return;
        };
        ui.label(format!("Session: {session_id}"));
        if let Some(status) = &self.goal_status {
            ui.small(status);
        }
        if let Some(error) = &self.goal_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if self.goal_loading && self.goal.is_none() {
            ui.spinner();
            return;
        }

        let goal = self.goal.clone();
        if let Some(goal) = goal.filter(|goal| goal.phase != GoalPhase::Complete) {
            ui.label(format!(
                "{} · {:?} · {:?}",
                goal.id, goal.phase, goal.activation
            ));
            ui.label(format!(
                "{} of {} rounds used",
                goal.rounds_started, goal.max_goal_rounds
            ));
            ui.add(
                egui::TextEdit::multiline(&mut self.goal_objective)
                    .hint_text("Goal objective")
                    .desired_rows(3),
            );
            ui.horizontal(|ui| {
                ui.label("Maximum rounds");
                ui.add(egui::TextEdit::singleline(&mut self.goal_max_rounds).desired_width(80.0));
            });
            let reference = GoalRef {
                id: goal.id.clone(),
                revision: goal.revision,
            };
            let objective_changed = self.goal_objective.trim() != goal.objective;
            let rounds_text_changed = self.goal_max_rounds != goal.max_goal_rounds.to_string();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.goal_mutation_loading
                            && (objective_changed || rounds_text_changed)
                            && !self.goal_objective.trim().is_empty(),
                        egui::Button::new("Save changes"),
                    )
                    .clicked()
                {
                    let max_goal_rounds = if rounds_text_changed {
                        match self.goal_max_rounds.parse::<u64>() {
                            Ok(rounds) if rounds > 0 && rounds <= MAX_SAFE_SEQUENCE as u64 => {
                                Some(rounds)
                            }
                            _ => {
                                self.goal_error = Some(
                                    "Maximum rounds must be a positive whole number".to_owned(),
                                );
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    self.mutate_goal(GoalMutation::Edit(
                        reference.clone(),
                        EditGoalRequest {
                            objective: objective_changed
                                .then(|| self.goal_objective.trim().to_owned()),
                            max_goal_rounds,
                        },
                    ));
                }
                match (goal.phase, goal.activation) {
                    (GoalPhase::Active, GoalActivation::Armed) => {
                        if ui.button("Pause").clicked() {
                            self.mutate_goal(GoalMutation::Pause(reference.clone()));
                        }
                    }
                    (GoalPhase::Paused | GoalPhase::Blocked, _)
                    | (GoalPhase::Active, GoalActivation::Disarmed) => {
                        if ui.button("Resume").clicked() {
                            self.mutate_goal(GoalMutation::Resume(reference.clone()));
                        }
                    }
                    (GoalPhase::Complete, _) => {}
                }
                if ui
                    .add_enabled(!self.goal_mutation_loading, egui::Button::new("Complete"))
                    .clicked()
                {
                    self.mutate_goal(GoalMutation::Complete(reference.clone()));
                }
                if ui
                    .add_enabled(!self.goal_mutation_loading, egui::Button::new("Clear"))
                    .clicked()
                {
                    self.clear_goal_confirmation = Some(reference);
                }
            });
        } else {
            if self
                .goal
                .as_ref()
                .is_some_and(|goal| goal.phase == GoalPhase::Complete)
            {
                ui.label("The previous goal is complete. Create another goal for this Session.");
            } else {
                ui.label("No goal is active for this Session.");
            }
            ui.add(
                egui::TextEdit::multiline(&mut self.goal_objective)
                    .hint_text("What should the agent accomplish?")
                    .desired_rows(3),
            );
            ui.horizontal(|ui| {
                ui.label("Maximum rounds (optional)");
                ui.add(egui::TextEdit::singleline(&mut self.goal_max_rounds).desired_width(80.0));
            });
            if ui
                .add_enabled(
                    !self.goal_mutation_loading && !self.goal_objective.trim().is_empty(),
                    egui::Button::new("Create goal"),
                )
                .clicked()
            {
                let max_goal_rounds = if self.goal_max_rounds.trim().is_empty() {
                    None
                } else {
                    match self.goal_max_rounds.parse::<u64>() {
                        Ok(rounds) if rounds > 0 && rounds <= MAX_SAFE_SEQUENCE as u64 => {
                            Some(rounds)
                        }
                        _ => {
                            self.goal_error =
                                Some("Maximum rounds must be a positive whole number".to_owned());
                            return;
                        }
                    }
                };
                self.mutate_goal(GoalMutation::Create(CreateGoalRequest {
                    objective: self.goal_objective.trim().to_owned(),
                    max_goal_rounds,
                }));
            }
        }
        self.render_clear_goal_confirmation(ui.ctx());
        ui.separator();
        ui.heading("Recent tasks and plan changes");
        let work_rows: Vec<_> = self
            .transcript
            .rows()
            .iter()
            .rev()
            .filter(|row| {
                matches!(
                    row.kind,
                    RowKind::Todo | RowKind::Goal | RowKind::Notice
                )
            })
            .take(12)
            .collect();
        if work_rows.is_empty() {
            ui.label("Task and plan updates will appear here when the assistant records them.");
        } else {
            for row in work_rows {
                ui.strong(format!("{} · sequence {}", row.title, row.seq));
                ui.label(&row.text);
                ui.separator();
            }
        }
        ui.heading("Plan mode");
        ui.horizontal(|ui| {
            let can_run =
                !self.plan_command_loading && !self.chat_loading && !self.assistant_streaming;
            if ui
                .add_enabled(can_run, egui::Button::new("Enter plan mode"))
                .clicked()
            {
                self.run_plan_command(true);
            }
            if ui
                .add_enabled(can_run, egui::Button::new("Leave plan mode"))
                .clicked()
            {
                self.run_plan_command(false);
            }
        });
        if let Some(status) = &self.plan_command_status {
            ui.small(status);
        }
    }

    fn render_clear_goal_confirmation(&mut self, ctx: &egui::Context) {
        let Some(reference) = self.clear_goal_confirmation.clone() else {
            return;
        };
        let mut action = None;
        egui::Modal::new(egui::Id::new("clear-goal-confirmation")).show(ctx, |ui| {
            ui.heading("Clear current goal?");
            ui.label("The goal will be removed from this Session. Its history remains in the session log.");
            ui.horizontal(|ui| {
                if ui.button("Clear goal").clicked() {
                    action = Some(true);
                }
                if ui.button("Keep goal").clicked() {
                    action = Some(false);
                }
            });
        });
        match action {
            Some(true) => {
                self.clear_goal_confirmation = None;
                self.mutate_goal(GoalMutation::Clear(reference));
            }
            Some(false) => self.clear_goal_confirmation = None,
            None => {}
        }
    }

    fn pick_attachment(&mut self) {
        if self.transport.is_none() {
            self.attachment_error = Some("Connect before attaching a file".to_owned());
            return;
        }
        if self.selected_session.is_none() {
            self.attachment_error = Some("Select a Session before attaching a file".to_owned());
            return;
        }
        self.dialog = Some((DialogPurpose::Attach, dialog::open_file_async()));
        self.attachment_error = None;
        self.status = "Choose a file to attach…".to_owned();
    }

    /// Uploads the file the picker returned; the picker itself never runs here.
    fn attach_path(&mut self, path: PathBuf) {
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

    /// Applies a picked path to the document preview, which reads Host paths.
    fn open_preview_path(&mut self, path: &Path) {
        self.preview_path = path.display().to_string();
        if self.selected_session.is_some() && self.transport.is_some() {
            self.read_document();
        } else {
            self.preview_error =
                Some("Select a connected Session before reading a file.".to_owned());
        }
    }

    /// Writes the previewed text to a local path chosen through the save picker.
    fn save_preview_path(&mut self, path: PathBuf) {
        let Some(document) = self.preview_document.clone() else {
            self.status = "Save a document after previewing one".to_owned();
            return;
        };
        let text = document.text;
        let name = path.display().to_string();
        match std::fs::write(&path, text) {
            Ok(()) => self.status = format!("Saved preview to {name}"),
            Err(error) => self.status = format!("Could not save {name}: {error}"),
        }
    }

    fn poll_dialog(&mut self) {
        let Some((purpose, receiver)) = &self.dialog else {
            return;
        };
        let purpose = *purpose;
        match receiver.try_recv() {
            Ok(path) => {
                self.dialog = None;
                match (purpose, path) {
                    (DialogPurpose::Attach, Some(path)) => self.attach_path(path),
                    (DialogPurpose::Open, Some(path)) => self.open_preview_path(&path),
                    (DialogPurpose::Save, Some(path)) => self.save_preview_path(path),
                    (_, None) => self.status = "File picker cancelled".to_owned(),
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.dialog = None;
                self.status = "File picker stopped unexpectedly".to_owned();
            }
        }
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
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
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
        let (Some(session), Some(transport)) =
            (self.selected_session.clone(), self.transport.clone())
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
            let update = self
                .jobs_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
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
            let update = self
                .job_receiver
                .as_ref()
                .map(|receiver| receiver.try_recv());
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
                assistant_stream: true,
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

    fn render_update_prompt(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.update_prompt.clone() else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::new("native-update-confirmation")).show(ctx, |ui| {
            ui.heading(match prompt.action.as_str() {
                "download" => "Download desktop update?",
                "install" => "Install desktop update?",
                _ => "Desktop update",
            });
            ui.label(format!("Version {} is available.", prompt.version));
            if let Some(error) = &self.update_error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                if ui
                    .button(match prompt.action.as_str() {
                        "download" => "Download",
                        "install" => "Install and restart",
                        _ => "Continue",
                    })
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui.button("Not now").clicked() {
                    answer = Some(false);
                }
            });
        });
        let Some(accepted) = answer else {
            return;
        };
        let response = self
            .host
            .as_ref()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotConnected, "Host is unavailable")
            })
            .and_then(|host| host.respond_to_update_prompt(prompt.prompt_id, accepted));
        match response {
            Ok(()) => {
                self.update_prompt = None;
                self.update_error = None;
                self.status = if accepted {
                    format!("Update {} accepted", prompt.action)
                } else {
                    "Update postponed".to_owned()
                };
            }
            Err(error) => {
                self.update_error = Some(format!("Could not send update choice: {error}"))
            }
        }
    }
}

impl DesktopApp {
    /// Applies the client-local shortcut bindings; each chord toggles its view once.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let Ok([settings, marketplace, goals, new_session]) =
            self.ui_preferences.keybindings.chords()
        else {
            return;
        };
        if let Some(chord) = settings {
            if ctx.input_mut(|input| chord.consume(input)) {
                self.show_settings = !self.show_settings;
            }
        }
        if let Some(chord) = marketplace {
            if ctx.input_mut(|input| chord.consume(input)) {
                self.show_market = !self.show_market;
            }
        }
        if let Some(chord) = goals {
            if ctx.input_mut(|input| chord.consume(input)) {
                self.show_goals = !self.show_goals;
            }
        }
        if let Some(chord) = new_session {
            // Creating a Session replaces the view, so it must not fire while a
            // text editor owns the keyboard and would lose the draft.
            if !ctx.text_edit_focused() && ctx.input_mut(|input| chord.consume(input)) {
                self.request_sessions(SessionOperation::Create);
                self.focus_composer = true;
            }
        }
    }
}

impl eframe::App for DesktopApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tick(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render(ui);
    }
}

impl DesktopApp {
    /// Drains Host events and worker channels once per frame.
    ///
    /// Kept free of `eframe::Frame` so a windowless harness can run the same
    /// lifecycle the shipped shell runs.
    fn tick(&mut self, ctx: &egui::Context) {
        if let Ok(mut viewport) = self.viewport.lock() {
            *viewport = Some(ctx.clone());
        }
        self.poll_dialog();
        self.poll_attachment();
        self.poll_jobs();
        self.poll_schedules();
        if self.dialog.is_some()
            || self.jobs_receiver.is_some()
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
        if let Some(host) = self.host.as_mut() {
            while let Some(event) = host.event() {
                match event {
                    host::HostEvent::Failed(message) => {
                        eprintln!("dsh: native Host failed: {message}");
                        self.status = message;
                        self.update_prompt = None;
                        self.update_error = None;
                        self.host_url = None;
                        self.transport = None;
                        self.streams = None;
                        self.connect_receiver = None;
                        self.terminal.detach();
                    }
                    host::HostEvent::UpdatePrompt(prompt) => {
                        self.update_prompt = Some(prompt);
                        self.update_error = None;
                        ctx.request_repaint();
                    }
                    host::HostEvent::Restart => {
                        match restart::executable(std::env::var_os("APPIMAGE").map(Into::into))
                            .and_then(|path| restart::schedule(&path, std::process::id())) {
                            Ok(_) => {
                                self.quitting.store(true, Ordering::Release);
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                return;
                            }
                            Err(error) => eprintln!("dsh: native restart failed; restart the application manually: {error}"),
                        }
                    }
                }
            }
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
                    if self.host_url.is_some() {
                        eprintln!("dsh: native host=ready");
                    }
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
                    let terminal_probe =
                        std::env::var("DSH_DESKTOP_PROBE_TERMINAL").as_deref() == Ok("1");
                    let timed_question_probe =
                        std::env::var("DSH_DESKTOP_PROBE_TIMED_QUESTION").as_deref() == Ok("1");
                    if approval_probe {
                        // The probe owns the only `$events` reader so one waterfall
                        // answer cannot race a second connected client.
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_approval(probe_transport, probe_mux));
                    } else if timed_question_probe {
                        // Same exclusivity: the timed question is answered by the
                        // probe, not by a card the operator would have to click.
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || {
                            probe_timed_question(probe_transport, probe_mux)
                        });
                    } else {
                        self.start_remote_events(transport.clone(), streams.clone());
                    }
                    if terminal_probe {
                        let probe_transport = transport.clone();
                        let probe_mux = streams.clone();
                        std::thread::spawn(move || probe_terminal(probe_transport, probe_mux));
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
                Ok(Ok((through_seq, page, live))) => {
                    self.page_through_seq = Some(through_seq);
                    self.transcript.apply_snapshot(&live);
                    let before_seq = self.page_before_seq.take();
                    let oldest_incoming = page.records.first().map(|record| record.event.seq);
                    let advanced = match (before_seq, oldest_incoming) {
                        (Some(before_seq), Some(seq)) => seq < before_seq,
                        (None, _) => true,
                        (Some(_), None) => false,
                    };
                    // History and live events share one projection; the page
                    // is no longer kept as a second copy of the same events.
                    let history: Vec<_> = page
                        .records
                        .iter()
                        .map(|record| {
                            dsh_client_core::follow::SessionWireEvent::from(&record.event)
                        })
                        .collect();
                    for event in &history {
                        self.record_deliverables(event);
                    }
                    self.transcript.merge(history.iter());
                    self.page_has_more = page.has_more && advanced;
                    self.page_loading = false;
                    self.page_error = None;
                    self.page_receiver = None;
                }
                Ok(Err(error)) => {
                    self.page_loading = false;
                    self.page_before_seq = None;
                    self.page_error = Some(format!("Could not load session: {error}"));
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.page_loading = false;
                    self.page_before_seq = None;
                    self.page_error = Some("Session request stopped unexpectedly".to_owned());
                    self.page_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.goal_receiver {
            match receiver.try_recv() {
                Ok(Ok(goal)) => {
                    self.goal = goal;
                    if let Some(goal) = &self.goal {
                        self.goal_objective = goal.objective.clone();
                        self.goal_max_rounds = goal.max_goal_rounds.to_string();
                    } else {
                        self.goal_objective.clear();
                        self.goal_max_rounds.clear();
                    }
                    self.goal_loading = false;
                    self.goal_error = None;
                    self.goal_receiver = None;
                }
                Ok(Err(error)) => {
                    self.goal_loading = false;
                    self.goal_error = Some(format!("Could not load goal: {error}"));
                    self.goal_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.goal_loading = false;
                    self.goal_error = Some("Goal request stopped unexpectedly".to_owned());
                    self.goal_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.goal_mutation_receiver {
            match receiver.try_recv() {
                Ok(Ok(goal)) => {
                    self.goal = goal;
                    if let Some(goal) = &self.goal {
                        self.goal_objective = goal.objective.clone();
                        self.goal_max_rounds = goal.max_goal_rounds.to_string();
                        self.goal_status = Some("Goal updated".to_owned());
                    } else {
                        self.goal_objective.clear();
                        self.goal_status = Some("Goal cleared".to_owned());
                    }
                    self.goal_mutation_loading = false;
                    self.goal_error = None;
                    self.goal_mutation_receiver = None;
                }
                Ok(Err(error)) => {
                    self.goal_mutation_loading = false;
                    self.goal_error = Some(format!("Could not change goal: {error}"));
                    self.goal_mutation_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.goal_mutation_loading = false;
                    self.goal_error = Some("Goal update stopped unexpectedly".to_owned());
                    self.goal_mutation_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.plan_command_receiver {
            match receiver.try_recv() {
                Ok(Ok(Some(execution))) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    match execution.result {
                        CommandResult::Success { text, .. } => {
                            self.plan_command_status =
                                text.or_else(|| Some("Plan command completed".to_owned()));
                            if let Some(session_id) = self.selected_session.clone() {
                                self.load_session(session_id);
                            }
                        }
                        CommandResult::Error { text } => {
                            self.plan_command_status = Some(text);
                        }
                    }
                }
                Ok(Ok(None)) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status =
                        Some("The /plan command is not available in this Session".to_owned());
                }
                Ok(Err(error)) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status = Some(format!("Plan command failed: {error}"));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plan_command_loading = false;
                    self.plan_command_receiver = None;
                    self.plan_command_status = Some("Plan command stopped unexpectedly".to_owned());
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
                    self.model_requested_at = None;
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Could not load models: {error}"));
                    self.model_receiver = None;
                    self.model_requested_at = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Model catalog worker stopped unexpectedly".into());
                    self.model_receiver = None;
                    self.model_requested_at = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // A Host without a configured provider can leave this call
                    // unanswered; stop waiting so the picker can say why.
                    if self.model_requested_at.is_some_and(|started| {
                        started.elapsed() > std::time::Duration::from_secs(10)
                    }) {
                        self.model_receiver = None;
                        self.model_requested_at = None;
                        self.model_error = Some(
                            "The Host did not answer the model catalog; check provider settings"
                                .to_owned(),
                        );
                    }
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
        if let Some(receiver) = &self.model_init_receiver {
            match receiver.try_recv() {
                Ok(Ok(())) => {
                    self.model_init_receiver = None;
                    self.model_selection = None;
                    self.model_error = None;
                    self.refresh_models();
                }
                Ok(Err(error)) => {
                    self.model_error = Some(format!("Default model selection failed: {error}"));
                    self.model_init_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.model_error = Some("Default model worker stopped unexpectedly".into());
                    self.model_init_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.plugins_receiver {
            match receiver.try_recv() {
                Ok(Ok((snapshot, bundles, plugins))) => {
                    self.plugins_snapshot = Some(snapshot);
                    self.plugin_bundles = bundles;
                    self.managed_plugins = plugins;
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
                    self.plugins_error =
                        Some("Plugin inventory worker stopped unexpectedly".into());
                    self.plugins_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.plugin_write_receiver {
            match receiver.try_recv() {
                Ok(Ok(result)) => {
                    let warning = result
                        .warnings
                        .first()
                        .map(|warning| format!(" ({warning})"))
                        .unwrap_or_default();
                    self.plugin_write_status = Some(format!(
                        "{} · {}{warning}",
                        result.target, result.application
                    ));
                    self.plugin_write_receiver = None;
                    if result.changed {
                        self.refresh_plugins();
                    }
                }
                Ok(Err(error)) => {
                    self.plugins_error = Some(format!("Plugin change failed: {error}"));
                    self.plugin_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.plugins_error = Some("Plugin change worker stopped unexpectedly".into());
                    self.plugin_write_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.feedback_put_receiver {
            match receiver.try_recv() {
                Ok(Ok(item)) => {
                    if let Some(existing) = self
                        .feedback_items
                        .iter_mut()
                        .find(|x| x.message_id == item.message_id)
                    {
                        *existing = item;
                    } else {
                        self.feedback_items.push(item);
                    }
                    self.feedback_put_receiver = None;
                    self.feedback_status = Some("Feedback saved.".to_owned());
                }
                Ok(Err(error)) => {
                    self.feedback_status = Some(format!("Feedback failed: {error}"));
                    self.feedback_put_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.feedback_status = Some("Feedback worker stopped unexpectedly".into());
                    self.feedback_put_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.feedback_receiver {
            match receiver.try_recv() {
                Ok(Ok(items)) => {
                    self.feedback_items = items;
                    self.feedback_receiver = None;
                }
                Ok(Err(error)) => {
                    self.feedback_status = Some(format!("Feedback load failed: {error}"));
                    self.feedback_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.feedback_status = Some("Feedback worker stopped unexpectedly".into());
                    self.feedback_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.market_receiver {
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    if let Some(text) = value.as_str() {
                        self.market_status = Some(text.to_owned());
                        self.refresh_plugins();
                    } else if value.get("entries").is_some() {
                        self.market_results = value
                            .get("entries")
                            .and_then(serde_json::Value::as_array)
                            .cloned()
                            .unwrap_or_default();
                        self.market_status =
                            Some(format!("{} result(s)", self.market_results.len()));
                    } else if value.get("url").is_some() || value.is_array() {
                        let indexes = if value.is_array() {
                            value.as_array().cloned().unwrap_or_default()
                        } else {
                            vec![value.clone()]
                        };
                        self.market_indexes = indexes;
                        self.market_url = self
                            .market_indexes
                            .first()
                            .and_then(|index| index.get("url"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned);
                        self.market_status =
                            Some(format!("{} index(es)", self.market_indexes.len()));
                    }
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Ok(Err(error)) => {
                    self.market_status = Some(format!("Marketplace failed: {error}"));
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.market_status = Some("Marketplace worker stopped unexpectedly".into());
                    self.market_loading = false;
                    self.market_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.skills_receiver {
            match receiver.try_recv() {
                Ok(Ok(skills)) => {
                    self.skills = Some(skills);
                    self.skills_loading = false;
                    self.skills_error = None;
                    self.skills_receiver = None;
                }
                Ok(Err(error)) => {
                    self.skills_loading = false;
                    self.skills_error = Some(format!("Could not load skills: {error}"));
                    self.skills_receiver = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.skills_loading = false;
                    self.skills_error = Some("Skill list worker stopped unexpectedly".into());
                    self.skills_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
        if let Some(receiver) = &self.chat_receiver {
            match receiver.try_recv() {
                Ok(Ok(frame)) => match frame {
                    FollowFrame::Snapshot(snapshot) => {
                        self.transcript.apply_snapshot(&snapshot.records);
                        self.deliverables.clear();
                        for record in &snapshot.records {
                            self.record_deliverables(&record.event);
                        }
                    }
                    FollowFrame::Event(frame) => {
                        if frame.event.event_type == "assistant/message" {
                            self.assistant_draft.clear();
                        }
                        self.record_deliverables(&frame.event);
                        self.transcript.apply(&frame.event);
                    }
                    FollowFrame::AssistantStream { frame } => {
                        use dsh_client_core::follow::AssistantStreamFrame;
                        match &frame {
                            AssistantStreamFrame::Start { .. } => {
                                self.assistant_streaming = true;
                                self.assistant_draft.clear();
                            }
                            AssistantStreamFrame::Chunk { chunk, .. } => {
                                append_stream_text(&mut self.assistant_draft, chunk);
                            }
                            AssistantStreamFrame::End { .. } => {
                                self.assistant_streaming = false;
                            }
                        }
                    }
                },
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
                    } else if invocation.event == "user-questions/request" {
                        match PendingQuestion::from_invocation(invocation) {
                            Ok(pending) => self.pending_questions.push(QuestionCard::new(pending)),
                            Err(error) => {
                                self.page_error = Some(format!("Invalid question request: {error}"))
                            }
                        }
                    } else if let Some(commands) = &self.approval_commands {
                        let _ = commands.send(ApprovalCommand::Delegate(invocation));
                    }
                }
                Ok(Ok(RemoteEvent::Cancel { event_id })) => {
                    self.pending_approvals
                        .retain(|pending| pending.invocation.event_id != event_id);
                    self.pending_questions.retain(|card| {
                        card.pending()
                            .is_some_and(|pending| pending.invocation.event_id != event_id)
                    });
                }
                Ok(Ok(RemoteEvent::Emit { .. })) => {}
                Ok(Err(error)) => {
                    self.page_error = Some(format!("Permission event stream failed: {error}"));
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                    self.pending_questions.clear();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status = "Permission event stream closed".to_owned();
                    self.approval_events_receiver = None;
                    self.approval_commands = None;
                    self.pending_approvals.clear();
                    self.pending_questions.clear();
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

    /// Draws the whole shell into the frame's root Ui.
    ///
    /// The caller owns the window, so a windowless harness renders the same
    /// panels by owning the `egui::Ui` instead.
    fn render(&mut self, ui: &mut egui::Ui) {
        if self.font_catalog_snapshot.is_none() {
            self.font_catalog_snapshot = fonts::catalog_snapshot(&self.font_catalog);
            if self.font_catalog_snapshot.is_none() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
            self.apply_fonts(ui.ctx());
        }
        if self.applied_ui_preferences.as_ref() != Some(&self.ui_preferences) {
            self.apply_ui_preferences(ui.ctx());
        }
        // A CJK character anywhere switches the fallback chain on; dropping the
        // 19 MiB collection when nothing needs it keeps peak memory down.
        if self.font_catalog_snapshot.is_some() && self.applied_cjk != Some(self.cjk_required()) {
            self.apply_fonts(ui.ctx());
        }
        self.handle_shortcuts(ui.ctx());
        let mut nav_rail_width = 0.0;
        let mut sessions_width = 0.0;
        let mut central_size = egui::Vec2::ZERO;
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
        // Navigation rail: compact, icon-first, grouped.
        egui::Panel::left("nav_rail")
            .resizable(false)
            .exact_size(layout::NAV_RAIL_WIDTH)
            .show(ui, |ui| {
                nav_rail_width = ui.max_rect().width();
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("DSH").strong().size(11.0));
                    ui.add_space(8.0);
                    let items = [
                        ("S", "Sessions", ActiveView::Sessions),
                        ("D", "Document", ActiveView::Document),
                        ("P", "Plugins", ActiveView::Plugins),
                        ("K", "Skills", ActiveView::Skills),
                        ("M", "Market", ActiveView::Marketplace),
                        ("G", "Goals", ActiveView::Goals),
                        ("T", "Settings", ActiveView::Settings),
                    ];
                    for (icon, tooltip, view) in items {
                        let selected = self.active_view == view;
                        let btn = ui.add(
                            egui::Button::new(egui::RichText::new(icon).size(16.0).strong())
                                .min_size(egui::vec2(40.0, 40.0)),
                        );
                        let rect = btn.rect;
                        let clicked = btn.clicked();
                        btn.on_hover_text(tooltip);
                        if selected {
                            ui.painter()
                                .rect_filled(rect, 8.0, ui.visuals().selection.bg_fill);
                        }
                        if clicked {
                            self.active_view = view;
                            match view {
                                ActiveView::Sessions => {
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    self.show_goals = false;
                                }
                                ActiveView::Settings => {
                                    self.open_settings();
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    self.show_goals = false;
                                }
                                ActiveView::Document => {
                                    self.show_document = true;
                                    self.show_settings = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    self.show_goals = false;
                                }
                                ActiveView::Plugins => {
                                    self.show_plugins = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    self.show_goals = false;
                                    if self.plugins_snapshot.is_none() {
                                        self.refresh_plugins();
                                    }
                                }
                                ActiveView::Skills => {
                                    self.show_skills = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_market = false;
                                    self.show_goals = false;
                                    if self.skills.is_none() {
                                        self.refresh_skills();
                                    }
                                }
                                ActiveView::Marketplace => {
                                    self.show_market = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_goals = false;
                                    if self.market_indexes.is_empty() {
                                        self.refresh_market_indexes();
                                    }
                                }
                                ActiveView::Goals => {
                                    self.show_goals = true;
                                    self.show_settings = false;
                                    self.show_document = false;
                                    self.show_plugins = false;
                                    self.show_skills = false;
                                    self.show_market = false;
                                    self.refresh_goal();
                                }
                            }
                        }
                    }
                });
            });
        egui::Panel::left("sessions")
            .resizable(true)
            .default_size(self.ui_preferences.session_panel_width)
            .min_size(layout::SIDEBAR_MIN)
            .max_size(layout::SIDEBAR_MAX)
            .show(ui, |ui| {
                sessions_width = ui.max_rect().width();
                if ui
                    .selectable_label(self.active_view == ActiveView::Sessions, "Sessions")
                    .clicked()
                {
                    self.active_view = ActiveView::Sessions;
                    self.show_settings = false;
                    self.show_document = false;
                    self.show_plugins = false;
                    self.show_skills = false;
                    self.show_market = false;
                    self.show_goals = false;
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Settings, "Settings")
                    .clicked()
                {
                    self.active_view = ActiveView::Settings;
                    self.open_settings();
                    self.show_document = false;
                    self.show_plugins = false;
                    self.show_skills = false;
                    self.show_market = false;
                    self.show_goals = false;
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Document, "Document")
                    .clicked()
                {
                    self.active_view = ActiveView::Document;
                    self.show_document = true;
                    self.show_settings = false;
                    self.show_plugins = false;
                    self.show_skills = false;
                    self.show_market = false;
                    self.show_goals = false;
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Plugins, "Plugins")
                    .clicked()
                {
                    self.active_view = ActiveView::Plugins;
                    self.show_plugins = true;
                    self.show_settings = false;
                    self.show_document = false;
                    self.show_skills = false;
                    self.show_market = false;
                    self.show_goals = false;
                    if self.plugins_snapshot.is_none() {
                        self.refresh_plugins();
                    }
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Skills, "Skills")
                    .clicked()
                {
                    self.active_view = ActiveView::Skills;
                    self.show_skills = true;
                    self.show_settings = false;
                    self.show_document = false;
                    self.show_plugins = false;
                    self.show_market = false;
                    self.show_goals = false;
                    if self.skills.is_none() {
                        self.refresh_skills();
                    }
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Marketplace, "Marketplace")
                    .clicked()
                {
                    self.active_view = ActiveView::Marketplace;
                    self.show_market = true;
                    self.show_settings = false;
                    self.show_document = false;
                    self.show_plugins = false;
                    self.show_skills = false;
                    self.show_goals = false;
                    if self.market_indexes.is_empty() {
                        self.refresh_market_indexes();
                    }
                }
                if ui
                    .selectable_label(self.active_view == ActiveView::Goals, "Goals")
                    .clicked()
                {
                    self.active_view = ActiveView::Goals;
                    self.show_goals = true;
                    self.show_settings = false;
                    self.show_document = false;
                    self.show_plugins = false;
                    self.show_skills = false;
                    self.show_market = false;
                    self.refresh_goal();
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
                            self.request_sessions(SessionOperation::Search(
                                self.session_query.clone(),
                            ));
                        }
                    }
                    if let Some(session_id) = self.selected_session.clone() {
                        let title_field = ui.add(
                            egui::TextEdit::singleline(&mut self.session_title)
                                .hint_text("New session title")
                                .desired_width(170.0),
                        );
                        accessible_label(&title_field, "Session title");
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(
                                    !self.session_title.trim().is_empty(),
                                    egui::Button::new("Rename selected"),
                                )
                                .clicked()
                            {
                                self.request_sessions(SessionOperation::Rename(
                                    RenameSessionRequest {
                                        session_id: session_id.clone(),
                                        title: self.session_title.trim().to_owned(),
                                    },
                                ));
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
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading sessions…");
                    });
                }
                if !self.sessions_status.is_empty() {
                    ui.small(&self.sessions_status);
                }
                if self.sessions.is_empty() && self.session_search.is_none() && !busy && connected {
                    ui.add_space(8.0);
                    ui.label("No sessions yet.");
                    ui.small("Create one to start chatting.");
                }
                if let Some(error) = &self.sessions_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    if ui.button("Retry").clicked() {
                        self.session_query.clear();
                        self.refresh_sessions();
                    }
                }
                ui.add_enabled_ui(!busy, |ui| {
                    let mut selected = session_list::render(
                        ui,
                        &self.sessions,
                        self.session_search.as_ref(),
                        self.selected_session.as_deref(),
                        now_millis(),
                    );
                    // Keyboard navigation in session list
                    if !ui.ctx().egui_wants_keyboard_input()
                        && ui.input(|i| {
                            i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::ArrowUp)
                        })
                    {
                        let direction = if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                            1
                        } else {
                            -1
                        };
                        let sessions = if self.session_search.is_some() {
                            self.session_search
                                .as_ref()
                                .map(|r| {
                                    r.items
                                        .iter()
                                        .map(|i| i.session_id.clone())
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                        } else {
                            self.sessions.iter().map(|s| s.session_id.clone()).collect()
                        };
                        let current_idx = self
                            .selected_session
                            .as_ref()
                            .and_then(|id| sessions.iter().position(|session| session == id));
                        selected = session_list::next_index(current_idx, direction, sessions.len())
                            .map(|index| sessions[index].clone());
                    }
                    if let Some(session_id) = selected {
                        self.load_session(session_id);
                    }
                });
                if ui.button("Open file").clicked() {
                    self.dialog = Some((DialogPurpose::Open, dialog::open_file_async()));
                }
                if ui.button("Save file").clicked() {
                    self.dialog = Some((DialogPurpose::Save, dialog::save_file_async()));
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
        if !self.deliverables.is_empty() {
            egui::Panel::bottom("deliverables-panel").show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Deliverables ({})", self.deliverables.len()))
                    .default_open(true)
                    .show(ui, |ui| {
                        let mut preview = None;
                        for file in &self.deliverables {
                            let label = egui::RichText::new(&file.path).monospace();
                            if ui
                                .link(label)
                                .on_hover_text(
                                    file.description
                                        .as_deref()
                                        .unwrap_or("Open this deliverable"),
                                )
                                .clicked()
                            {
                                preview = Some(file.path.clone());
                            }
                            if let Some(description) = &file.description {
                                ui.small(description);
                            }
                        }
                        if let Some(path) = preview {
                            self.preview_path = path.clone();
                            self.show_document = true;
                            self.show_settings = false;
                            self.show_plugins = false;
                            self.show_skills = false;
                            if Self::is_office_path(&path) {
                                self.read_office_document();
                            } else {
                                self.read_document();
                            }
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
            central_size = ui.available_size();
            if self.transport.is_none() && !self.show_settings {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading("Not connected");
                    ui.label("The Host sidecar is not running or authentication failed.");
                    ui.label("Check DSH_DESKTOP_HOST_BIN and the runtime tree.");
                    if ui.button("Retry connection").clicked() {
                        self.connect();
                    }
                });
                return;
            }
            if !self.show_settings
                && !self.show_plugins
                && !self.show_skills
                && !self.show_market
                && !self.show_document
                && !self.show_goals
                && self.selected_session.is_none()
            {
                ui.vertical_centered(|ui| {
                    ui.add_space(80.0);
                    ui.heading("No session selected");
                    ui.label("Open an existing session from the sidebar, or start a new one.");
                    ui.add_space(12.0);
                    if ui.button("New Session").clicked() {
                        self.request_sessions(SessionOperation::Create);
                    }
                    if ui.button("Refresh sessions").clicked() {
                        self.session_query.clear();
                        self.refresh_sessions();
                    }
                });
                return;
            }
            if self.show_settings {
                if let Some(action) = self.ui_preferences_panel.ui(
                    ui,
                    &self.ui_preferences_path,
                    self.font_catalog_snapshot.as_ref(),
                ) {
                    self.handle_ui_preferences_action(action, ui.ctx());
                }
                ui.separator();
                if self.transport.is_some() {
                    let mut settings_action = None;
                    ui.collapsing("Host settings (agent and plugin configuration)", |ui| {
                        settings_action = self.settings_panel.ui(
                            ui,
                            self.settings_description.as_ref(),
                            self.settings_loading,
                            self.settings_error.as_deref(),
                            self.settings_update_receiver.is_some(),
                        );
                    });
                    match settings_action {
                        Some(SettingsAction::Refresh) => self.refresh_settings(),
                        Some(SettingsAction::Mutate(request)) => self.mutate_settings(request),
                        None => {}
                    }
                } else {
                    ui.label("Connect to the Host to view or change agent and plugin settings.");
                }
            } else if self.show_goals {
                self.render_goals_panel(ui);
            } else if self.show_plugins {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading("Plugins");
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(!self.plugins_loading, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh_plugins();
                    }
                });
                ui.add_space(4.0);
                if self.plugins_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading plugins…");
                    });
                }
                if let Some(error) = &self.plugins_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(status) = &self.plugin_write_status {
                    ui.label(status);
                }
                let busy = self.plugin_write_receiver.is_some();
                let mut plugin_action = None;
                if let Some(snapshot) = &self.plugins_snapshot {
                    if snapshot.management_available == Some(false) {
                        ui.label("Plugin management is unavailable for this Host.");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("plugin-inventory")
                        .max_height(420.0)
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
                                ui.horizontal(|ui| {
                                    ui.label(format!("{} · {state} · {phase}", entry.module_name));
                                    let managed = self
                                        .managed_plugins
                                        .iter()
                                        .find(|plugin| plugin.entry_id == entry.entry_id);
                                    match managed {
                                        Some(plugin) if plugin.patch_id.is_some() => {
                                            let label =
                                                if entry.enabled { "Disable" } else { "Enable" };
                                            if ui
                                                .add_enabled(
                                                    !busy,
                                                    egui::Button::new(label).small(),
                                                )
                                                .clicked()
                                            {
                                                plugin_action = Some(PluginAction::SetPlugin {
                                                    id: entry.entry_id.clone(),
                                                    enabled: !entry.enabled,
                                                });
                                            }
                                        }
                                        Some(plugin) => {
                                            if let Some(reason) = &plugin.read_only_reason {
                                                ui.small(format!("read-only: {reason}"));
                                            }
                                        }
                                        None => {}
                                    }
                                });
                            }
                            if !self.plugin_bundles.is_empty() {
                                ui.separator();
                                ui.heading("Bundles");
                                for bundle in &self.plugin_bundles {
                                    ui.horizontal(|ui| {
                                        let state = if bundle.enabled {
                                            "enabled"
                                        } else {
                                            "disabled"
                                        };
                                        ui.label(format!(
                                            "{} · {state} · {}",
                                            bundle.name, bundle.availability
                                        ));
                                        let label =
                                            if bundle.enabled { "Disable" } else { "Enable" };
                                        if ui
                                            .add_enabled(!busy, egui::Button::new(label).small())
                                            .clicked()
                                        {
                                            plugin_action = Some(PluginAction::SetBundle {
                                                name: bundle.name.clone(),
                                                enabled: !bundle.enabled,
                                            });
                                        }
                                    });
                                    if let Some(description) = &bundle.description {
                                        ui.small(description);
                                    }
                                }
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
                if let Some(action) = plugin_action {
                    self.write_plugin(action);
                }
                if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            } else if self.show_market {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading("Marketplace");
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(!self.market_loading, egui::Button::new("Refresh indexes"))
                        .clicked()
                    {
                        self.refresh_market_indexes();
                    }
                });
                ui.add_space(4.0);
                if self.market_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading…");
                    });
                }
                if let Some(status) = &self.market_status {
                    ui.label(status);
                }
                ui.horizontal(|ui| {
                    let query = ui.add(
                        egui::TextEdit::singleline(&mut self.market_query)
                            .hint_text("Search text")
                            .desired_width(240.0),
                    );
                    accessible_label(&query, "Marketplace search");
                    let search = ui.add_enabled(!self.market_loading, egui::Button::new("Search"));
                    // The Sessions panel also has a control named "Search"; the
                    // marketplace button needs its own accessible name.
                    accessible_button_label(&search, "Search marketplace");
                    if search.clicked() {
                        self.search_market();
                    }
                });
                if self.market_indexes.is_empty() && !self.market_loading {
                    ui.label("No market indexes configured.");
                }
                let mut install_spec = None;
                if !self.market_results.is_empty() {
                    egui::ScrollArea::vertical()
                        .id_salt("market-results")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            for entry in &self.market_results {
                                let name = entry
                                    .get("name")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("plugin");
                                ui.horizontal(|ui| {
                                    ui.label(name);
                                    let default_spec = entry
                                        .get("versions")
                                        .and_then(serde_json::Value::as_array)
                                        .and_then(|versions| versions.first())
                                        .and_then(|version| version.get("install"))
                                        .and_then(|install| install.get("spec"))
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_owned);
                                    if let Some(spec) = default_spec {
                                        if ui
                                            .add_enabled(
                                                !self.market_loading,
                                                egui::Button::new("Install").small(),
                                            )
                                            .clicked()
                                        {
                                            install_spec = Some(spec);
                                        }
                                    }
                                });
                                if let Some(description) =
                                    entry.get("description").and_then(serde_json::Value::as_str)
                                {
                                    ui.small(description);
                                }
                                ui.separator();
                            }
                        });
                }
                if let Some(spec) = install_spec {
                    self.install_market_entry(spec);
                }
            } else if self.show_skills {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading("Skills");
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(!self.skills_loading, egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.refresh_skills();
                    }
                });
                ui.add_space(4.0);
                if self.skills_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading skills…");
                    });
                }
                if self.selected_session.is_none() {
                    ui.label("Select a session to list the skills its composer can invoke.");
                }
                if let Some(error) = &self.skills_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if let Some(skills) = &self.skills {
                    egui::ScrollArea::vertical()
                        .id_salt("skill-list")
                        .max_height(420.0)
                        .show(ui, |ui| {
                            if skills.skills.is_empty() {
                                ui.label("No skills are available to this session.");
                            }
                            for skill in &skills.skills {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&skill.name).strong())
                                        .wrap(),
                                );
                                ui.add(
                                    egui::Label::new(&skill.description).wrap().selectable(true),
                                );
                                if let Some(when) = &skill.when_to_use {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(format!("When: {when}"))
                                                .color(ui.visuals().weak_text_color()),
                                        )
                                        .wrap(),
                                    );
                                }
                                if !skill.model_invocable {
                                    ui.small("Not model-invocable");
                                }
                                if let Some(path) = &skill.path {
                                    ui.small(path);
                                }
                                ui.separator();
                            }
                        });
                } else if !self.skills_loading {
                    ui.label("Open this tab with a session selected to load skills.");
                }
            } else if self.show_document {
                ui.horizontal(|ui| {
                    let path = ui.add(
                        egui::TextEdit::singleline(&mut self.preview_path)
                            .hint_text("Host file path")
                            .desired_width(360.0),
                    );
                    accessible_label(&path, "Document path");
                    if ui
                        .add_enabled(!self.preview_loading, egui::Button::new("Preview"))
                        .clicked()
                    {
                        if Self::is_office_path(&self.preview_path) {
                            self.read_office_document();
                        } else {
                            self.read_document();
                        }
                    }
                    if Self::is_office_path(&self.preview_path)
                        && ui
                            .add_enabled(
                                !self.preview_loading,
                                egui::Button::new("Render Office preview"),
                            )
                            .clicked()
                    {
                        self.read_office_document();
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
                if self.page_loading && self.transcript.is_empty() {
                    ui.label("Loading session…");
                } else if let Some(error) = &self.page_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                    if ui.button("Retry session load").clicked() {
                        if let Some(session_id) = self.selected_session.clone() {
                            self.load_session(session_id);
                        }
                    }
                } else if self.transcript.is_empty() {
                    ui.label("This session has no events");
                } else if self.page_has_more {
                    if self.page_loading {
                        ui.label("Loading older events…");
                    } else if ui.button("Load older events").clicked() {
                        self.load_older_session_page();
                    }
                } else if self.page_loading {
                    ui.label("Loading older events…");
                }
                ui.separator();
                ui.heading("Chat");
                ui.horizontal_wrapped(|ui| {
                    let selected = self.model_picker_label();
                    let model_picker = egui::ComboBox::from_id_salt("model-selection")
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
                                            reasoning_effort: model.reasoning.as_ref().and_then(
                                                |reasoning| reasoning.default_effort.clone(),
                                            ),
                                        };
                                        let current =
                                            self.model_selection.as_ref().is_some_and(|current| {
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
                    accessible_label(&model_picker.response, "Model");
                    if ui
                        .add_enabled(self.model_receiver.is_none(), egui::Button::new("Reload"))
                        .clicked()
                    {
                        self.refresh_models();
                    }
                    if ui
                        .add_enabled(
                            self.model_init_receiver.is_none()
                                && self.model_select_receiver.is_none(),
                            egui::Button::new("Use profile default"),
                        )
                        .clicked()
                    {
                        self.initialize_default_model();
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
                if self.model_selection.is_none() {
                    if let Some(error) = &self.model_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                        if ui.button("Open Settings").clicked() {
                            self.show_settings = true;
                        }
                    } else if self.model_catalog_empty() {
                        ui.label("Add a provider key in Settings to choose a model.");
                        if ui.button("Open Settings").clicked() {
                            self.show_settings = true;
                        }
                    }
                } else if let Some(error) = &self.model_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                let text_edit = ui.add_enabled(
                    !self.chat_loading,
                    egui::TextEdit::multiline(&mut self.chat_text)
                        .id_salt("composer")
                        .hint_text("Message (Ctrl+Enter to send)")
                        .desired_rows(3),
                );
                accessible_label(&text_edit, "Composer");
                if self.focus_composer && !self.chat_loading {
                    text_edit.request_focus();
                    self.focus_composer = false;
                }
                if composer_input::submit_requested(
                    text_edit.has_focus(),
                    ui.input(|input| input.key_pressed(egui::Key::Enter)),
                    ui.input(|input| input.modifiers.ctrl),
                ) && !self.chat_loading
                    && !self.chat_text.trim().is_empty()
                    && self.attachment_receiver.is_none()
                {
                    self.send_prompt();
                }
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
                let mut feedback_actions = Vec::new();
                if let Some(status) = &self.feedback_status {
                    ui.label(status);
                }
                let feedback: std::collections::BTreeMap<String, String> = self
                    .feedback_items
                    .iter()
                    .map(|item| (item.message_id.clone(), item.rating.clone()))
                    .collect();
                render_transcript(
                    ui,
                    &self.transcript,
                    &self.assistant_draft,
                    &feedback,
                    self.chat_loading,
                    self.assistant_streaming,
                    &mut feedback_actions,
                );
                for action in feedback_actions {
                    let if_version = self
                        .feedback_items
                        .iter()
                        .find(|item| item.message_id == action.0)
                        .map(|item| item.version.clone());
                    let rating = action.1.clone();
                    let request = MessageFeedbackPutRequest {
                        session_id: self.selected_session.clone().unwrap_or_default(),
                        message_id: action.0,
                        rating: rating.clone(),
                        note: None,
                        category: None,
                        if_version,
                    };
                    let Some(transport) = self.transport.clone() else {
                        continue;
                    };
                    let (sender, receiver) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(MessageFeedbackClient::new(&transport).put(request));
                    });
                    self.feedback_put_receiver = Some(receiver);
                    self.feedback_status = Some(format!("Rating {}…", rating));
                }
                if let Some(card) = self.pending_questions.first_mut() {
                    let action = card.render(ui);
                    if let Some(action) = action {
                        if let Some(commands) = &self.approval_commands {
                            if let Some(pending) = card.pending().cloned() {
                                let command = match action {
                                    QuestionAction::Answer => {
                                        ApprovalCommand::AnswerQuestion(pending, card.answer())
                                    }
                                    QuestionAction::Skip => {
                                        ApprovalCommand::AnswerQuestion(pending, card.skip())
                                    }
                                };
                                let _ = commands.send(command);
                                self.pending_questions.remove(0);
                            }
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
                        let mut choice = None;
                        ui.horizontal(|ui| {
                            if ui.button("Allow once").clicked() {
                                choice =
                                    Some(ApprovalChoice::Answer(ApprovalDecision::AllowedOnce));
                            }
                            if ui.button("Reject").clicked() {
                                choice = Some(ApprovalChoice::Answer(ApprovalDecision::Rejected));
                            }
                            if ui.button("Delegate").clicked() {
                                choice = Some(ApprovalChoice::Delegate);
                            }
                        });
                        if let Some(choice) = choice {
                            let event_id = pending.invocation.event_id.clone();
                            let command = match choice {
                                ApprovalChoice::Answer(decision) => {
                                    ApprovalCommand::Answer(pending, decision)
                                }
                                ApprovalChoice::Delegate => {
                                    ApprovalCommand::Delegate(pending.invocation.clone())
                                }
                            };
                            let _ = commands.send(command);
                            self.pending_approvals
                                .retain(|item| item.invocation.event_id != event_id);
                        }
                    }
                }
            }
        });
        if std::env::var("DSH_DESKTOP_DEBUG_LAYOUT").as_deref() == Ok("1") {
            let viewport = ui
                .ctx()
                .input(|input| input.viewport().inner_rect)
                .unwrap_or(egui::Rect::ZERO);
            eprintln!(
                "dsh: layout viewport={}x{} nav_rail={} sessions={} central={}x{}",
                viewport.width() as i32,
                viewport.height() as i32,
                nav_rail_width as i32,
                sessions_width as i32,
                central_size.x as i32,
                central_size.y as i32
            );
        }
        self.render_update_prompt(ui.ctx());
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
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Appends or refreshes the files a `deliverables/presented` event declared.
fn record_deliverables(
    deliverables: &mut Vec<DeliverableFile>,
    event: &dsh_client_core::follow::SessionWireEvent,
) {
    if event.event_type != "deliverables/presented" {
        return;
    }
    let Some(files) = event
        .data
        .get("files")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    for file in files {
        let Some(path) = file.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let description = file
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        match deliverables.iter_mut().find(|entry| entry.path == path) {
            Some(entry) => entry.description = description,
            None => deliverables.push(DeliverableFile {
                path: path.to_owned(),
                description,
            }),
        }
    }
}

/// Appends one assistant-stream text delta, bounded to the transcript budget.
fn append_stream_text(draft: &mut String, chunk: &serde_json::Value) {
    if chunk.get("type").and_then(serde_json::Value::as_str) != Some("text-delta") {
        return;
    }
    let Some(text) = chunk.get("text").and_then(serde_json::Value::as_str) else {
        return;
    };
    if draft.len() + text.len() > transcript::MAX_BYTES {
        return;
    }
    draft.push_str(text);
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

/// Gives a widget a stable AccessKit name for windowless UI tests.
///
/// `egui::TextEdit` takes no label; it reports its hint text as a placeholder,
/// so a harness cannot address the field by name without this.
///
/// @param response - Response of the widget that needs a name.
/// @param label - Name the accessibility tree and the test harness read.
pub(crate) fn accessible_label(response: &egui::Response, label: &str) {
    response.output_event(egui::output::OutputEvent::ValueChanged(
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, response.enabled(), label),
    ));
}

/// Replaces a control's accessible name when its visible text is ambiguous.
fn accessible_button_label(response: &egui::Response, label: &str) {
    response.output_event(egui::output::OutputEvent::ValueChanged(
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label),
    ));
}

/// Labels the model picker: the chosen model, or why there is nothing to pick.
///
/// @param selection - Model already chosen for this Session.
/// @param error - Last catalog or selection failure.
/// @param catalog - Loaded catalog, or `None` when none loaded.
/// @param pending - Whether a catalog request is still in flight.
/// @returns Text for the picker and for its empty-state hint.
fn model_picker_label(
    selection: Option<&ModelSelection>,
    error: Option<&str>,
    catalog: Option<&ModelCatalog>,
    pending: bool,
) -> String {
    if let Some(selection) = selection {
        return format!("{} / {}", selection.provider, selection.model);
    }
    if error.is_some() {
        return "Model unavailable".to_owned();
    }
    if pending {
        return "Model: loading…".to_owned();
    }
    match catalog {
        Some(catalog) if catalog.groups.iter().all(|group| group.models.is_empty()) => {
            "No provider configured".to_owned()
        }
        Some(_) => "Choose a model".to_owned(),
        None => "No provider configured".to_owned(),
    }
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
                    match events.submit(&invocation, RemoteEventOutcome::Result { value: answer }) {
                        Ok(()) => {
                            eprintln!("dsh: native question=answered");
                            if !roundtrip_reported && std::path::Path::new(&marker).exists() {
                                eprintln!("dsh: native approval=roundtrip-ok");
                                probe_job_follow(&mux, &session);
                            }
                            if probe_question_result(&mux, &session) {
                                eprintln!("dsh: native question=answer-ok");
                                if probe_transcript(&mux, &session) {
                                    eprintln!("dsh: native transcript=ok");
                                } else {
                                    eprintln!("dsh: native transcript=failed");
                                }
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
                                eprintln!(
                                    "dsh: native approval=failed: no {expected} decision logged"
                                );
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

/// Runs only when `DSH_DESKTOP_PROBE_TERMINAL=1`.
///
/// Exercises the terminal path the panel uses against a live Host: create a
/// Session, create a terminal through the RPCs, attach over the shared mux,
/// write one command, and read the streamed output back. The lane's probe reads
/// the `dsh: native terminal=` marker.
fn probe_terminal(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::Duration;
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: session create: {error}");
            return;
        }
    };
    let client = TerminalClient::new(&transport, session.as_str());
    let environment = match client.environment() {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: environment: {error}");
            return;
        }
    };
    if let Err(error) = client.create(TerminalCreateRequest {
        shell_path: None,
        id: "probe-terminal".to_owned(),
        cols: 80,
        rows: 24,
    }) {
        eprintln!("dsh: native terminal=failed: create: {error}");
        return;
    }
    let (mut frames, input) = match terminal::attach(&mux, session.as_str(), "probe-terminal") {
        Ok(attachment) => attachment,
        Err(error) => {
            eprintln!("dsh: native terminal=failed: attach: {error}");
            return;
        }
    };
    let command = "printf 'terminal-probe-ok\\n'\n";
    if input
        .send(TerminalInput::Write {
            sequence: 1,
            data: command.to_owned(),
        })
        .is_err()
    {
        eprintln!("dsh: native terminal=failed: input closed");
        return;
    }
    let mut output = String::new();
    let mut exit_code = None;
    while let Some(frame) = frames.recv_timeout(Duration::from_secs(20)) {
        match frame {
            Ok(TerminalFrame::Output(chunk)) => output.push_str(&chunk.data),
            Ok(TerminalFrame::State(state)) => exit_code = state.info.exit_code,
            Ok(_) => {}
            Err(error) => {
                eprintln!("dsh: native terminal=failed: frame: {error}");
                return;
            }
        }
        if output.contains("terminal-probe-ok") {
            break;
        }
    }
    let _ = input.send(TerminalInput::Write {
        sequence: 2,
        data: "exit\n".to_owned(),
    });
    let _ = client.close(TerminalCloseRequest {
        id: "probe-terminal".to_owned(),
    });
    if output.contains("terminal-probe-ok") {
        eprintln!(
            "dsh: native terminal=ok max_input={} max_cols={} exit={}",
            environment.max_input_bytes,
            environment.max_cols,
            exit_code.map(|code| code.to_string()).unwrap_or_default()
        );
    } else {
        eprintln!(
            "dsh: native terminal=failed: no marker in output bytes={}",
            output.len()
        );
    }
}

/// Runs only when `DSH_DESKTOP_PROBE_TIMED_QUESTION=1`.
///
/// Answers a timed `ask_user_question` the way the app does, but also proves the
/// foreground wait stream the question card subscribes to. The lane's probe
/// reads the `dsh: native timed-question=` markers.
fn probe_timed_question(transport: HttpTransport, mux: RemoteStreamMux) {
    use std::time::{Duration, Instant};
    let session = match SessionsClient::new(&transport).create(CreateSessionRequest {
        workspace_id: None,
        cwd: None,
        session_id: None,
        agent_preset: None,
    }) {
        Ok(session) => session.session_id,
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: session create: {error}");
            return;
        }
    };
    let model = match ModelClient::new(&transport).catalog() {
        Ok(catalog) => match probe_model_selection(&catalog) {
            Some(selection) => selection,
            None => {
                eprintln!("dsh: native timed-question=failed: empty model catalog");
                return;
            }
        },
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: model catalog: {error}");
            return;
        }
    };
    if let Err(error) = ModelClient::new(&transport).select(SelectModelRequest {
        session_id: session.clone(),
        selection: model,
    }) {
        eprintln!("dsh: native timed-question=failed: select model: {error}");
        return;
    }
    let events = match RemoteEvents::connect(transport.clone(), &mux) {
        Ok(events) => events,
        Err(error) => {
            eprintln!("dsh: native timed-question=failed: events: {error}");
            return;
        }
    };
    let chat = ChatClient::new(&transport);
    let request = PromptRequest {
        request_id: chat.next_request_id(),
        session_id: session.clone(),
        mode: PromptMode::Queue,
        content: vec![PromptContentPart::Text {
            text: "Ask the timed probe question.".to_owned(),
        }],
        client_time_zone: None,
    };
    if let Err(error) = chat.prompt(request) {
        eprintln!("dsh: native timed-question=failed: prompt: {error}");
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(RemoteEvent::Waterfall(invocation))) => {
                if invocation.event != "user-questions/request" {
                    continue;
                }
                let pending = match PendingQuestion::from_invocation(invocation) {
                    Ok(pending) => pending,
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: decode: {error}");
                        return;
                    }
                };
                let Some(wait) = pending.request.wait.clone() else {
                    eprintln!("dsh: native timed-question=failed: blocking question");
                    return;
                };
                if !wait.timed {
                    eprintln!("dsh: native timed-question=failed: not timed");
                    return;
                }
                let Some(call_id) = wait.call_id.clone() else {
                    eprintln!("dsh: native timed-question=failed: no call id");
                    return;
                };
                let mut frames = 0_u32;
                let mut remaining = 0_u64;
                match streams::user_question_wait(&mux, session.as_str(), &call_id) {
                    Ok(mut stream) => {
                        while let Some(frame) = stream.recv_timeout(Duration::from_secs(5)) {
                            match frame {
                                Ok(frame) => {
                                    frames += 1;
                                    remaining = remaining.max(frame.remaining_ms);
                                }
                                Err(error) => {
                                    eprintln!(
                                        "dsh: native timed-question=failed: wait frame: {error}"
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: attach wait: {error}");
                        return;
                    }
                }
                eprintln!("dsh: native timed-question=attached frames={frames}");
                if frames == 0 {
                    eprintln!("dsh: native timed-question=failed: no wait frames");
                    return;
                }
                // The foreground wait ends at the model's timeout; the answer then
                // arrives as a continued-question reply.
                std::thread::sleep(Duration::from_secs(3));
                let answer = AskUserQuestionAnswer {
                    answers: pending
                        .request
                        .questions
                        .iter()
                        .map(|question| AskUserQuestionAnswerItem {
                            id: question.id.clone(),
                            selected: Vec::new(),
                            custom: Some("timed probe answer".to_owned()),
                        })
                        .collect(),
                };
                match UserQuestionsClient::new(&transport).answer(&session, &call_id, answer) {
                    Ok(true) => eprintln!(
                        "dsh: native timed-question=ok frames={frames} remaining_ms={remaining}"
                    ),
                    Ok(false) => eprintln!("dsh: native timed-question=failed: not answerable"),
                    Err(error) => {
                        eprintln!("dsh: native timed-question=failed: answer: {error}");
                    }
                }
                return;
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(error) => {
                eprintln!("dsh: native timed-question=failed: events closed: {error}");
                return;
            }
        }
    }
    eprintln!("dsh: native timed-question=failed: timeout");
}

/// Reads the current session snapshot, retrying after a failed open.
fn probe_snapshot(mux: &RemoteStreamMux, session: &str) -> Option<follow::SnapshotFrame> {
    let request = FollowRequest {
        address: SessionAddress {
            kind: SessionAddressKind::Session,
            session_id: session.to_owned(),
        },
        max_messages: Some(200),
        turn_window: None,
        assistant_stream: false,
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
                    && event
                        .data
                        .get("outcome")
                        .and_then(serde_json::Value::as_str)
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
fn user_question_answer(
    request: &serde_json::Map<String, serde_json::Value>,
) -> AskUserQuestionAnswer {
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

/// Projects the live session log into transcript rows and checks the kinds.
fn probe_transcript(mux: &RemoteStreamMux, session: &str) -> bool {
    let Some(snapshot) = probe_snapshot(mux, session) else {
        eprintln!("dsh: native transcript=failed: no snapshot");
        return false;
    };
    let mut transcript = Transcript::default();
    transcript.apply_snapshot(&snapshot.records);
    let rows = transcript.rows();
    let has = |kind| rows.iter().any(|row| row.kind == kind);
    let ok = has(RowKind::User)
        && has(RowKind::Assistant)
        && has(RowKind::ToolResult)
        && has(RowKind::Approval)
        && has(RowKind::Job)
        && rows.len() >= 5;
    eprintln!(
        "dsh: native transcript rows={} user={} assistant={} tool={} approval={} job={}",
        rows.len(),
        has(RowKind::User),
        has(RowKind::Assistant),
        has(RowKind::ToolResult),
        has(RowKind::Approval),
        has(RowKind::Job)
    );
    ok
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
            catalog
                .groups
                .iter()
                .map(|group| group.models.len())
                .sum::<usize>(),
            catalog.failures.len()
        ),
        Err(error) => eprintln!("dsh: native models=failed: {error}"),
    }
}

/// Maps a screen position inside the terminal grid to a one-based cell.
///
/// The raster uses `FontId::monospace(12.0)`; pointer rows therefore count
/// visual rows, which match program rows for a full-screen alternate buffer.
fn terminal_cell(ui: &egui::Ui, rect: egui::Rect, position: egui::Pos2) -> Option<(usize, usize)> {
    if !rect.contains(position) {
        return None;
    }
    let font = egui::FontId::monospace(12.0);
    let (width, height) = ui
        .painter()
        .fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    Some((
        (((position.x - rect.left()) / width).floor() as usize) + 1,
        (((position.y - rect.top()) / height).floor() as usize) + 1,
    ))
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
    TrayIconEvent::set_event_handler(Some(move |event| {
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
            Ok(Box::new(DesktopApp::new(ShellServices {
                quitting: app_quitting,
                show_requested: app_show,
                hide_requested: app_hide,
                hidden: app_hidden,
                viewport: app_viewport,
                tray_available: app_tray_available,
                ui_preferences_path: UiPreferences::config_path(),
                spawn_host: true,
            })))
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

#[cfg(test)]
mod transcript_render_tests {
    use super::render_transcript;
    use crate::transcript::Transcript;
    use dsh_client_core::follow::SessionWireEvent;
    use eframe::egui;
    use serde_json::json;

    fn event(seq: i64, event_type: &str, data: serde_json::Value) -> SessionWireEvent {
        SessionWireEvent {
            event_type: event_type.into(),
            seq,
            time: 0,
            data,
            ignorable: None,
            source_event_seqs: None,
            surface_op: None,
        }
    }

    fn render(transcript: &Transcript) -> (Vec<String>, egui::Rect) {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("transcript"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        render_transcript(
            &mut ui,
            transcript,
            "",
            &std::collections::BTreeMap::new(),
            false,
            false,
            &mut Vec::new(),
        );
        assert!(ui.min_rect().is_finite());
        let rect = ui.min_rect();
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        (texts, rect)
    }

    fn collect_text(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, texts);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn renders_the_streaming_draft() {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("draft"),
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 480.0),
            )),
        );
        render_transcript(
            &mut ui,
            &Transcript::default(),
            "partial answer",
            &std::collections::BTreeMap::new(),
            false,
            true,
            &mut Vec::new(),
        );
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        for expected in [
            "Waiting for assistant…",
            "Assistant (streaming)",
            "partial answer",
        ] {
            assert!(texts.iter().any(|text| text == expected), "{texts:?}");
        }
    }

    #[test]
    fn records_deliverables_once_and_refreshes_the_description() {
        use crate::record_deliverables;
        let mut files = Vec::new();
        record_deliverables(
            &mut files,
            &event(
                1,
                "deliverables/presented",
                json!({"files": [
                    {"path": "out/report.pdf", "description": "First"},
                    {"path": "out/data.csv"}
                ]}),
            ),
        );
        record_deliverables(
            &mut files,
            &event(
                2,
                "deliverables/presented",
                json!({"files": [{"path": "out/report.pdf", "description": "Final"}]}),
            ),
        );
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].description.as_deref(), Some("Final"));
        assert_eq!(files[1].description, None);
    }

    #[test]
    fn renders_titles_bodies_and_the_empty_state() {
        let mut transcript = Transcript::default();
        transcript.apply(&event(
            1,
            "user/message",
            json!({"content": [{"type": "text", "text": "hello transcript"}]}),
        ));
        transcript.apply(&event(
            2,
            "tool/result",
            json!({"message": {"isError": true, "content": [{"type": "text", "text": "exit 1"}]}}),
        ));
        let (texts, _) = render(&transcript);
        for expected in ["You", "hello transcript", "Tool error", "exit 1"] {
            assert!(texts.iter().any(|text| text == expected), "{texts:?}");
        }

        let (empty, _) = render(&Transcript::default());
        assert!(empty
            .iter()
            .any(|text| text == "No messages in this window yet."));
    }

    #[test]
    fn long_transcripts_stay_inside_the_viewport_scroll_region() {
        let mut transcript = Transcript::default();
        for seq in 1..=1000 {
            transcript.apply(&event(
                seq,
                "user/message",
                json!({"content": format!("Message {seq}: {}", "text ".repeat(20))}),
            ));
        }
        let (_, rect) = render(&transcript);
        assert!(rect.height() <= 480.0, "transcript UI overflowed: {rect:?}");
        assert_eq!(transcript.rows().len(), 1000);
    }
}

#[cfg(test)]
mod composer_identity {
    use super::*;

    /// The composer keeps one id while the transcript above it changes size, so a
    /// focused buffer also receives typed text on later frames.
    #[test]
    fn composer_focus_survives_a_changing_transcript() {
        let context = egui::Context::default();
        let mut text = String::new();
        let mut ids = Vec::new();
        for (rows, typed) in [(1_usize, None), (4, Some("hi"))] {
            let mut input = egui::RawInput::default();
            if let Some(typed) = typed {
                input.events.push(egui::Event::Text(typed.to_owned()));
            }
            let mut output = context.run_ui(input, |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    for _ in 0..rows {
                        ui.label("transcript row");
                    }
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .id_salt("composer")
                            .hint_text("Message (Ctrl+Enter to send)")
                            .desired_rows(3),
                    );
                    ids.push(response.id);
                    if typed.is_some() {
                        assert!(response.has_focus(), "the composer lost focus");
                    }
                });
            });
            output.textures_delta.clear();
            context.memory_mut(|memory| memory.request_focus(ids[0]));
        }
        assert_eq!(ids[0], ids[1]);
        assert_eq!(text, "hi");
    }
}

#[cfg(test)]
mod model_status_tests {
    use super::*;
    use dsh_client_core::models::ModelProviderGroup;

    fn selection() -> ModelSelection {
        ModelSelection {
            provider: "deepseek-official".to_owned(),
            model: "deepseek-chat".to_owned(),
            reasoning_effort: None,
        }
    }

    fn catalog(models: usize) -> ModelCatalog {
        ModelCatalog {
            default: selection(),
            routable_providers: Vec::new(),
            groups: vec![ModelProviderGroup {
                id: "deepseek-official".to_owned(),
                name: "DeepSeek".to_owned(),
                models: (0..models)
                    .map(|index| ModelCatalogModel {
                        id: format!("model-{index}"),
                        name: format!("Model {index}"),
                        description: None,
                        reasoning: None,
                    })
                    .collect(),
            }],
            failures: Vec::new(),
        }
    }

    #[test]
    fn picker_states_name_the_selected_model_or_the_blocker() {
        assert_eq!(
            model_picker_label(Some(&selection()), None, None, false),
            "deepseek-official / deepseek-chat"
        );
        assert_eq!(
            model_picker_label(None, Some("offline"), Some(&catalog(1)), false),
            "Model unavailable"
        );
        assert_eq!(
            model_picker_label(None, None, None, true),
            "Model: loading…"
        );
        assert_eq!(
            model_picker_label(None, None, Some(&catalog(0)), false),
            "No provider configured"
        );
        assert_eq!(
            model_picker_label(None, None, Some(&catalog(2)), false),
            "Choose a model"
        );
        assert_eq!(
            model_picker_label(None, None, None, false),
            "No provider configured"
        );
    }
}
