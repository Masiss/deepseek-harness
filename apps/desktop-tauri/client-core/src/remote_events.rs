//! Authenticated `$events` delivery and generation-correlated waterfall results.
//!
//! A reader processes Host cancellations independently of UI consumption. Submissions are
//! one-shot: an HTTP failure can leave the Host outcome unknown, so callers must not retry.
//! Dropping the client cancels the mux subscription and withdraws pending presentations.

use crate::{
    remote_stream::{RemoteStreamMux, Uplink},
    transport::HttpTransport,
    Error,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

/// Event-stream generation identity, distinct from a waterfall invocation identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteEventClientId(String);

/// Host-minted waterfall correlation identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteEventId(String);

/// Agent owning a scoped waterfall invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RemoteEventAgentId(String);

impl RemoteEventClientId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl RemoteEventId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl RemoteEventAgentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable Host facts attached to a ready generation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteEventHostInfo {
    pub home: String,
}

/// JSON-safe fields of a listener failure, not a user's approval denial.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteEventRejection {
    pub name: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Waterfall delegation, a returned value, or a thrown listener failure.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RemoteEventOutcome<T> {
    Next,
    Result { value: T },
    Rejected { error: RemoteEventRejection },
}

/// Exact `$events/result` payload; the HTTP transport supplies its standard RPC envelope.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteEventResult<'a, T> {
    client_id: &'a RemoteEventClientId,
    event_id: &'a RemoteEventId,
    outcome: RemoteEventOutcome<T>,
}

/// One scoped presentation; cancellation is local state and never serialized.
#[derive(Debug, Clone)]
pub struct RemoteEventInvocation {
    pub client_id: RemoteEventClientId,
    pub event_id: RemoteEventId,
    pub agent_id: RemoteEventAgentId,
    pub event: String,
    pub request: Map<String, Value>,
    cancelled: Arc<AtomicBool>,
}

impl RemoteEventInvocation {
    /// True after Host cancellation, stream closure, or client disposal.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Delivered events; callers may delegate unsupported waterfall events using `Next`.
#[derive(Debug, Clone)]
pub enum RemoteEvent {
    Ready {
        client_id: RemoteEventClientId,
        host: RemoteEventHostInfo,
    },
    Emit {
        event: String,
        args: Vec<Value>,
    },
    Waterfall(RemoteEventInvocation),
    Cancel {
        event_id: RemoteEventId,
    },
}

/// Remote Event protocol, lifecycle, or transport failure.
#[derive(Debug, thiserror::Error)]
pub enum RemoteEventError {
    #[error(transparent)]
    Transport(#[from] Error),
    #[error("invalid Remote Event: {0}")]
    Protocol(&'static str),
    #[error("Remote Event stream is closed")]
    Closed,
    #[error("Remote Event belongs to another client generation")]
    GenerationMismatch,
    #[error("Remote Event was cancelled, answered, or is unknown")]
    NotPending,
    #[error("Remote Event state lock poisoned")]
    StatePoisoned,
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum Downlink {
    Ready {
        client_id: RemoteEventClientId,
        host: RemoteEventHostInfo,
    },
    Emit {
        event: String,
        args: Vec<Value>,
    },
    Waterfall {
        event: String,
        event_id: RemoteEventId,
        agent_id: RemoteEventAgentId,
        request: Map<String, Value>,
    },
    Cancel {
        event_id: RemoteEventId,
    },
}

#[derive(Default)]
struct State {
    client_id: Option<RemoteEventClientId>,
    closed: bool,
    pending: HashMap<RemoteEventId, Pending>,
}

struct Pending {
    cancelled: Arc<AtomicBool>,
    submitted: bool,
}

impl State {
    fn close(&mut self) {
        self.closed = true;
        for (_, pending) in self.pending.drain() {
            pending.cancelled.store(true, Ordering::Release);
        }
    }

    fn receive(&mut self, frame: Downlink) -> Result<RemoteEvent, RemoteEventError> {
        if self.closed {
            return Err(RemoteEventError::Closed);
        }
        if let Downlink::Ready { client_id, host } = frame {
            if self.client_id.is_some() || client_id.0.is_empty() {
                return Err(RemoteEventError::Protocol("invalid or repeated ready"));
            }
            self.client_id = Some(client_id.clone());
            return Ok(RemoteEvent::Ready { client_id, host });
        }
        let client_id = self
            .client_id
            .clone()
            .ok_or(RemoteEventError::Protocol("frame before ready"))?;
        match frame {
            Downlink::Emit { event, args } => Ok(RemoteEvent::Emit { event, args }),
            Downlink::Waterfall {
                event,
                event_id,
                agent_id,
                request,
            } => {
                if event_id.0.is_empty()
                    || agent_id.0.is_empty()
                    || event.is_empty()
                    || self.pending.contains_key(&event_id)
                {
                    return Err(RemoteEventError::Protocol(
                        "invalid or duplicate invocation",
                    ));
                }
                if request.contains_key("signal") || request.contains_key("agent") {
                    return Err(RemoteEventError::Protocol("unprojected request"));
                }
                let cancelled = Arc::new(AtomicBool::new(false));
                self.pending.insert(
                    event_id.clone(),
                    Pending {
                        cancelled: cancelled.clone(),
                        submitted: false,
                    },
                );
                Ok(RemoteEvent::Waterfall(RemoteEventInvocation {
                    client_id,
                    event_id,
                    agent_id,
                    event,
                    request,
                    cancelled,
                }))
            }
            Downlink::Cancel { event_id } => {
                if event_id.0.is_empty() {
                    return Err(RemoteEventError::Protocol("empty cancellation id"));
                }
                // Cancellation may arrive after a result was submitted.
                if let Some(pending) = self.pending.remove(&event_id) {
                    pending.cancelled.store(true, Ordering::Release);
                }
                Ok(RemoteEvent::Cancel { event_id })
            }
            Downlink::Ready { .. } => Err(RemoteEventError::Protocol("repeated ready")),
        }
    }

    fn reserve(&mut self, invocation: &RemoteEventInvocation) -> Result<(), RemoteEventError> {
        if self.closed {
            return Err(RemoteEventError::Closed);
        }
        if self.client_id.as_ref() != Some(&invocation.client_id) {
            return Err(RemoteEventError::GenerationMismatch);
        }
        let pending = self
            .pending
            .get_mut(&invocation.event_id)
            .ok_or(RemoteEventError::NotPending)?;
        if !Arc::ptr_eq(&pending.cancelled, &invocation.cancelled)
            || invocation.is_cancelled()
            || pending.submitted
        {
            return Err(RemoteEventError::NotPending);
        }
        pending.submitted = true;
        Ok(())
    }
}

/// One authenticated Remote Event subscription. Polling UI events is independent of cancellation processing.
pub struct RemoteEvents {
    transport: HttpTransport,
    uplink: Uplink,
    receiver: mpsc::Receiver<Result<RemoteEvent, RemoteEventError>>,
    state: Arc<Mutex<State>>,
}

impl RemoteEvents {
    /// Opens `$events` using the transport's authenticated cookie and mounted base URL.
    pub fn connect(
        transport: HttpTransport,
        mux: &RemoteStreamMux,
    ) -> Result<Self, RemoteEventError> {
        let stream = mux.open("$events", serde_json::json!({}))?;
        let (uplink, downlink) = (stream.uplink, stream.downlink);
        let state = Arc::new(Mutex::new(State::default()));
        let worker_state = state.clone();
        let worker_uplink = uplink.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for value in downlink {
                let result = (|| {
                    let frame = serde_json::from_value::<Downlink>(value?).map_err(Error::from)?;
                    worker_state
                        .lock()
                        .map_err(|_| RemoteEventError::StatePoisoned)?
                        .receive(frame)
                })();
                let failed = result.is_err();
                if failed {
                    if let Ok(mut state) = worker_state.lock() {
                        state.close();
                    }
                }
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
            if let Ok(mut state) = worker_state.lock() {
                state.close();
            }
            worker_uplink.cancel();
        });
        Ok(Self {
            transport,
            uplink,
            receiver,
            state,
        })
    }

    /// Waits up to `timeout` for a presentation event; `None` means no item yet.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<RemoteEvent>, RemoteEventError> {
        match self.receiver.recv_timeout(timeout) {
            Ok(result) => result.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RemoteEventError::Closed),
        }
    }

    /// Returns exactly one waterfall outcome. A failed HTTP attempt is not safe to retry.
    pub fn submit<T: Serialize>(
        &self,
        invocation: &RemoteEventInvocation,
        outcome: RemoteEventOutcome<T>,
    ) -> Result<(), RemoteEventError> {
        if let RemoteEventOutcome::Rejected { error } = &outcome {
            if error.name.is_empty() {
                return Err(RemoteEventError::Protocol("empty rejection name"));
            }
        }
        let payload = serde_json::to_value(RemoteEventResult {
            client_id: &invocation.client_id,
            event_id: &invocation.event_id,
            outcome,
        })
        .map_err(Error::from)?;
        self.state
            .lock()
            .map_err(|_| RemoteEventError::StatePoisoned)?
            .reserve(invocation)?;
        let result = self
            .transport
            .request("$events/result", payload, &invocation.cancelled);
        self.state
            .lock()
            .map_err(|_| RemoteEventError::StatePoisoned)?
            .pending
            .remove(&invocation.event_id);
        result?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{ApprovalDecision, PendingApproval};
    use crate::remote_stream::tests::{read, send};
    use serde_json::json;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
    };

    fn ready(state: &mut State) {
        state
            .receive(
                serde_json::from_value(
                    json!({"type":"ready","clientId":"client-1","host":{"home":"/home/test"}}),
                )
                .unwrap(),
            )
            .unwrap();
    }

    fn invocation(state: &mut State, id: &str) -> RemoteEventInvocation {
        let frame = serde_json::from_value(json!({"type":"waterfall","event":"approval/request","eventId":id,"agentId":"agent-1","request":{"toolName":"bash","callId":"call-1","reason":"run command","displayReason":{"en":"Run command","zh":"执行命令"}}})).unwrap();
        let RemoteEvent::Waterfall(invocation) = state.receive(frame).unwrap() else {
            panic!("expected waterfall")
        };
        invocation
    }

    #[test]
    fn exact_outcomes_distinguish_user_denial_delegation_and_listener_failure() {
        let client = RemoteEventClientId("c".into());
        let event = RemoteEventId("e".into());
        let rejection = RemoteEventRejection {
            name: "Error".into(),
            message: "failed".into(),
            code: Some("ui/failed".into()),
            details: Some(json!({"cause":1})),
        };
        for (outcome, expected) in [
            (
                RemoteEventOutcome::Result {
                    value: ApprovalDecision::AllowedOnce,
                },
                json!({"kind":"result","value":"allowed-once"}),
            ),
            (
                RemoteEventOutcome::Result {
                    value: ApprovalDecision::Rejected,
                },
                json!({"kind":"result","value":"rejected"}),
            ),
            (RemoteEventOutcome::Next, json!({"kind":"next"})),
            (
                RemoteEventOutcome::Rejected { error: rejection },
                json!({"kind":"rejected","error":{"name":"Error","message":"failed","code":"ui/failed","details":{"cause":1}}}),
            ),
        ] {
            assert_eq!(
                serde_json::to_value(RemoteEventResult {
                    client_id: &client,
                    event_id: &event,
                    outcome
                })
                .unwrap(),
                json!({"clientId":"c","eventId":"e","outcome":expected})
            );
        }
        assert!(serde_json::from_value::<ApprovalDecision>(json!("allow-always")).is_err());
    }

    #[test]
    fn cancellation_correlation_and_stream_lifetime_withdraw_presentations() {
        let mut state = State::default();
        ready(&mut state);
        let first = invocation(&mut state, "one");
        let second = invocation(&mut state, "two");
        state
            .receive(Downlink::Cancel {
                event_id: first.event_id.clone(),
            })
            .unwrap();
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert!(matches!(
            state.reserve(&first),
            Err(RemoteEventError::NotPending)
        ));
        let mut wrong_generation = second.clone();
        wrong_generation.client_id = RemoteEventClientId("other".into());
        assert!(matches!(
            state.reserve(&wrong_generation),
            Err(RemoteEventError::GenerationMismatch)
        ));
        let approval = PendingApproval::from_invocation(second.clone()).unwrap();
        assert_eq!(approval.request.tool_name, "bash");
        assert_eq!(
            approval.request.display_reason.unwrap().localized["zh"],
            "执行命令"
        );
        state.close();
        assert!(second.is_cancelled());
        assert!(matches!(
            state.reserve(&second),
            Err(RemoteEventError::Closed)
        ));
    }

    #[test]
    fn validates_ready_order_duplicate_ids_and_projected_requests() {
        let mut state = State::default();
        assert!(matches!(
            state.receive(Downlink::Cancel {
                event_id: RemoteEventId("x".into())
            }),
            Err(RemoteEventError::Protocol("frame before ready"))
        ));
        ready(&mut state);
        let first = invocation(&mut state, "one");
        assert!(matches!(
            state.receive(Downlink::Waterfall {
                event: first.event.clone(),
                event_id: first.event_id.clone(),
                agent_id: first.agent_id.clone(),
                request: first.request.clone()
            }),
            Err(RemoteEventError::Protocol(
                "invalid or duplicate invocation"
            ))
        ));
        for field in ["signal", "agent"] {
            assert!(matches!(
                state.receive(Downlink::Waterfall {
                    event: "approval/request".into(),
                    event_id: RemoteEventId("two".into()),
                    agent_id: first.agent_id.clone(),
                    request: Map::from_iter([(field.into(), json!({}))])
                }),
                Err(RemoteEventError::Protocol("unprojected request"))
            ));
        }
        state.reserve(&first).unwrap();
        assert!(matches!(
            state.reserve(&first),
            Err(RemoteEventError::NotPending)
        ));
        state
            .receive(Downlink::Cancel {
                event_id: first.event_id.clone(),
            })
            .unwrap();
        assert!(
            first.is_cancelled(),
            "cancellation also reaches in-flight submissions"
        );
    }

    fn accept(listener: &TcpListener) -> TcpStream {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
    }

    fn http_request(stream: &mut TcpStream) -> (String, String, Value) {
        let mut reader = BufReader::new(stream);
        let mut first = String::new();
        reader.read_line(&mut first).unwrap();
        let mut headers = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(!line.is_empty(), "unexpected HTTP EOF");
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            headers.push_str(&line.to_ascii_lowercase());
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        (
            first,
            headers,
            if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            },
        )
    }

    fn reply(stream: &mut TcpStream, body: &str, extra_headers: &str) {
        write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}\r\n{body}", body.len()).unwrap();
        stream.flush().unwrap();
    }

    fn redirect(stream: &mut TcpStream, location: &str, cookie: &str) {
        write!(stream, "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: {location}\r\nSet-Cookie: {cookie}\r\nContent-Length: 0\r\n\r\n").unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn authenticated_fixture_opens_events_and_posts_exact_correlated_decision() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/?token=secret", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut auth = accept(&listener);
            assert!(http_request(&mut auth)
                .0
                .starts_with("GET /connect?token=secret&interactions=answer "));
            redirect(
                &mut auth,
                "/api/remote.mux",
                "dsh-auth-test=secret; Path=/; HttpOnly",
            );
            drop(auth);
            let mut socket = tungstenite::accept_hdr(
                accept(&listener),
                |request: &tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri(), "/api/remote.mux");
                    assert_eq!(request.headers()["cookie"], "dsh-auth-test=secret");
                    Ok(response)
                },
            )
            .unwrap();
            let open = read(&mut socket);
            assert_eq!(
                open,
                json!({"type":"open","streamId":open["streamId"],"endpoint":"$events","payload":{"args":{}}})
            );
            for frame in [
                json!({"type":"ready","clientId":"client-1","host":{"home":"/home/test"}}),
                json!({"type":"waterfall","event":"approval/request","eventId":"one","agentId":"agent-1","request":{"toolName":"bash"}}),
                json!({"type":"waterfall","event":"approval/request","eventId":"two","agentId":"agent-1","request":{"toolName":"read"}}),
                json!({"type":"cancel","eventId":"two"}),
            ] {
                send(
                    &mut socket,
                    json!({"type":"item","streamId":open["streamId"],"value":frame}),
                );
            }
            let mut http = accept(&listener);
            let (line, headers, envelope) = http_request(&mut http);
            assert!(line.starts_with("POST /api/$events/result "));
            assert!(headers.contains("cookie: dsh-auth-test=secret"));
            assert_eq!(
                envelope,
                json!({"type":"client-request","rpcId":envelope["rpcId"],"method":"$events/result","payload":{"args":{"clientId":"client-1","eventId":"one","outcome":{"kind":"result","value":"allowed-once"}}}})
            );
            reply(
                &mut http,
                &json!({"type":"server-response","rpcId":envelope["rpcId"],"result":{"ok":true}})
                    .to_string(),
                "",
            );
            assert_eq!(
                read(&mut socket),
                json!({"type":"cancel","streamId":open["streamId"]})
            );
        });
        let (_, transport) = HttpTransport::authenticate(&url).unwrap();
        let mux = RemoteStreamMux::connect(
            transport.base_url().as_str(),
            transport.credential().unwrap(),
        )
        .unwrap();
        let client = RemoteEvents::connect(transport, &mux).unwrap();
        assert!(matches!(
            client.recv_timeout(Duration::from_secs(5)).unwrap(),
            Some(RemoteEvent::Ready { .. })
        ));
        let Some(RemoteEvent::Waterfall(first)) =
            client.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("expected first approval")
        };
        let Some(RemoteEvent::Waterfall(second)) =
            client.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("expected second approval")
        };
        assert!(matches!(
            client.recv_timeout(Duration::from_secs(5)).unwrap(),
            Some(RemoteEvent::Cancel { .. })
        ));
        assert!(second.is_cancelled());
        let cancelled = PendingApproval::from_invocation(second).unwrap();
        assert!(matches!(
            cancelled.answer(&client, ApprovalDecision::Rejected),
            Err(RemoteEventError::NotPending)
        ));
        let approval = PendingApproval::from_invocation(first).unwrap();
        approval
            .answer(&client, ApprovalDecision::AllowedOnce)
            .unwrap();
        assert!(matches!(
            approval.delegate(&client),
            Err(RemoteEventError::NotPending)
        ));
        drop(client);
        worker.join().unwrap();
    }
}

impl Drop for RemoteEvents {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.close();
        }
        self.uplink.cancel();
    }
}
