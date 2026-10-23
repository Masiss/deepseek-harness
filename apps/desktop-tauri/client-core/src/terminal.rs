//! Typed terminal HTTP operations and authenticated terminal WebSocket streams.

use crate::{
    auth::Credential,
    remote_stream::{self, ClientFrame},
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
    pub input_window: Option<Value>,
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
    uplink: mpsc::Sender<ClientFrame>,
    stream_id: String,
}
impl TerminalCancel {
    /// Requests stream shutdown without waiting for the socket worker to exit.
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.uplink.send(ClientFrame::Cancel {
            stream_id: self.stream_id.clone(),
        });
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
    let stream_id = cancel.stream_id.clone();
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
        let _ = uplink.send(ClientFrame::Cancel { stream_id });
    });
    mapped_rx
}

/// Opens an authenticated terminal attachment and its input uplink.
pub fn attach(
    base_url: &str,
    credential: &Credential,
    agent_id: impl Into<String>,
    id: impl Into<String>,
) -> Result<(TerminalStream<TerminalFrame>, mpsc::Sender<TerminalInput>), Error> {
    let (stream_id, socket_uplink, receiver) = remote_stream::open(
        base_url,
        credential,
        "terminal/attach",
        serde_json::json!({"agentId": agent_id.into(), "id": id.into()}),
    )?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = TerminalCancel {
        cancelled: Arc::clone(&cancelled),
        uplink: socket_uplink.clone(),
        stream_id: stream_id.clone(),
    };
    let mapped_rx = decode_stream(receiver, &cancel);
    let (uplink_tx, uplink_rx) = mpsc::channel::<TerminalInput>();
    thread::spawn(move || {
        while !cancelled.load(Ordering::Acquire) {
            match uplink_rx.recv_timeout(std::time::Duration::from_millis(40)) {
                Ok(input) => {
                    let value = match serde_json::to_value(input) {
                        Ok(value) => value,
                        Err(_) => break,
                    };
                    if socket_uplink
                        .send(ClientFrame::Item {
                            stream_id: stream_id.clone(),
                            value,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = socket_uplink.send(ClientFrame::End {
                        stream_id: stream_id.clone(),
                    });
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
    base_url: &str,
    credential: &Credential,
    session_id: impl Into<String>,
    id: impl Into<String>,
) -> Result<TerminalStream<TerminalRetainedFrame>, Error> {
    let (stream_id, uplink, receiver) = remote_stream::open(
        base_url,
        credential,
        "terminal/retain",
        serde_json::json!({"sessionId": session_id.into(), "id": id.into()}),
    )?;
    let cancel = TerminalCancel {
        cancelled: Arc::new(AtomicBool::new(false)),
        uplink,
        stream_id,
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
        let (mut stream, uplink) = attach(&url, &credential, "a", "t").unwrap();
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
            let (stream, uplink) = attach(&url, &credential, "a", "t").unwrap();
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
        let mut stream = retain(&url, &credential, "s", "t").unwrap();
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
}
