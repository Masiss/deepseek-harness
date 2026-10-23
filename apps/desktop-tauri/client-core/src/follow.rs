//! Authenticated `session/follow` WebSocket frames for native chat clients.
//!
//! The stream sends the same snapshot, durable event, and optional assistant
//! stream records as the browser client. Cancellation and stream drop request
//! socket shutdown, waking blocked iteration without awaiting worker completion.

use crate::{
    remote_stream::{RemoteStreamMux, Uplink},
    session::SessionAddress,
    Error,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

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
    #[serde(rename = "type", default, skip_serializing_if = "String::is_empty")]
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
#[serde(rename_all = "camelCase")]
pub struct AssistantStreamBaseline {
    pub revision: u64,
    #[serde(default)]
    pub active_attempt: Option<AssistantStreamAttempt>,
}

/// Snapshot frame sent first by `session/follow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFrame {
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
    uplink: Uplink,
}
impl FollowCancel {
    /// Requests stream shutdown without waiting for the socket worker to exit.
    pub fn cancel(&self) {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        self.uplink.cancel();
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
impl Drop for FollowStream {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Opens an authenticated `session/follow` WebSocket using the supplied cookie.
pub fn follow(mux: &RemoteStreamMux, request: FollowRequest) -> Result<FollowStream, Error> {
    let stream = mux.open(
        "session/follow",
        serde_json::json!({"request": serde_json::to_value(request)?}),
    )?;
    let (uplink, mux_receiver) = (stream.uplink, stream.downlink);
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_uplink = uplink.clone();
    std::thread::spawn(move || {
        while !worker_cancelled.load(Ordering::Acquire) {
            match mux_receiver.recv() {
                Ok(Ok(value)) => {
                    let result = serde_json::from_value::<FollowFrame>(value).map_err(Error::from);
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        break;
                    }
                }
                Ok(Err(error)) => {
                    let _ = sender.send(Err(error));
                    break;
                }
                Err(_) => break,
            }
        }
        worker_cancelled.store(true, Ordering::Release);
        worker_uplink.cancel();
    });
    Ok(FollowStream {
        receiver,
        cancel: FollowCancel {
            cancelled,
            uplink,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_stream::tests::{read, send, server};
    use std::time::Duration;

    fn request() -> FollowRequest {
        FollowRequest {
            address: SessionAddress {
                kind: crate::session::SessionAddressKind::Session,
                session_id: "s".into(),
            },
            max_messages: Some(20),
            turn_window: Some(TurnWindow {
                min_messages: 2,
                min_turns: 1,
            }),
            assistant_stream: Some(true),
        }
    }

    #[test]
    fn authenticated_follow_sends_named_request_and_decodes_exact_snapshot_fields() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            assert_eq!(open["endpoint"], "session/follow");
            assert_eq!(
                open["payload"],
                serde_json::json!({"args":{"request":{
                    "address":{"kind":"session","sessionId":"s"},"maxMessages":20,
                    "turnWindow":{"minMessages":2,"minTurns":1},"assistantStream":true
                }}})
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{
                    "type":"snapshot","header":{"version":4,"id":"s","createdAt":1,"isSeeded":true,"parentSession":"parent","agentPreset":"preset"},
                    "cursor":7,"hasMore":true,"projections":{"asOfSeq":7,"values":{}},
                    "records":[{"type":"event","event":{"type":"user/message","seq":7,"time":10,"data":{"text":"hello"},"surfaceOp":"append"}}],
                    "assistantStream":{"revision":3,"activeAttempt":{"attemptId":"a","startedAfterSeq":7,"turn":2,"step":1,"nextIndex":1,"stream":[{"text":"hi"}]}}
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream = follow(&mux, request()).unwrap();
        let frame = stream.next().unwrap().unwrap();
        let FollowFrame::Snapshot(snapshot) = &frame else {
            panic!("expected snapshot")
        };
        assert_eq!(snapshot.cursor, 7);
        assert!(snapshot.has_more);
        assert_eq!(snapshot.header.parent_session.as_deref(), Some("parent"));
        assert_eq!(snapshot.header.agent_preset.as_deref(), Some("preset"));
        assert_eq!(snapshot.records[0].frame_type, "event");
        assert_eq!(
            snapshot.records[0].event.surface_op,
            Some(serde_json::json!("append"))
        );
        let attempt = snapshot
            .assistant_stream
            .as_ref()
            .unwrap()
            .active_attempt
            .as_ref()
            .unwrap();
        assert_eq!(attempt.attempt_id, "a");
        assert_eq!(attempt.next_index, 1);
        assert_eq!(attempt.stream, vec![serde_json::json!({"text":"hi"})]);
        let encoded = serde_json::to_string(&frame).unwrap();
        assert!(encoded.starts_with("{\"type\":\"snapshot\",\"header\":"));
        assert!(encoded.contains("\"activeAttempt\":"));
        assert_eq!(
            serde_json::from_str::<FollowFrame>(&encoded).unwrap(),
            frame
        );
        assert!(stream.next().is_none());
        worker.join().unwrap();
    }

    #[test]
    fn cancel_and_drop_release_idle_follow_subscriptions() {
        for explicit in [true, false] {
            let (url, credential, worker) = server(|mut socket| {
                let open = read(&mut socket);
                assert_eq!(
                    read(&mut socket),
                    serde_json::json!({"type":"cancel","streamId":open["streamId"]})
                );
            });
            let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
            let stream = follow(&mux, request()).unwrap();
            if explicit {
                stream.cancel_handle().cancel();
                assert!(matches!(
                    stream.receiver.recv_timeout(Duration::from_secs(3)),
                    Err(mpsc::RecvTimeoutError::Disconnected)
                ));
            }
            drop(stream);
            worker.join().unwrap();
        }
    }
    #[test]
    fn deserializes_snapshot() {
        let frame: FollowFrame = serde_json::from_str(r#"{"type":"snapshot","header":{"version":4,"id":"s","createdAt":1,"isSeeded":false},"cursor":2,"records":[],"hasMore":false,"projections":{"asOfSeq":2,"values":{}}}"#).unwrap();
        assert!(matches!(frame, FollowFrame::Snapshot(_)));
    }
    #[test]
    fn deserializes_event() {
        let frame: FollowFrame = serde_json::from_str(
            r#"{"type":"event","event":{"type":"turn/start","seq":3,"time":4,"data":{"turn":1}}}"#,
        )
        .unwrap();
        assert!(matches!(frame, FollowFrame::Event(_)));
    }
    #[test]
    fn deserializes_assistant_stream() {
        let frame: FollowFrame = serde_json::from_str(r#"{"type":"assistant-stream","frame":{"type":"chunk","attemptId":"a","revision":1,"index":0,"time":2,"chunk":{"text":"hi"}}}"#).unwrap();
        assert!(matches!(frame, FollowFrame::AssistantStream { .. }));
    }
}
