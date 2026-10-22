//! Typed terminal HTTP operations and authenticated terminal WebSocket streams.

use crate::{auth::Credential, transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tungstenite::Message;
use url::Url;

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
pub struct TerminalListRequest {
    pub session_id: String,
}

/// Request for terminal retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalRetainRequest {
    pub session_id: String,
    pub id: String,
}

/// Native terminal HTTP client.
pub struct TerminalClient<'a> {
    transport: &'a HttpTransport,
}
impl<'a> TerminalClient<'a> {
    /// Creates a terminal client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }
    fn request<T: for<'de> Deserialize<'de>, R: Serialize>(
        &self,
        method: &str,
        request: R,
    ) -> Result<T, Error> {
        let cancel = AtomicBool::new(false);
        let value = self
            .transport
            .request(method, serde_json::to_value(request)?, &cancel)?;
        Ok(serde_json::from_value(value)?)
    }
    /// Returns terminal environment limits.
    pub fn environment(&self) -> Result<TerminalEnvironment, Error> {
        self.request("terminal/environment", Value::Null)
    }
    /// Returns executable shells.
    pub fn shells(&self) -> Result<Vec<TerminalShell>, Error> {
        self.request("terminal/shells", Value::Null)
    }
    /// Lists terminals for a session.
    pub fn list(&self, session_id: impl Into<String>) -> Result<Vec<WebTerminalInfo>, Error> {
        self.request(
            "terminal/list",
            TerminalListRequest {
                session_id: session_id.into(),
            },
        )
    }
    /// Creates a terminal.
    pub fn create(&self, request: TerminalCreateRequest) -> Result<WebTerminalInfo, Error> {
        self.request("terminal/create", request)
    }
    /// Renames a terminal.
    pub fn rename(&self, request: TerminalRenameRequest) -> Result<(), Error> {
        self.request::<Value, _>("terminal/rename", request)
            .map(|_| ())
    }
    /// Closes a terminal.
    pub fn close(&self, request: TerminalCloseRequest) -> Result<(), Error> {
        self.request::<Value, _>("terminal/close", request)
            .map(|_| ())
    }
}

/// Stops a terminal stream.
pub struct TerminalCancel {
    cancelled: Arc<AtomicBool>,
    socket: Arc<
        Mutex<
            Option<
                tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
            >,
        >,
    >,
}
impl TerminalCancel {
    /// Requests stream shutdown.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut socket) = self.socket.lock() {
            if let Some(socket) = socket.as_mut() {
                let _ = socket.close(None);
            }
        }
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

fn websocket(
    base_url: &str,
    credential: &Credential,
    path: &str,
    payload: String,
) -> Result<(TerminalCancel, mpsc::Receiver<Result<String, Error>>), Error> {
    let mut url = Url::parse(base_url).map_err(|source| Error::InvalidUrl { source })?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => return Err(Error::InvalidCredential),
    };
    url.set_scheme(scheme)
        .map_err(|_| Error::InvalidCredential)?;
    url.set_path(&format!(
        "{}{}",
        url.path().trim_end_matches('/').to_owned() + "/api/",
        path
    ));
    let request = tungstenite::http::Request::builder()
        .uri(url.as_str())
        .header("Cookie", credential.cookie.as_str())
        .body(())
        .map_err(|_| Error::InvalidCredential)?;
    let (socket, _) =
        tungstenite::client::connect(request).map_err(|_| Error::InvalidCredential)?;
    let socket = Arc::new(Mutex::new(Some(socket)));
    if let Ok(mut guard) = socket.lock() {
        if let Some(socket) = guard.as_mut() {
            socket
                .write(Message::Text(payload.into()))
                .map_err(|_| Error::InvalidCredential)?;
        }
    }
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_socket = Arc::clone(&socket);
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        loop {
            if worker_cancelled.load(Ordering::Acquire) {
                break;
            }
            let result = worker_socket
                .lock()
                .ok()
                .and_then(|mut guard| guard.as_mut().map(|socket| socket.read()));
            match result {
                Some(Ok(Message::Text(text))) => {
                    if sender.send(Ok(text.to_string())).is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            }
        }
        let _ = worker_socket
            .lock()
            .ok()
            .and_then(|mut guard| guard.as_mut().map(|socket| socket.close(None)));
    });
    Ok((TerminalCancel { cancelled, socket }, receiver))
}

/// Opens an authenticated terminal attachment and its input uplink.
pub fn attach(
    base_url: &str,
    credential: &Credential,
    id: impl Into<String>,
) -> Result<
    (
        TerminalStream<TerminalFrame>,
        mpsc::Sender<Vec<TerminalInput>>,
    ),
    Error,
> {
    let (cancel, receiver) = websocket(
        base_url,
        credential,
        "terminal/attach",
        serde_json::json!({"id": id.into()}).to_string(),
    )?;
    let (uplink_tx, uplink_rx) = mpsc::channel::<Vec<TerminalInput>>();
    let socket = Arc::clone(&cancel.socket);
    let cancelled = Arc::clone(&cancel.cancelled);
    thread::spawn(move || {
        while !cancelled.load(Ordering::Acquire) {
            match uplink_rx.recv() {
                Ok(inputs) => {
                    let text = match serde_json::to_string(&inputs) {
                        Ok(text) => text,
                        Err(_) => break,
                    };
                    if let Ok(mut guard) = socket.lock() {
                        if let Some(socket) = guard.as_mut() {
                            if socket.write(Message::Text(text.into())).is_err() {
                                break;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    let (mapped_tx, mapped_rx) = mpsc::channel();
    thread::spawn(move || {
        for item in receiver {
            let result = item.and_then(|text| serde_json::from_str(&text).map_err(Error::from));
            if mapped_tx.send(result).is_err() {
                break;
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
    let payload = serde_json::json!({"sessionId": session_id.into(), "id": id.into()}).to_string();
    let (cancel, receiver) = websocket(base_url, credential, "terminal/retain", payload)?;
    let (mapped_tx, mapped_rx) = mpsc::channel();
    thread::spawn(move || {
        for item in receiver {
            let result = item.and_then(|text| serde_json::from_str(&text).map_err(Error::from));
            if mapped_tx.send(result).is_err() {
                break;
            }
        }
    });
    Ok(TerminalStream {
        receiver: mapped_rx,
        cancel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
