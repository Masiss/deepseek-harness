//! Authenticated Gateway Remote stream mux framing for native stream clients.

use crate::{auth::Credential, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{net::TcpStream, sync::mpsc, thread, time::Duration};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};
use url::Url;

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

/// Opens a logical stream over the Gateway's authenticated mux WebSocket.
pub(crate) fn open(
    base_url: &str,
    credential: &Credential,
    endpoint: &str,
    args: Value,
) -> Result<
    (
        String,
        mpsc::Sender<ClientFrame>,
        mpsc::Receiver<Result<Value, Error>>,
    ),
    Error,
> {
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
    let stream_id = format!(
        "native-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| Error::InvalidCredential)?
            .as_nanos()
    );
    socket
        .send(Message::Text(
            serde_json::to_string(&ClientFrame::Open {
                stream_id: stream_id.clone(),
                endpoint: endpoint.into(),
                payload: serde_json::json!({ "args": args }),
            })?
            .into(),
        ))
        .map_err(|_| Error::InvalidCredential)?;
    let (uplink, uplink_rx) = mpsc::channel::<ClientFrame>();
    let (downlink_tx, downlink) = mpsc::channel();
    let id = stream_id.clone();
    thread::spawn(move || run(socket, id, uplink_rx, downlink_tx));
    Ok((stream_id, uplink, downlink))
}

fn set_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<(), Error> {
    let stream = match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => &mut stream.sock,
        _ => return Err(Error::InvalidCredential),
    };
    stream
        .set_read_timeout(Some(Duration::from_millis(40)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(1))))
        .map_err(|_| Error::InvalidCredential)
}

fn run(
    mut socket: WebSocket<MaybeTlsStream<TcpStream>>,
    id: String,
    uplink: mpsc::Receiver<ClientFrame>,
    downlink: mpsc::Sender<Result<Value, Error>>,
) {
    'worker: loop {
        loop {
            let frame = match uplink.try_recv() {
                Ok(frame) => frame,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = socket.send(Message::Text(
                        serde_json::json!({"type":"cancel","streamId":id})
                            .to_string()
                            .into(),
                    ));
                    break 'worker;
                }
            };
            let cancel = matches!(frame, ClientFrame::Cancel { .. });
            if socket
                .send(Message::Text(match serde_json::to_string(&frame) {
                    Ok(text) => text.into(),
                    Err(error) => {
                        let _ = downlink.send(Err(error.into()));
                        break 'worker;
                    }
                }))
                .is_err()
            {
                let _ = downlink.send(Err(Error::InvalidEnvelope(
                    "Remote stream socket write failed",
                )));
                break 'worker;
            }
            if cancel {
                break 'worker;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => match serde_json::from_str::<ServerFrame>(&text) {
                Ok(ServerFrame::Item { stream_id, value }) if stream_id == id => {
                    if downlink.send(Ok(value.unwrap_or(Value::Null))).is_err() {
                        break;
                    }
                }
                Ok(ServerFrame::End { stream_id }) if stream_id == id => break,
                Ok(ServerFrame::Error { stream_id, error }) if stream_id == id => {
                    let _ = downlink.send(Err(Error::Remote {
                        code: error.code,
                        message: error.message,
                        details: error.details,
                    }));
                    break;
                }
                Ok(_) => {
                    let _ = downlink.send(Err(Error::InvalidEnvelope("Remote stream correlation")));
                    break;
                }
                Err(error) => {
                    let _ = downlink.send(Err(error.into()));
                    break;
                }
            },
            Ok(Message::Close(_)) => {
                let _ = downlink.send(Err(Error::InvalidEnvelope(
                    "Remote stream disconnected before end",
                )));
                break;
            }
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => {
                let _ = downlink.send(Err(Error::InvalidEnvelope(
                    "Remote stream socket read failed",
                )));
                break;
            }
            Ok(Message::Ping(_)) => {
                if socket.flush().is_err() {
                    let _ = downlink.send(Err(Error::InvalidEnvelope(
                        "Remote stream socket flush failed",
                    )));
                    break;
                }
            }
            Ok(_) => {}
        }
    }
    let _ = socket.close(None);
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
    fn authenticated_socket_flushes_open_and_distinguishes_end_from_disconnect() {
        for clean_end in [true, false] {
            let (url, credential, worker) = server(move |mut socket| {
                let open = read(&mut socket);
                assert_eq!(open["type"], "open");
                assert_eq!(open["endpoint"], "test/stream");
                assert_eq!(open["payload"], serde_json::json!({"args":{"id":"t"}}));
                let id = &open["streamId"];
                send(
                    &mut socket,
                    serde_json::json!({"type":"item","streamId":id,"value":{"answer":42}}),
                );
                if clean_end {
                    send(&mut socket, serde_json::json!({"type":"end","streamId":id}));
                } else {
                    socket.close(None).unwrap();
                }
            });
            let (_, _uplink, receiver) = open(
                &url,
                &credential,
                "test/stream",
                serde_json::json!({"id":"t"}),
            )
            .unwrap();
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap(),
                serde_json::json!({"answer":42})
            );
            let next = receiver.recv_timeout(Duration::from_secs(3));
            if clean_end {
                assert!(matches!(next, Err(mpsc::RecvTimeoutError::Disconnected)));
            } else {
                assert!(matches!(
                    next,
                    Ok(Err(Error::InvalidEnvelope(
                        "Remote stream disconnected before end"
                    )))
                ));
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn dropping_all_uplink_handles_cancels_an_idle_subscription() {
        let (url, credential, worker) = server(|mut socket| {
            let open = read(&mut socket);
            assert_eq!(
                read(&mut socket),
                serde_json::json!({"type":"cancel","streamId":open["streamId"]})
            );
        });
        let (_, uplink, receiver) =
            open(&url, &credential, "test/stream", serde_json::json!({})).unwrap();
        drop(uplink);
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(3)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
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
        let (_, _uplink, receiver) =
            open(&url, &credential, "test/stream", serde_json::json!({})).unwrap();
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
    fn rejects_wrong_stream_correlation_and_malformed_socket_json() {
        for malformed_json in [false, true] {
            let (url, credential, worker) = server(move |mut socket| {
                let _ = read(&mut socket);
                if malformed_json {
                    socket.send(Message::Text("{".into())).unwrap();
                } else {
                    send(
                        &mut socket,
                        serde_json::json!({"type":"end","streamId":"unrelated"}),
                    );
                }
            });
            let (_, _uplink, receiver) =
                open(&url, &credential, "test/stream", serde_json::json!({})).unwrap();
            assert!(matches!(
                receiver.recv_timeout(Duration::from_secs(3)),
                Ok(Err(Error::InvalidEnvelope(_)))
            ));
            worker.join().unwrap();
        }
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

    #[test]
    fn server_error_retains_failure_fields_and_stream_correlation() {
        let parsed: ServerFrame = serde_json::from_value(serde_json::json!({"type":"error","streamId":"s1","error":{"code":"denied","message":"No","details":{"reason":"policy"}}})).unwrap();
        assert_eq!(
            parsed,
            ServerFrame::Error {
                stream_id: "s1".into(),
                error: RemoteFailure {
                    code: "denied".into(),
                    message: "No".into(),
                    details: serde_json::json!({"reason":"policy"})
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                }
            }
        );
    }
}
