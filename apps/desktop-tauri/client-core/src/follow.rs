//! Authenticated `session/follow` WebSocket frames for native chat clients.
//!
//! The stream sends the same snapshot, durable event, and optional assistant
//! stream records as the browser client. A cancel handle closes the socket so
//! blocked iteration ends without leaving a live Host subscription.

use crate::{auth::Credential, session::SessionAddress, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use tungstenite::Message;
use url::Url;

/// Request options for `session/follow`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowRequest {
    pub address: SessionAddress,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_messages: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_window: Option<TurnWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_stream: Option<bool>,
}

/// Minimum history window requested from the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnWindow {
    pub min_messages: u32,
    pub min_turns: u32,
}

/// Header identifying the followed session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWireHeader {
    pub version: u32,
    pub id: String,
    pub created_at: i64,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub parent_session: Option<String>,
    pub is_seeded: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub delegation_depth: Option<u32>,
    #[serde(default)]
    pub agent_preset: Option<String>,
}

/// Wire durable event, retaining extensible event data without reinterpretation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWireEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub seq: i64,
    pub time: i64,
    pub data: Value,
    #[serde(default)]
    pub ignorable: Option<bool>,
    #[serde(default)]
    pub source_event_seqs: Option<Value>,
    #[serde(default)]
    pub surface_op: Option<Value>,
}

/// A durable event frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventFrame {
    #[serde(rename = "type", default)]
    pub frame_type: String,
    pub event: SessionWireEvent,
}

/// Baseline assistant attempt in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStreamAttempt {
    pub attempt_id: String,
    pub started_after_seq: i64,
    pub turn: u32,
    pub step: u32,
    pub next_index: u32,
    pub stream: Vec<Value>,
}

/// Assistant-stream snapshot baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantStreamBaseline {
    pub revision: u64,
    #[serde(default)]
    pub active_attempt: Option<AssistantStreamAttempt>,
}

/// Snapshot frame sent first by `session/follow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFrame {
    #[serde(rename = "type", default)]
    pub frame_type: String,
    pub header: SessionWireHeader,
    pub cursor: i64,
    pub records: Vec<EventFrame>,
    pub has_more: bool,
    pub projections: Value,
    #[serde(default)]
    pub assistant_stream: Option<AssistantStreamBaseline>,
}

/// A live assistant-stream frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AssistantStreamFrame {
    #[serde(rename = "start")]
    Start {
        #[serde(rename = "attemptId")]
        attempt_id: String,
        revision: u64,
        #[serde(rename = "startedAfterSeq")]
        started_after_seq: i64,
        turn: u32,
        step: u32,
    },
    #[serde(rename = "chunk")]
    Chunk {
        #[serde(rename = "attemptId")]
        attempt_id: String,
        revision: u64,
        index: u32,
        time: i64,
        chunk: Value,
    },
    #[serde(rename = "end")]
    End {
        #[serde(rename = "attemptId")]
        attempt_id: String,
        revision: u64,
        index: u32,
        outcome: AssistantStreamOutcome,
    },
}

/// Terminal outcome for an assistant stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AssistantStreamOutcome {
    #[serde(rename = "committed")]
    Committed {
        #[serde(rename = "eventType")]
        event_type: String,
        seq: i64,
    },
    #[serde(rename = "abandoned")]
    Abandoned,
}

/// One decoded follow frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FollowFrame {
    #[serde(rename = "snapshot")]
    Snapshot(SnapshotFrame),
    #[serde(rename = "event")]
    Event(EventFrame),
    #[serde(rename = "assistant-stream")]
    AssistantStream { frame: AssistantStreamFrame },
}

/// Stops a follow stream and closes its WebSocket.
pub struct FollowCancel {
    cancelled: Arc<AtomicBool>,
}
impl FollowCancel {
    /// Requests stream shutdown.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// A receiving end for decoded follow frames and its cancellation handle.
pub struct FollowStream {
    receiver: mpsc::Receiver<Result<FollowFrame, Error>>,
    cancel: FollowCancel,
}
impl FollowStream {
    /// Returns the cancellation handle.
    pub fn cancel_handle(&self) -> &FollowCancel {
        &self.cancel
    }
}
impl Iterator for FollowStream {
    type Item = Result<FollowFrame, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

/// Opens an authenticated `session/follow` WebSocket using the supplied cookie.
pub fn follow(
    base_url: &str,
    credential: &Credential,
    request: FollowRequest,
) -> Result<FollowStream, Error> {
    let mut url = Url::parse(base_url).map_err(|source| Error::InvalidUrl { source })?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => {
            return Err(Error::InvalidUrl {
                source: url::ParseError::RelativeUrlWithoutBase,
            })
        }
    };
    url.set_scheme(scheme)
        .map_err(|_| Error::InvalidCredential)?;
    let path = format!(
        "{}session/follow",
        url.path().trim_end_matches('/').to_owned() + "/api/"
    );
    url.set_path(&path);
    let request_text = serde_json::to_string(&request)?;
    let request = tungstenite::http::Request::builder()
        .uri(url.as_str())
        .header("Cookie", credential.cookie.as_str())
        .body(())
        .map_err(|_| Error::InvalidCredential)?;
    let (mut socket, _) =
        tungstenite::client::connect(request).map_err(|_| Error::InvalidCredential)?;
    socket
        .write(Message::Text(request_text.into()))
        .map_err(|_| Error::InvalidCredential)?;
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        while !worker_cancelled.load(Ordering::Acquire) {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let result = serde_json::from_str::<FollowFrame>(&text);
                    if sender.send(result.map_err(Error::from)).is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = socket.close(None);
    });
    Ok(FollowStream {
        receiver,
        cancel: FollowCancel { cancelled },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deserializes_snapshot() {
        let frame: FollowFrame = serde_json::from_str(r#"{"type":"snapshot","header":{"version":4,"id":"s","createdAt":1,"isSeeded":false},"cursor":2,"records":[],"hasMore":false,"projections":{"asOfSeq":2,"values":{}}}"#).unwrap();
        assert!(matches!(frame, FollowFrame::Snapshot(_)));
    }
    #[test]
    fn deserializes_event() {
        let frame: FollowFrame = serde_json::from_str(r#"{"type":"event","event":{"type":"turn/start","seq":3,"time":4,"data":{"turn":1}}}"#).unwrap();
        assert!(matches!(frame, FollowFrame::Event(_)));
    }
    #[test]
    fn deserializes_assistant_stream() {
        let frame: FollowFrame = serde_json::from_str(r#"{"type":"assistant-stream","frame":{"type":"chunk","attemptId":"a","revision":1,"index":0,"time":2,"chunk":{"text":"hi"}}}"#).unwrap();
        assert!(matches!(frame, FollowFrame::AssistantStream { .. }));
    }
}
