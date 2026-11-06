//! Interactive terminal panel state shared by the gpui shell.

use crate::terminal_ansi;
use crate::terminal_input;
use dsh_client_core::{
    terminal::{
        TerminalClient, TerminalCloseRequest, TerminalCreateRequest, TerminalEnvironment,
        TerminalFrame, TerminalInput, TerminalInputLimits, TerminalRenameRequest, TerminalWriter,
        WebTerminalInfo,
    },
    transport::HttpTransport,
    remote_stream::RemoteStreamMux,
    terminal, Error,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

pub(crate) const TERMINAL_VISIBLE_BYTES: usize = 128 * 1024;

/// Keeps the newest UTF-8 output, including when one frame exceeds the budget.
pub(crate) fn append_bounded_output(output: &mut String, text: &str) {
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

pub(crate) enum TerminalOperation {
    List,
    Create(TerminalCreateRequest),
    Rename(TerminalRenameRequest),
    Close(TerminalCloseRequest),
}

pub(crate) struct TerminalInventory {
    items: Vec<WebTerminalInfo>,
    environment: TerminalEnvironment,
    selected: Option<String>,
}

/// What the running file picker is collecting.
pub(crate) enum TerminalUpdate {
    Attached(mpsc::Sender<TerminalInput>),
    Frame(Box<TerminalFrame>),
    Error(String),
}


/// Owns workers until they settle; cancellation never joins a network worker on the render frame.
#[derive(Default)]
pub(crate) struct TerminalPanel {
    pub(crate) session: Option<String>,
    pub(crate) items: Vec<WebTerminalInfo>,
    pub(crate) environment: Option<TerminalEnvironment>,
    pub(crate) selected: Option<String>,
    pub(crate) inventory: Option<mpsc::Receiver<Result<TerminalInventory, String>>>,
    pub(crate) updates: Option<mpsc::Receiver<TerminalUpdate>>,
    pub(crate) input: Option<TerminalWriter>,
    pub(crate) stop: Option<mpsc::Sender<()>>,
    pub(crate) workers: Vec<std::thread::JoinHandle<()>>,
    pub(crate) output: String,
    pub(crate) output_revision: u64,
    pub(crate) parsed: Vec<terminal_ansi::AnsiLine>,
    pub(crate) parsed_revision: u64,
    pub(crate) parsed_width: u32,
    pub(crate) text: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) snapshot_status: String,
    pub(crate) state_status: String,
    pub(crate) ack_status: String,
    pub(crate) error: Option<String>,
    pub(crate) writable: bool,
    pub(crate) pending: Option<u64>,
    pub(crate) queued_input: std::collections::VecDeque<String>,
    pub(crate) queued_input_bytes: usize,
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    /// Input modes the attached program enabled.
    pub(crate) modes: terminal_ansi::InputModes,
    pub(crate) mode_tracker: terminal_ansi::InputModeTracker,
    /// Set when the panel gate should poll again soon.
    pub(crate) dirty: bool,
    /// Whether typed keys go straight to the program instead of a submit line.
    pub(crate) raw_keys: bool,
    /// First selected output line, by index into [`Self::parsed`].
    pub(crate) selection_start: Option<usize>,
    /// Last selected output line, which may precede the first.
    pub(crate) selection_end: Option<usize>,
}

/// Removes one Office preview's PDF and raster files when conversion finishes.
pub(crate) struct PreviewScratchDirectory(pub(crate) PathBuf);

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
    pub(crate) fn detach(&mut self) {
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
        self.status = "Detached (shell remains alive)".to_owned();
    }

    /// Answers a program's device or cursor query through the same uplink.
    pub(crate) fn reply_to_program(&mut self, replies: &str) {
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

    pub(crate) fn clear_output(&mut self) {
        self.output.clear();
        self.output_revision = self.output_revision.wrapping_add(1);
        self.mode_tracker.reset();
        self.modes = terminal_ansi::InputModes::default();
        self.clear_selection();
    }

    /// Drops the output selection.
    pub(crate) fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
    }

    /// Starts a selection at one output line.
    pub(crate) fn begin_selection(&mut self, line: usize) {
        self.selection_start = Some(line);
        self.selection_end = Some(line);
    }

    /// Extends the selection to one output line.
    pub(crate) fn extend_selection(&mut self, line: usize) {
        if self.selection_start.is_some() {
            self.selection_end = Some(line);
        }
    }

    /// The inclusive selected line range, in display order.
    pub(crate) fn selection_range(&self) -> Option<(usize, usize)> {
        let start = self.selection_start?;
        let end = self.selection_end.unwrap_or(start);
        Some(if start <= end {
            (start, end)
        } else {
            (end, start)
        })
    }

    /// Whether one output line is inside the selection.
    pub(crate) fn line_selected(&self, line: usize) -> bool {
        self.selection_range()
            .is_some_and(|(start, end)| (start..=end).contains(&line))
    }

    /// Text of the selected lines, or `None` when nothing is selected.
    pub(crate) fn selected_output(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let lines: Vec<String> = self
            .parsed
            .iter()
            .skip(start)
            .take(end.saturating_sub(start) + 1)
            .map(output_line_text)
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    pub(crate) fn push_output(&mut self, text: &str) {
        self.modes = self.mode_tracker.feed(text);
        append_bounded_output(&mut self.output, text);
        self.output_revision = self.output_revision.wrapping_add(1);
    }

    pub(crate) fn select_session(
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

    pub(crate) fn request(&mut self, transport: HttpTransport, operation: TerminalOperation) {
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

    pub(crate) fn request_list(&mut self, transport: HttpTransport) {
        self.request(transport, TerminalOperation::List);
    }

    pub(crate) fn create_default(&mut self, transport: HttpTransport) {
        let id = format!(
            "native-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.request(
            transport,
            TerminalOperation::Create(TerminalCreateRequest {
                shell_path: None,
                id,
                cols: self.cols,
                rows: self.rows,
            }),
        );
    }

    pub(crate) fn select_terminal(&mut self, id: String) {
        if self.selected.as_deref() == Some(id.as_str()) {
            return;
        }
        self.detach();
        self.clear_output();
        self.text.clear();
        self.snapshot_status.clear();
        self.state_status.clear();
        self.ack_status.clear();
        self.error = None;
        self.title = self
            .items
            .iter()
            .find(|item| item.id == id)
            .map_or_else(String::new, |item| item.title.clone());
        if let Some(item) = self.items.iter().find(|item| item.id == id) {
            self.cols = item.cols;
            self.rows = item.rows;
        }
        self.selected = Some(id);
    }

    pub(crate) fn attach_selected(&mut self, streams: RemoteStreamMux) {
        self.attach(streams);
    }

    pub(crate) fn close_selected(&mut self, transport: HttpTransport) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.detach();
        self.request(
            transport,
            TerminalOperation::Close(TerminalCloseRequest { id }),
        );
    }

    pub(crate) fn rename_selected(&mut self, transport: HttpTransport, title: String) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if title.trim().is_empty() {
            return;
        }
        self.request(
            transport,
            TerminalOperation::Rename(TerminalRenameRequest {
                id,
                title: title.trim().to_owned(),
            }),
        );
    }

    /// Rebuilds the parsed screen when output or width changed.
    pub(crate) fn refresh_screen(&mut self) {
        if self.parsed_revision != self.output_revision || self.parsed_width != self.cols {
            let screen = terminal_ansi::parse_screen(&self.output, self.cols as usize);
            self.parsed = screen.lines;
            self.parsed_revision = self.output_revision;
            self.parsed_width = self.cols;
            if !screen.replies.is_empty() {
                self.reply_to_program(&screen.replies);
            }
        }
    }

    /// Current attachment state, named for the panel header.
    pub(crate) fn lifecycle_label(&self) -> &'static str {
        if self.error.is_some() {
            return "error";
        }
        if self.updates.is_some() {
            return if self.snapshot_status.is_empty() {
                "starting"
            } else {
                "attached"
            };
        }
        if self.selected.is_some() {
            "detached"
        } else {
            "idle"
        }
    }

    pub(crate) fn attach(&mut self, streams: RemoteStreamMux) {
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

    pub(crate) fn poll(&mut self) {
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
            self.dirty = true;
        }
    }

    pub(crate) fn send(&mut self, resize: bool) {
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
    pub(crate) fn send_encoded(&mut self, data: &str) -> bool {
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

    pub(crate) fn write_encoded(&mut self, data: &str) -> bool {
        if !self.writable {
            return false;
        }
        let result = {
            let Some(writer) = &mut self.input else {
                return false;
            };
            writer
                .write(data)
                .map(|sequence| (sequence, writer.is_closed()))
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

    pub(crate) fn flush_queued_input(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(data) = self.queued_input.pop_front() {
            self.queued_input_bytes = self.queued_input_bytes.saturating_sub(data.len());
            self.write_encoded(&data);
        }
    }

    /// Sends the command text framed as a paste when the program asked for it.
    pub(crate) fn send_paste(&mut self) {
        let data = terminal_input::paste(&self.text, self.modes.bracketed_paste);
        if self.send_encoded(&data) {
            self.text.clear();
        }
    }

    /// Sends one or more pointer gestures at a one-based grid cell.
    pub(crate) fn send_mouse(&mut self, actions: &[terminal_input::MouseAction], cell: (usize, usize)) {
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
mod tests {
    use super::*;

    #[test]
    fn append_bounded_output_keeps_short_frames() {
        let mut output = String::new();
        append_bounded_output(&mut output, "hello");
        assert_eq!(output, "hello");
    }

    #[test]
    fn append_bounded_output_caps_history_at_budget() {
        let mut output = String::new();
        append_bounded_output(&mut output, &"a".repeat(TERMINAL_VISIBLE_BYTES + 10));
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
    }

    #[test]
    fn append_bounded_output_clears_on_oversized_frame() {
        let mut output = String::new();
        append_bounded_output(&mut output, &"b".repeat(TERMINAL_VISIBLE_BYTES + 10));
        assert_eq!(output.len(), TERMINAL_VISIBLE_BYTES);
        assert!(output.starts_with('b'));
    }
}

/// Editable text of one parsed output line.
pub(crate) fn output_line_text(line: &terminal_ansi::AnsiLine) -> String {
    line.spans.iter().map(|span| span.text.as_str()).collect()
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    use dsh_client_core::terminal::{
        TerminalOutputFrame, TerminalShell, TerminalSnapshotFrame, TerminalState,
    };

    fn info() -> WebTerminalInfo {
        WebTerminalInfo {
            id: "t1".to_owned(),
            title: "shell".to_owned(),
            shell: TerminalShell {
                path: "/bin/sh".to_owned(),
                args: Vec::new(),
                name: "sh".to_owned(),
            },
            cwd: "/tmp".to_owned(),
            cols: 80,
            rows: 24,
            state: TerminalState::Running,
            exit_code: None,
            error: None,
        }
    }

    fn snapshot() -> TerminalFrame {
        TerminalFrame::Snapshot(TerminalSnapshotFrame {
            sequence: 1,
            screen: "hello".to_owned(),
            info: info(),
            writable: true,
            input_window: None,
        })
    }

    fn attached_panel() -> (TerminalPanel, mpsc::Sender<TerminalUpdate>) {
        let (sender, receiver) = mpsc::channel();
        let mut panel = TerminalPanel::default();
        panel.items = vec![info()];
        panel.selected = Some("t1".to_owned());
        panel.updates = Some(receiver);
        (panel, sender)
    }

    #[test]
    fn snapshot_and_output_frames_populate_the_screen() {
        let (mut panel, sender) = attached_panel();
        sender
            .send(TerminalUpdate::Frame(Box::new(snapshot())))
            .unwrap();
        panel.poll();
        assert_eq!(panel.output, "hello");
        assert!(panel.writable);
        assert_eq!(panel.lifecycle_label(), "attached");

        sender
            .send(TerminalUpdate::Frame(Box::new(TerminalFrame::Output(
                TerminalOutputFrame {
                    sequence: 2,
                    data: " world".to_owned(),
                },
            ))))
            .unwrap();
        panel.poll();
        assert_eq!(panel.output, "hello world");
        panel.refresh_screen();
        let first: String = panel.parsed[0]
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(first, "hello world");
    }

    #[test]
    fn a_dropped_stream_reports_detached_and_keeps_scrollback() {
        let (mut panel, sender) = attached_panel();
        sender
            .send(TerminalUpdate::Frame(Box::new(snapshot())))
            .unwrap();
        panel.poll();
        drop(sender);
        panel.poll();
        assert_eq!(panel.lifecycle_label(), "detached");
        assert_eq!(panel.selected.as_deref(), Some("t1"));
        assert_eq!(panel.output, "hello");
        assert_eq!(panel.status, "Terminal stream ended");
    }

    #[test]
    fn detach_keeps_the_terminal_and_screen_for_reconnect() {
        let (mut panel, sender) = attached_panel();
        sender
            .send(TerminalUpdate::Frame(Box::new(snapshot())))
            .unwrap();
        panel.poll();
        panel.detach();
        assert_eq!(panel.lifecycle_label(), "detached");
        assert_eq!(panel.selected.as_deref(), Some("t1"));
        assert_eq!(panel.output, "hello");
        assert!(panel.input.is_none());
        assert_eq!(panel.status, "Detached (shell remains alive)");
    }

    #[test]
    fn a_new_session_clears_the_previous_terminals() {
        let (mut panel, sender) = attached_panel();
        sender
            .send(TerminalUpdate::Frame(Box::new(snapshot())))
            .unwrap();
        panel.poll();
        panel.select_session(Some("other".to_owned()), None, None);
        assert!(panel.selected.is_none());
        assert!(panel.output.is_empty());
        assert!(panel.items.is_empty());
        assert_eq!(panel.lifecycle_label(), "idle");
    }

    #[test]
    fn line_selection_copies_the_selected_range() {
        let mut panel = TerminalPanel::default();
        panel.push_output("first\nsecond\nthird");
        panel.refresh_screen();
        assert!(panel.selected_output().is_none());
        panel.begin_selection(0);
        panel.extend_selection(1);
        assert!(panel.line_selected(0));
        assert!(panel.line_selected(1));
        assert!(!panel.line_selected(2));
        assert_eq!(panel.selected_output().as_deref(), Some("first\nsecond"));
    }

    #[test]
    fn selection_accepts_a_reversed_range() {
        let mut panel = TerminalPanel::default();
        panel.push_output("a\nb\nc");
        panel.refresh_screen();
        panel.begin_selection(2);
        panel.extend_selection(1);
        assert_eq!(panel.selection_range(), Some((1, 2)));
        assert_eq!(panel.selected_output().as_deref(), Some("b\nc"));
    }

    #[test]
    fn clearing_output_drops_the_selection() {
        let mut panel = TerminalPanel::default();
        panel.push_output("a\nb");
        panel.refresh_screen();
        panel.begin_selection(0);
        panel.clear_output();
        assert!(panel.selected_output().is_none());
        assert!(!panel.line_selected(0));
    }

    #[test]
    fn terminal_panel_starts_with_submit_line_input() {
        assert!(!TerminalPanel::default().raw_keys);
    }

    #[test]
    fn terminal_panel_push_output_accumulates_and_bumps_revision() {
        let mut panel = TerminalPanel::default();
        let before = panel.output_revision;
        panel.push_output("hi");
        assert_eq!(panel.output, "hi");
        assert_eq!(panel.output_revision, before.wrapping_add(1));
        panel.push_output(" there");
        assert_eq!(panel.output, "hi there");
    }

    #[test]
    fn terminal_panel_clear_output_empties_the_screen() {
        let mut panel = TerminalPanel::default();
        panel.push_output("text");
        panel.clear_output();
        assert!(panel.output.is_empty());
        assert!(panel.output_revision > 0);
    }
}
