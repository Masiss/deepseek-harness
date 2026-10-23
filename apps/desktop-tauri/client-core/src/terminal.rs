//! Typed terminal HTTP operations and authenticated terminal WebSocket streams.

use crate::{
    remote_stream::{RemoteStreamMux, Uplink},
    transport::HttpTransport,
    Error,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

/// A terminal identity returned by the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebTerminalInfo {
    pub id: String,
    pub title: String,
    pub shell: TerminalShell,
    pub cwd: String,
    pub cols: u32,
    pub rows: u32,
    pub state: TerminalState,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Current terminal process state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalState {
    Running,
    Exited,
    Failed,
}

/// Session terminal environment limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEnvironment {
    pub cwd: String,
    pub max_input_bytes: u32,
    pub max_buffered_bytes: u32,
    pub max_cols: u32,
    pub max_rows: u32,
    pub scrollback: u32,
}

/// An executable shell available to the terminal provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalShell {
    pub path: String,
    pub args: Vec<String>,
    pub name: String,
}

/// Request to create a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCreateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_path: Option<String>,
    pub id: String,
    pub cols: u32,
    pub rows: u32,
}

/// Request to rename a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRenameRequest {
    pub id: String,
    pub title: String,
}

/// Request to close a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCloseRequest {
    pub id: String,
}

/// One input command sent to an attached terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TerminalInput {
    #[serde(rename = "write")]
    Write { sequence: u64, data: String },
    #[serde(rename = "resize")]
    Resize { sequence: u64, cols: u32, rows: u32 },
}

/// A terminal output frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOutputFrame {
    pub sequence: u64,
    pub data: String,
}

/// A terminal state frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStateFrame {
    pub info: WebTerminalInfo,
    pub writable: bool,
}

/// An acknowledgement for one terminal input sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TerminalAckFrame {
    pub sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<TerminalRefusal>,
}

/// Why terminal input was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalRefusal {
    ReadOnly,
    NotRunning,
}

/// A complete screen snapshot sent when an attachment opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSnapshotFrame {
    pub sequence: u64,
    pub screen: String,
    pub info: WebTerminalInfo,
    pub writable: bool,
    #[serde(default)]
    pub input_window: Option<RemoteUplinkWindow>,
}

/// One decoded terminal attachment frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TerminalFrame {
    #[serde(rename = "snapshot")]
    Snapshot(TerminalSnapshotFrame),
    #[serde(rename = "output")]
    Output(TerminalOutputFrame),
    #[serde(rename = "state")]
    State(TerminalStateFrame),
    #[serde(rename = "ack")]
    Ack(TerminalAckFrame),
}

/// One retained terminal stream frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TerminalRetainedFrame {
    #[serde(rename = "retained")]
    Retained,
}

/// Request for terminal list operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalListRequest {
    pub session_id: String,
}

/// Request for terminal retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRetainRequest {
    pub session_id: String,
    pub id: String,
}

/// Native terminal HTTP client.
pub struct TerminalClient<'a> {
    transport: &'a HttpTransport,
    agent_id: String,
}
impl<'a> TerminalClient<'a> {
    /// Creates a terminal client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport, agent_id: impl Into<String>) -> Self {
        Self {
            transport,
            agent_id: agent_id.into(),
        }
    }
    fn request<T: for<'de> Deserialize<'de>>(&self, method: &str, args: Value) -> Result<T, Error> {
        let cancel = AtomicBool::new(false);
        let value = self.transport.request(method, args, &cancel)?;
        Ok(serde_json::from_value(value)?)
    }
    /// Returns terminal environment limits.
    pub fn environment(&self) -> Result<TerminalEnvironment, Error> {
        self.request(
            "terminal/environment",
            serde_json::json!({"agentId":self.agent_id}),
        )
    }
    /// Returns executable shells.
    pub fn shells(&self) -> Result<Vec<TerminalShell>, Error> {
        self.request(
            "terminal/shells",
            serde_json::json!({"agentId":self.agent_id}),
        )
    }
    /// Lists terminals for a session.
    pub fn list(&self, session_id: impl Into<String>) -> Result<Vec<WebTerminalInfo>, Error> {
        self.request(
            "terminal/list",
            serde_json::to_value(TerminalListRequest {
                session_id: session_id.into(),
            })?,
        )
    }
    /// Creates a terminal.
    pub fn create(&self, request: TerminalCreateRequest) -> Result<WebTerminalInfo, Error> {
        self.request(
            "terminal/create",
            serde_json::json!({"agentId":self.agent_id,"request":request}),
        )
    }
    /// Renames a terminal.
    pub fn rename(&self, request: TerminalRenameRequest) -> Result<(), Error> {
        self.request::<Value>(
            "terminal/rename",
            serde_json::json!({"agentId":self.agent_id,"id":request.id,"title":request.title}),
        )
        .map(|_| ())
    }
    /// Closes a terminal.
    pub fn close(&self, request: TerminalCloseRequest) -> Result<(), Error> {
        self.request::<Value>(
            "terminal/close",
            serde_json::json!({"agentId":self.agent_id,"id":request.id}),
        )
        .map(|_| ())
    }
}

/// Stops a terminal stream.
#[derive(Clone)]
pub struct TerminalCancel {
    cancelled: Arc<AtomicBool>,
    uplink: Uplink,
}
impl TerminalCancel {
    /// Requests stream shutdown without waiting for the socket worker to exit.
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        self.uplink.cancel();
    }
}
/// Receiving end of a terminal stream.
pub struct TerminalStream<F> {
    receiver: mpsc::Receiver<Result<F, Error>>,
    cancel: TerminalCancel,
}
impl<F> TerminalStream<F> {
    /// Returns the cancellation handle.
    pub fn cancel_handle(&self) -> &TerminalCancel {
        &self.cancel
    }
}
impl<F> Iterator for TerminalStream<F> {
    type Item = Result<F, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}
impl<F> Drop for TerminalStream<F> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

fn decode_stream<F: serde::de::DeserializeOwned + Send + 'static>(
    receiver: mpsc::Receiver<Result<Value, Error>>,
    cancel: &TerminalCancel,
) -> mpsc::Receiver<Result<F, Error>> {
    let (mapped_tx, mapped_rx) = mpsc::channel();
    let cancelled = Arc::clone(&cancel.cancelled);
    let uplink = cancel.uplink.clone();
    thread::spawn(move || {
        for item in receiver {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            let result = item.and_then(|value| serde_json::from_value(value).map_err(Error::from));
            let failed = result.is_err();
            if mapped_tx.send(result).is_err() || failed {
                break;
            }
        }
        cancelled.store(true, Ordering::Release);
        uplink.cancel();
    });
    mapped_rx
}

/// Gateway capacity for encoded uplink frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteUplinkWindow {
    pub max_bytes: usize,
    pub frame_overhead_bytes: usize,
}

/// Session terminal limits applied to one attachment's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalInputLimits {
    pub max_input_bytes: usize,
    pub max_buffered_bytes: usize,
    pub max_cols: u32,
    pub max_rows: u32,
}

/// Input rejected before it reaches the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TerminalInputError {
    #[error("terminal input exceeds the session byte budget")]
    InputFull,
    #[error("terminal attachment is closed")]
    Closed,
}

struct QueuedInput {
    command: TerminalInput,
    frame_bytes: usize,
}

/// Client-side input queue that honors the raw input and encoded-frame budgets.
///
/// A command stays queued while the carrier window is full; acknowledgements
/// release capacity. Sequence numbers never repeat within one attachment.
pub struct TerminalWriter {
    sender: mpsc::Sender<TerminalInput>,
    limits: TerminalInputLimits,
    window: Option<RemoteUplinkWindow>,
    sequence: u64,
    input_bytes: usize,
    buffered_bytes: usize,
    in_flight: std::collections::BTreeMap<u64, usize>,
    queued: std::collections::VecDeque<QueuedInput>,
    closed: bool,
}

impl TerminalWriter {
    /// Creates a writer over the attachment's uplink.
    pub fn new(sender: mpsc::Sender<TerminalInput>, limits: TerminalInputLimits) -> Self {
        Self {
            sender,
            limits,
            window: None,
            sequence: 0,
            input_bytes: 0,
            buffered_bytes: 0,
            in_flight: std::collections::BTreeMap::new(),
            queued: std::collections::VecDeque::new(),
            closed: false,
        }
    }

    /// Applies the carrier window carried by an attachment snapshot.
    pub fn set_window(&mut self, window: RemoteUplinkWindow) {
        self.window = Some(window);
        self.flush();
    }

    /// Oldest unacknowledged sequence, if any.
    pub fn pending(&self) -> Option<u64> {
        self.in_flight.keys().next().copied()
    }

    /// True after the uplink failed or the attachment ended.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Writes exact text, or queues it until the carrier has room.
    pub fn write(&mut self, data: &str) -> Result<u64, TerminalInputError> {
        if self.closed {
            return Err(TerminalInputError::Closed);
        }
        if data.is_empty() {
            return Ok(self.sequence);
        }
        let bytes = data.len();
        if self.input_bytes + bytes > self.limits.max_input_bytes {
            return Err(TerminalInputError::InputFull);
        }
        let sequence = self.sequence + 1;
        let command = TerminalInput::Write {
            sequence,
            data: data.to_owned(),
        };
        self.enqueue(command, bytes)?;
        Ok(sequence)
    }

    /// Resizes the terminal, clamping to the session limit and ordering with writes.
    pub fn resize(&mut self, cols: u32, rows: u32) -> Result<u64, TerminalInputError> {
        if self.closed {
            return Err(TerminalInputError::Closed);
        }
        let cols = cols.min(self.limits.max_cols);
        let rows = rows.min(self.limits.max_rows);
        let sequence = self.sequence + 1;
        let command = TerminalInput::Resize {
            sequence,
            cols,
            rows,
        };
        self.enqueue(command, 0)?;
        Ok(sequence)
    }

    /// Releases capacity for one acknowledged command.
    pub fn acknowledge(&mut self, sequence: u64) {
        if let Some(frame_bytes) = self.in_flight.remove(&sequence) {
            self.buffered_bytes = self.buffered_bytes.saturating_sub(frame_bytes);
            self.flush();
        }
    }

    /// Marks the uplink unusable; queued commands are dropped.
    pub fn close(&mut self) {
        self.closed = true;
        self.queued.clear();
        self.in_flight.clear();
        self.buffered_bytes = 0;
    }

    fn enqueue(
        &mut self,
        command: TerminalInput,
        bytes: usize,
    ) -> Result<(), TerminalInputError> {
        let frame_bytes =
            serde_json::to_vec(&command).map_or(0, |value| value.len()) + self.frame_overhead();
        if let Some(window) = self.window {
            if frame_bytes > window.max_bytes {
                return Err(TerminalInputError::InputFull);
            }
        }
        if self.buffered_bytes + frame_bytes > self.limits.max_buffered_bytes {
            return Err(TerminalInputError::InputFull);
        }
        self.sequence += 1;
        self.input_bytes += bytes;
        self.buffered_bytes += frame_bytes;
        self.queued.push_back(QueuedInput {
            command,
            frame_bytes,
        });
        self.flush();
        Ok(())
    }

    fn frame_overhead(&self) -> usize {
        self.window.map_or(0, |window| window.frame_overhead_bytes)
    }

    fn flush(&mut self) {
        while let Some(next) = self.queued.front() {
            if self.closed {
                return;
            }
            if let Some(window) = self.window {
                let in_flight = self.in_flight.values().sum::<usize>();
                if in_flight + next.frame_bytes > window.max_bytes {
                    return;
                }
            }
            let next = self.queued.pop_front().expect("front exists");
            let sequence = match &next.command {
                TerminalInput::Write { sequence, .. } | TerminalInput::Resize { sequence, .. } => {
                    *sequence
                }
            };
            if self.sender.send(next.command).is_err() {
                self.closed = true;
                self.queued.clear();
                return;
            }
            self.in_flight.insert(sequence, next.frame_bytes);
        }
    }
}

/// Opens an authenticated terminal attachment and its input uplink.
pub fn attach(
    mux: &RemoteStreamMux,
    agent_id: impl Into<String>,
    id: impl Into<String>,
) -> Result<(TerminalStream<TerminalFrame>, mpsc::Sender<TerminalInput>), Error> {
    let stream = mux.open(
        "terminal/attach",
        serde_json::json!({"agentId": agent_id.into(), "id": id.into()}),
    )?;
    let (socket_uplink, receiver) = (stream.uplink, stream.downlink);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = TerminalCancel {
        cancelled: Arc::clone(&cancelled),
        uplink: socket_uplink.clone(),
    };
    let mapped_rx = decode_stream(receiver, &cancel);
    let (uplink_tx, uplink_rx) = mpsc::channel::<TerminalInput>();
    thread::spawn(move || {
        while !cancelled.load(Ordering::Acquire) {
            match uplink_rx.recv_timeout(std::time::Duration::from_millis(40)) {
                Ok(input) => match serde_json::to_value(input) {
                    Ok(value) => socket_uplink.item(value),
                    Err(_) => break,
                },
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    socket_uplink.end();
                    break;
                }
            }
        }
    });
    Ok((
        TerminalStream {
            receiver: mapped_rx,
            cancel,
        },
        uplink_tx,
    ))
}

/// Opens an authenticated retained-terminal stream.
pub fn retain(
    mux: &RemoteStreamMux,
    session_id: impl Into<String>,
    id: impl Into<String>,
) -> Result<TerminalStream<TerminalRetainedFrame>, Error> {
    let stream = mux.open(
        "terminal/retain",
        serde_json::json!({"sessionId": session_id.into(), "id": id.into()}),
    )?;
    let (uplink, receiver) = (stream.uplink, stream.downlink);
    let cancel = TerminalCancel {
        cancelled: Arc::new(AtomicBool::new(false)),
        uplink,
    };
    let mapped_rx = decode_stream(receiver, &cancel);
    Ok(TerminalStream {
        receiver: mapped_rx,
        cancel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_stream::tests::{read, send, server};
    use std::time::Duration;

    #[test]
    fn authenticated_attach_sends_agent_and_single_inputs_then_half_closes() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            assert_eq!(open["endpoint"], "terminal/attach");
            assert_eq!(
                open["payload"],
                serde_json::json!({"args":{"agentId":"a","id":"t"}})
            );
            let id = &open["streamId"];
            assert_eq!(
                read(&mut socket),
                serde_json::json!({"type":"item","streamId":id,"value":{"type":"write","sequence":1,"data":"x"}})
            );
            assert_eq!(
                read(&mut socket),
                serde_json::json!({"type":"item","streamId":id,"value":{"type":"resize","sequence":2,"cols":90,"rows":30}})
            );
            assert_eq!(
                read(&mut socket),
                serde_json::json!({"type":"end","streamId":id})
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":id,"value":{"type":"ack","sequence":2,"refusal":"not-running"}}),
            );
            send(&mut socket, serde_json::json!({"type":"end","streamId":id}));
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let (mut stream, uplink) = attach(&mux, "a", "t").unwrap();
        uplink
            .send(TerminalInput::Write {
                sequence: 1,
                data: "x".into(),
            })
            .unwrap();
        uplink
            .send(TerminalInput::Resize {
                sequence: 2,
                cols: 90,
                rows: 30,
            })
            .unwrap();
        drop(uplink);
        assert_eq!(
            stream.next().unwrap().unwrap(),
            TerminalFrame::Ack(TerminalAckFrame {
                sequence: 2,
                refusal: Some(TerminalRefusal::NotRunning)
            })
        );
        assert!(stream.next().is_none());
        worker.join().unwrap();
    }

    #[test]
    fn attach_cancel_and_drop_release_idle_socket_even_with_live_input_sender() {
        for explicit in [true, false] {
            let (url, credential, worker) = server(|mut socket| {
                let open = read(&mut socket);
                assert_eq!(
                    read(&mut socket),
                    serde_json::json!({"type":"cancel","streamId":open["streamId"]})
                );
            });
            let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
            let (stream, uplink) = attach(&mux, "a", "t").unwrap();
            if explicit {
                stream.cancel_handle().cancel();
                assert!(matches!(
                    stream.receiver.recv_timeout(Duration::from_secs(3)),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                ));
            }
            drop(stream);
            worker.join().unwrap();
            drop(uplink);
        }
    }

    #[test]
    fn retain_uses_session_id_and_releases_idle_hold_on_drop() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            assert_eq!(open["endpoint"], "terminal/retain");
            assert_eq!(
                open["payload"],
                serde_json::json!({"args":{"sessionId":"s","id":"t"}})
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"type":"retained"}}),
            );
            assert_eq!(
                read(&mut socket),
                serde_json::json!({"type":"cancel","streamId":open["streamId"]})
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream = retain(&mux, "s", "t").unwrap();
        assert_eq!(
            stream.next().unwrap().unwrap(),
            TerminalRetainedFrame::Retained
        );
        drop(stream);
        worker.join().unwrap();
        assert_eq!(
            serde_json::to_value(TerminalRetainRequest {
                session_id: "s".into(),
                id: "t".into()
            })
            .unwrap(),
            serde_json::json!({"sessionId":"s","id":"t"})
        );
    }
    #[test]
    fn serializes_requests() {
        assert_eq!(
            serde_json::to_value(TerminalCreateRequest {
                shell_path: Some("/bin/bash".into()),
                id: "t".into(),
                cols: 80,
                rows: 24
            })
            .unwrap(),
            serde_json::json!({"shellPath":"/bin/bash","id":"t","cols":80,"rows":24})
        );
        assert_eq!(
            serde_json::to_value(TerminalRenameRequest {
                id: "t".into(),
                title: "x".into()
            })
            .unwrap(),
            serde_json::json!({"id":"t","title":"x"})
        );
        assert_eq!(
            serde_json::to_value(TerminalCloseRequest { id: "t".into() }).unwrap(),
            serde_json::json!({"id":"t"})
        );
    }
    #[test]
    fn deserializes_attach_frames() {
        let snapshot = r#"{"type":"snapshot","sequence":0,"screen":"$ ","info":{"id":"t","title":"t","shell":{"path":"/bin/sh","args":[],"name":"sh"},"cwd":"/","cols":80,"rows":24,"state":"running","exitCode":null},"writable":true}"#;
        assert!(matches!(
            serde_json::from_str::<TerminalFrame>(snapshot).unwrap(),
            TerminalFrame::Snapshot(_)
        ));
        assert!(matches!(
            serde_json::from_str::<TerminalFrame>(r#"{"type":"output","sequence":1,"data":"ok"}"#)
                .unwrap(),
            TerminalFrame::Output(_)
        ));
        assert!(matches!(serde_json::from_str::<TerminalFrame>(r#"{"type":"state","info":{"id":"t","title":"t","shell":{"path":"/bin/sh","args":[],"name":"sh"},"cwd":"/","cols":80,"rows":24,"state":"exited","exitCode":0},"writable":false}"#).unwrap(), TerminalFrame::State(_)));
        assert!(matches!(
            serde_json::from_str::<TerminalFrame>(
                r#"{"type":"ack","sequence":1,"refusal":"read-only"}"#
            )
            .unwrap(),
            TerminalFrame::Ack(_)
        ));
    }

    fn limits() -> TerminalInputLimits {
        TerminalInputLimits {
            max_input_bytes: 64,
            max_buffered_bytes: 4096,
            max_cols: 120,
            max_rows: 40,
        }
    }

    #[test]
    fn writer_queues_until_window_capacity_and_ack_releases_it() {
        let (sender, receiver) = mpsc::channel();
        let mut writer = TerminalWriter::new(sender, limits());
        // One encoded frame fits; the second waits for the acknowledgement.
        let frame = serde_json::to_vec(&TerminalInput::Write {
            sequence: 1,
            data: "aa".into(),
        })
        .unwrap()
        .len();
        writer.set_window(RemoteUplinkWindow {
            max_bytes: frame + 5,
            frame_overhead_bytes: 0,
        });
        assert_eq!(writer.write("aa").unwrap(), 1);
        assert_eq!(writer.pending(), Some(1));
        assert_eq!(writer.write("bb").unwrap(), 2);
        // The second frame waits for the first acknowledgement.
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
        writer.acknowledge(1);
        assert!(receiver.try_recv().is_ok());
        assert_eq!(writer.pending(), Some(2));
    }

    #[test]
    fn writer_rejects_input_beyond_the_session_budget() {
        let (sender, _receiver) = mpsc::channel();
        let mut writer = TerminalWriter::new(sender, limits());
        assert_eq!(writer.write(&"x".repeat(64)).unwrap(), 1);
        assert_eq!(
            writer.write("x"),
            Err(TerminalInputError::InputFull)
        );
    }

    #[test]
    fn writer_rejects_a_frame_larger_than_the_window() {
        let (sender, _receiver) = mpsc::channel();
        let mut writer = TerminalWriter::new(sender, limits());
        writer.set_window(RemoteUplinkWindow {
            max_bytes: 10,
            frame_overhead_bytes: 0,
        });
        assert_eq!(
            writer.write("payload"),
            Err(TerminalInputError::InputFull)
        );
    }

    #[test]
    fn writer_clamps_resize_and_orders_sequences() {
        let (sender, receiver) = mpsc::channel();
        let mut writer = TerminalWriter::new(sender, limits());
        assert_eq!(writer.resize(500, 500).unwrap(), 1);
        assert_eq!(writer.write("hi").unwrap(), 2);
        assert_eq!(
            receiver.recv().unwrap(),
            TerminalInput::Resize {
                sequence: 1,
                cols: 120,
                rows: 40
            }
        );
        assert_eq!(
            receiver.recv().unwrap(),
            TerminalInput::Write {
                sequence: 2,
                data: "hi".into()
            }
        );
        writer.close();
        assert_eq!(writer.write("x"), Err(TerminalInputError::Closed));
    }
}
