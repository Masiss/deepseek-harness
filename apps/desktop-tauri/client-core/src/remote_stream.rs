//! Authenticated Gateway Remote stream mux framing for native stream clients.
//!
//! One physical WebSocket carries every logical stream. A lost socket fails the
//! streams that were open on it and is replaced with backoff; opens issued while
//! the carrier is down wait for the next socket instead of failing the caller.

use crate::{auth::Credential, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    net::TcpStream,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

const CANCEL_POLL: Duration = Duration::from_millis(40);
const RECONNECT_MIN: Duration = Duration::from_millis(200);
const RECONNECT_MAX: Duration = Duration::from_secs(5);
const MAX_QUEUED_OPENS: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum ClientFrame {
    Open {
        #[serde(rename = "streamId")]
        stream_id: String,
        endpoint: String,
        payload: Value,
    },
    Item {
        #[serde(rename = "streamId")]
        stream_id: String,
        value: Value,
    },
    End {
        #[serde(rename = "streamId")]
        stream_id: String,
    },
    Cancel {
        #[serde(rename = "streamId")]
        stream_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub(crate) enum ServerFrame {
    Item {
        #[serde(rename = "streamId")]
        stream_id: String,
        value: Option<Value>,
    },
    Error {
        #[serde(rename = "streamId")]
        stream_id: String,
        error: RemoteFailure,
    },
    End {
        #[serde(rename = "streamId")]
        stream_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemoteFailure {
    pub code: String,
    pub message: String,
    pub details: serde_json::Map<String, Value>,
}

enum Command {
    Open {
        id: String,
        endpoint: String,
        args: Value,
        downlink: mpsc::Sender<Result<Value, Error>>,
    },
    Item {
        id: String,
        value: Value,
    },
    End {
        id: String,
    },
    Cancel {
        id: String,
    },
}

/// Sends this logical stream's uplink frames through its mux.
#[derive(Clone)]
pub struct Uplink {
    id: String,
    commands: mpsc::Sender<Command>,
}

impl Uplink {
    /// Sends one uplink item.
    pub fn item(&self, value: Value) {
        let _ = self.commands.send(Command::Item {
            id: self.id.clone(),
            value,
        });
    }

    /// Half-closes the uplink.
    pub fn end(&self) {
        let _ = self.commands.send(Command::End {
            id: self.id.clone(),
        });
    }

    /// Cancels the logical stream and releases its uplink.
    pub fn cancel(&self) {
        let _ = self.commands.send(Command::Cancel {
            id: self.id.clone(),
        });
    }
}

/// One open logical stream: its identity, uplink, and downlink frames.
pub struct RemoteStream {
    pub id: String,
    pub uplink: Uplink,
    pub downlink: mpsc::Receiver<Result<Value, Error>>,
}

/// Shared authenticated mux whose physical socket reconnects after a drop.
#[derive(Clone)]
pub struct RemoteStreamMux {
    commands: mpsc::Sender<Command>,
}

impl RemoteStreamMux {
    /// Opens the first physical socket; later drops reconnect in the worker.
    pub fn connect(base_url: &str, credential: &Credential) -> Result<Self, Error> {
        let socket = open_socket(base_url, credential)?;
        let (commands, receiver) = mpsc::channel();
        let base = base_url.to_owned();
        let cookie = credential.clone();
        thread::spawn(move || run(socket, base, cookie, receiver));
        Ok(Self { commands })
    }

    /// Opens one logical stream over the shared socket.
    pub fn open(&self, endpoint: &str, args: Value) -> Result<RemoteStream, Error> {
        let id = next_stream_id();
        let (downlink, receiver) = mpsc::channel();
        self.commands
            .send(Command::Open {
                id: id.clone(),
                endpoint: endpoint.to_owned(),
                args,
                downlink,
            })
            .map_err(|_| Error::InvalidEnvelope("Remote stream mux is closed"))?;
        Ok(RemoteStream {
            id: id.clone(),
            uplink: Uplink {
                id,
                commands: self.commands.clone(),
            },
            downlink: receiver,
        })
    }
}

fn next_stream_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "native-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn open_socket(
    base_url: &str,
    credential: &Credential,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, Error> {
    let mut url = Url::parse(base_url).map_err(|source| Error::InvalidUrl { source })?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        _ => return Err(Error::InvalidCredential),
    };
    url.set_scheme(scheme)
        .map_err(|_| Error::InvalidCredential)?;
    url.set_path(&format!(
        "{}/api/remote.mux",
        url.path().trim_end_matches('/')
    ));
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| Error::InvalidCredential)?;
    request.headers_mut().insert(
        tungstenite::http::header::COOKIE,
        credential
            .cookie
            .parse()
            .map_err(|_| Error::InvalidCredential)?,
    );
    let (mut socket, _) =
        tungstenite::client::connect(request).map_err(|_| Error::InvalidCredential)?;
    set_read_timeout(&mut socket)?;
    Ok(socket)
}

fn set_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<(), Error> {
    let stream = match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => &mut stream.sock,
        _ => return Err(Error::InvalidCredential),
    };
    stream
        .set_read_timeout(Some(CANCEL_POLL))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(1))))
        .map_err(|_| Error::InvalidCredential)
}

struct PendingOpen {
    id: String,
    endpoint: String,
    args: Value,
    downlink: mpsc::Sender<Result<Value, Error>>,
}

fn run(
    mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
    base_url: String,
    credential: Credential,
    commands: mpsc::Receiver<Command>,
) {
    let mut streams: HashMap<String, mpsc::Sender<Result<Value, Error>>> = HashMap::new();
    let mut queued: VecDeque<PendingOpen> = VecDeque::new();
    let mut backoff = RECONNECT_MIN;
    loop {
        let reconnect = serve(&mut socket, &commands, &mut streams);
        match reconnect {
            Serve::Shutdown => return,
            Serve::Disconnected => {}
        }
        fail_streams(&mut streams, "Remote stream disconnected before end");
        // Replace the carrier; opens issued in the meantime queue until it is up.
        loop {
            match open_socket(&base_url, &credential) {
                Ok(next) => {
                    socket = next;
                    backoff = RECONNECT_MIN;
                    for open in queued.drain(..) {
                        send_open(&mut socket, open, &mut streams);
                    }
                    break;
                }
                Err(_) => {
                    backoff = (backoff * 2).min(RECONNECT_MAX);
                    let deadline = Instant::now() + backoff;
                    while Instant::now() < deadline {
                        match commands.recv_timeout(Duration::from_millis(50)) {
                            Ok(command) => queue_command(command, &mut queued),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                }
            }
        }
    }
}

enum Serve {
    Shutdown,
    Disconnected,
}

fn serve(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    commands: &mpsc::Receiver<Command>,
    streams: &mut HashMap<String, mpsc::Sender<Result<Value, Error>>>,
) -> Serve {
    loop {
        // Drain every queued command before reading so uplink latency stays low.
        loop {
            match commands.try_recv() {
                Ok(Command::Open {
                    id,
                    endpoint,
                    args,
                    downlink,
                }) => {
                    send_open(
                        socket,
                        PendingOpen {
                            id,
                            endpoint,
                            args,
                            downlink,
                        },
                        streams,
                    );
                }
                Ok(command) => forward(socket, command, streams),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.close(None);
                    return Serve::Shutdown;
                }
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => match serde_json::from_str::<ServerFrame>(&text) {
                Ok(ServerFrame::Item { stream_id, value }) => {
                    if let Some(downlink) = streams.get(&stream_id) {
                        if downlink.send(Ok(value.unwrap_or(Value::Null))).is_err() {
                            streams.remove(&stream_id);
                        }
                    }
                }
                Ok(ServerFrame::End { stream_id }) => {
                    streams.remove(&stream_id);
                }
                Ok(ServerFrame::Error { stream_id, error }) => {
                    if let Some(downlink) = streams.remove(&stream_id) {
                        let _ = downlink.send(Err(Error::Remote {
                            code: error.code,
                            message: error.message,
                            details: error.details,
                        }));
                    }
                }
                Err(_) => {
                    fail_streams(streams, "Remote stream sent invalid JSON");
                    return Serve::Disconnected;
                }
            },
            Ok(Message::Close(_)) => return Serve::Disconnected,
            Ok(Message::Ping(_)) => {
                if socket.flush().is_err() {
                    return Serve::Disconnected;
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return Serve::Disconnected,
        }
    }
}

fn send_open(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    open: PendingOpen,
    streams: &mut HashMap<String, mpsc::Sender<Result<Value, Error>>>,
) {
    let frame = ClientFrame::Open {
        stream_id: open.id.clone(),
        endpoint: open.endpoint,
        payload: serde_json::json!({ "args": open.args }),
    };
    match serde_json::to_string(&frame) {
        Ok(text) => {
            if socket.send(Message::Text(text.into())).is_ok() {
                streams.insert(open.id, open.downlink);
            } else {
                let _ =
                    open.downlink
                        .send(Err(Error::InvalidEnvelope("Remote stream socket write failed")));
            }
        }
        Err(error) => {
            let _ = open.downlink.send(Err(error.into()));
        }
    }
}

fn forward(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    command: Command,
    streams: &mut HashMap<String, mpsc::Sender<Result<Value, Error>>>,
) {
    let frame = match command {
        Command::Item { id, value } => ClientFrame::Item {
            stream_id: id,
            value,
        },
        Command::End { id } => ClientFrame::End { stream_id: id },
        Command::Cancel { id } => {
            streams.remove(&id);
            ClientFrame::Cancel { stream_id: id }
        }
        Command::Open { .. } => return,
    };
    if let Ok(text) = serde_json::to_string(&frame) {
        if socket.send(Message::Text(text.into())).is_err() {
            fail_streams(streams, "Remote stream socket write failed");
        }
    }
}

fn fail_streams(
    streams: &mut HashMap<String, mpsc::Sender<Result<Value, Error>>>,
    message: &'static str,
) {
    for (_, downlink) in streams.drain() {
        let _ = downlink.send(Err(Error::InvalidEnvelope(message)));
    }
}

fn queue_command(command: Command, queued: &mut VecDeque<PendingOpen>) {
    match command {
        Command::Open {
            id,
            endpoint,
            args,
            downlink,
        } => {
            if queued.len() < MAX_QUEUED_OPENS {
                queued.push_back(PendingOpen {
                    id,
                    endpoint,
                    args,
                    downlink,
                });
            } else {
                let _ = downlink.send(Err(Error::InvalidEnvelope(
                    "Remote stream mux reconnect queue is full",
                )));
            }
        }
        Command::Item { .. } | Command::End { .. } | Command::Cancel { .. } => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn server(
        handle: impl FnOnce(WebSocket<TcpStream>) + Send + 'static,
    ) -> (String, Credential, thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let socket = tungstenite::accept_hdr(
                stream,
                |request: &tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.method(), "GET");
                    assert_eq!(request.uri(), "/mount/api/remote.mux");
                    let headers = request.headers();
                    assert!(headers.contains_key("host"));
                    assert_eq!(headers["upgrade"], "websocket");
                    assert_eq!(headers["connection"], "Upgrade");
                    assert_eq!(headers["sec-websocket-version"], "13");
                    assert_eq!(headers["sec-websocket-key"].to_str().unwrap().len(), 24);
                    assert_eq!(headers["cookie"], "dsh-auth-test=secret");
                    Ok(response)
                },
            )
            .unwrap();
            handle(socket);
        });
        (
            format!("http://{address}/mount/"),
            Credential {
                cookie: "dsh-auth-test=secret".into(),
            },
            worker,
        )
    }

    pub(crate) fn read(socket: &mut WebSocket<TcpStream>) -> Value {
        let message = socket.read().unwrap();
        serde_json::from_str(message.to_text().unwrap()).unwrap()
    }

    pub(crate) fn send(socket: &mut WebSocket<TcpStream>, value: Value) {
        socket
            .send(Message::Text(value.to_string().into()))
            .unwrap();
    }

    fn stream(
        mux: &RemoteStreamMux,
        endpoint: &str,
        args: Value,
    ) -> mpsc::Receiver<Result<Value, Error>> {
        mux.open(endpoint, args).unwrap().downlink
    }

    #[test]
    fn frames_match_gateway_mux_wire_protocol() {
        let open = serde_json::to_value(ClientFrame::Open {
            stream_id: "s1".into(),
            endpoint: "terminal/attach".into(),
            payload: serde_json::json!({"args":{"agentId":"a","id":"t"}}),
        })
        .unwrap();
        assert_eq!(
            open,
            serde_json::json!({"type":"open","streamId":"s1","endpoint":"terminal/attach","payload":{"args":{"agentId":"a","id":"t"}}})
        );
        let item = serde_json::to_value(ClientFrame::Item {
            stream_id: "s1".into(),
            value: serde_json::json!({"type":"write","sequence":1,"data":"x"}),
        })
        .unwrap();
        assert_eq!(
            item,
            serde_json::json!({"type":"item","streamId":"s1","value":{"type":"write","sequence":1,"data":"x"}})
        );
        let parsed: ServerFrame = serde_json::from_value(
            serde_json::json!({"type":"item","streamId":"s1","value":{"type":"snapshot"}}),
        )
        .unwrap();
        assert_eq!(
            parsed,
            ServerFrame::Item {
                stream_id: "s1".into(),
                value: Some(serde_json::json!({"type":"snapshot"}))
            }
        );
    }

    #[test]
    fn one_physical_socket_serves_two_logical_streams() {
        let (url, credential, worker) = server(|mut socket| {
            let first = read(&mut socket);
            let second = read(&mut socket);
            assert_eq!(first["type"], "open");
            assert_eq!(second["type"], "open");
            for open in [&first, &second] {
                send(
                    &mut socket,
                    serde_json::json!({
                        "type":"item","streamId":open["streamId"],"value":{"endpoint":open["endpoint"]}
                    }),
                );
            }
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":first["streamId"]}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":second["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let first = stream(&mux, "session/follow", serde_json::json!({}));
        let second = stream(&mux, "terminal/attach", serde_json::json!({}));
        assert_eq!(
            first.recv_timeout(Duration::from_secs(3)).unwrap().unwrap()["endpoint"],
            "session/follow"
        );
        assert_eq!(
            second.recv_timeout(Duration::from_secs(3)).unwrap().unwrap()["endpoint"],
            "terminal/attach"
        );
        worker.join().unwrap();
    }

    #[test]
    fn reconnect_replaces_the_carrier_and_fails_open_streams() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (first, _) = listener.accept().unwrap();
            let mut first = tungstenite::accept(first).unwrap();
            let _open = first.read().unwrap();
            first.close(None).unwrap();
            drop(first);
            let (second, _) = listener.accept().unwrap();
            second
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut second = tungstenite::accept(second).unwrap();
            let open = second.read().unwrap();
            let open: Value = serde_json::from_str(open.to_text().unwrap()).unwrap();
            second
                .send(Message::Text(
                    serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"after":"reconnect"}})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            second
                .send(Message::Text(
                    serde_json::json!({"type":"end","streamId":open["streamId"]})
                        .to_string()
                        .into(),
                ))
                .unwrap();
        });
        let url = format!("http://{address}/");
        let credential = Credential {
            cookie: "dsh-auth-test=secret".into(),
        };
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let first = stream(&mux, "session/follow", serde_json::json!({}));
        // The failed carrier reports the loss to the stream that was open on it.
        let failure = first.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(failure.is_err(), "expected carrier failure");
        // A later open waits for the replacement socket.
        let second = stream(&mux, "session/follow", serde_json::json!({}));
        let value = second.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(value["after"], "reconnect");
        worker.join().unwrap();
    }

    #[test]
    fn clean_end_is_distinguished_from_disconnect() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"answer":42}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let receiver = stream(&mux, "test/stream", serde_json::json!({"id":"t"}));
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap(),
            serde_json::json!({"answer":42})
        );
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(3)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        worker.join().unwrap();
    }

    #[test]
    fn dropping_the_uplink_cancels_an_idle_subscription() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            let cancel = read(&mut socket);
            assert_eq!(cancel["type"], "cancel");
            assert_eq!(cancel["streamId"], open["streamId"]);
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let stream = mux.open("test/stream", serde_json::json!({})).unwrap();
        stream.uplink.cancel();
        drop(stream);
        worker.join().unwrap();
    }

    #[test]
    fn socket_flushes_pong_and_preserves_remote_failure() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            socket.send(Message::Ping(vec![1, 2, 3])).unwrap();
            assert_eq!(socket.read().unwrap(), Message::Pong(vec![1, 2, 3]));
            send(
                &mut socket,
                serde_json::json!({"type":"error","streamId":open["streamId"],"error":{"code":"denied","message":"No","details":{"reason":"policy"}}}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let receiver = stream(&mux, "test/stream", serde_json::json!({}));
        let error = receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap_err();
        let Error::Remote {
            code,
            message,
            details,
        } = error
        else {
            panic!("expected remote failure")
        };
        assert_eq!(code, "denied");
        assert_eq!(message, "No");
        assert_eq!(
            details,
            serde_json::json!({"reason":"policy"})
                .as_object()
                .unwrap()
                .clone()
        );
        worker.join().unwrap();
    }

    #[test]
    fn malformed_socket_json_fails_open_streams() {
        let (url, credential, worker) = server(|mut socket| {
            let _ = read(&mut socket);
            socket.send(Message::Text("{".into())).unwrap();
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let receiver = stream(&mux, "test/stream", serde_json::json!({}));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(3)),
            Ok(Err(Error::InvalidEnvelope(_)))
        ));
        worker.join().unwrap();
    }

    #[test]
    fn frames_for_other_streams_are_ignored() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":"unrelated"}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"kept":true}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let receiver = stream(&mux, "test/stream", serde_json::json!({}));
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(3)).unwrap().unwrap(),
            serde_json::json!({"kept":true})
        );
        worker.join().unwrap();
    }

    #[test]
    fn rejects_invalid_mux_fields_and_failure_details() {
        for value in [
            serde_json::json!({"type":"end","stream_id":"s1"}),
            serde_json::json!({"type":"end","streamId":"s1","extra":1}),
            serde_json::json!({"type":"error","streamId":"s1","error":{"code":"bad","message":"bad","details":[]}}),
        ] {
            assert!(serde_json::from_value::<ServerFrame>(value).is_err());
        }
        for value in [
            serde_json::json!({"type":"item","streamId":"s1"}),
            serde_json::json!({"type":"item","streamId":"s1","value":null}),
        ] {
            assert_eq!(
                serde_json::from_value::<ServerFrame>(value).unwrap(),
                ServerFrame::Item {
                    stream_id: "s1".into(),
                    value: None
                }
            );
        }
    }
}
