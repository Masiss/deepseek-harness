//! Windowless UI harness for the shipped shell.
//!
//! `egui_kittest` drives the same `DesktopApp::tick`/`DesktopApp::render` pair
//! the window runs, addressing widgets by AccessKit name instead of screen
//! coordinates. Transport calls reach an in-process fixture Host whose replies
//! are deterministic, so a test asserts widget identity, app state, and the
//! requests the UI issued — never pixels or pointer positions.

use super::*;
use dsh_client_core::remote_events::RemoteEventInvocation;
use dsh_client_core::remote_stream::RemoteStreamMux;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

/// One RPC the interface issued, as the fixture Host received it.
#[derive(Debug, Clone)]
struct FixtureRequest {
    rpc_id: String,
    method: String,
    payload: Value,
}

/// Mutable fixture Host state; the worker answers one request at a time.
struct FixtureState {
    created: Vec<String>,
    pages: BTreeMap<String, Vec<Value>>,
    page_limit: usize,
    /// One redacted settings namespace, or `None` before a describe.
    settings: Option<Value>,
    fail_settings_write: bool,
    fail_install: bool,
    /// PDF bytes an Office conversion returns.
    office_pdf: Option<Vec<u8>>,
}

impl Default for FixtureState {
    fn default() -> Self {
        Self {
            created: Vec::new(),
            pages: BTreeMap::new(),
            page_limit: PAGE_LIMIT,
            settings: None,
            fail_settings_write: false,
            fail_install: false,
            office_pdf: None,
        }
    }
}

impl FixtureState {
    /// Deterministic reply for one method, or an RPC error the UI must report.
    fn reply(&mut self, method: &str, payload: &Value) -> Result<Value, (String, String)> {
        // The transport wraps call arguments under `args`.
        let payload = payload.get("args").unwrap_or(payload);
        match method {
            "session/list" => {
                let mut items: Vec<Value> = self
                    .pages
                    .keys()
                    .map(|id| session_item(id, None))
                    .collect();
                items.extend(self.created.iter().map(|id| session_item(id, Some(()))));
                Ok(serde_json::json!({"items": items}))
            }
            "session/create" => {
                self.created.push("fixture-created".to_owned());
                Ok(serde_json::json!({
                    "sessionId": "fixture-created",
                    "agentPreset": null,
                }))
            }
            "session/page" => {
                let request = payload
                    .get("request")
                    .or_else(|| payload.get("args").and_then(|args| args.get("request")));
                let session_id = request
                    .and_then(|request| request.get("address"))
                    .and_then(|address| address.get("sessionId"))
                    .and_then(Value::as_str)
                    .unwrap_or("fixture-session");
                let before = request
                    .and_then(|request| request.get("beforeSeq"))
                    .and_then(Value::as_i64);
                let all = self.pages.get(session_id).cloned().unwrap_or_default();
                let mut visible: Vec<Value> = match before {
                    Some(before) => all
                        .iter()
                        .filter(|event| event["seq"].as_i64().unwrap_or_default() < before)
                        .cloned()
                        .collect(),
                    None => all.clone(),
                };
                let has_more = visible.len() > self.page_limit;
                if has_more {
                    visible = visible.split_off(visible.len() - self.page_limit);
                }
                let records: Vec<Value> = visible
                    .into_iter()
                    .map(|event| serde_json::json!({"type": "event", "event": event}))
                    .collect();
                Ok(serde_json::json!({"records": records, "hasMore": has_more}))
            }
            "session/prompt" => Ok(serde_json::json!({"accepted": true})),
            "model/catalog" => Ok(serde_json::json!({
                "default": {"provider": "fixture", "model": "fixture-model"},
                "routableProviders": ["fixture"],
                "groups": [{
                    "id": "fixture",
                    "name": "Fixture provider",
                    "models": [{"id": "fixture-model", "name": "Fixture model"}],
                }],
                "failures": [],
            })),
            "settings/describe" => Ok(serde_json::json!({
                "writable": true,
                "hasDocument": true,
                "namespaces": self.settings.iter().cloned().collect::<Vec<_>>(),
            })),
            "settings/mutate" => {
                if self.fail_settings_write {
                    return Err((
                        "SETTINGS_CONFLICT".to_owned(),
                        "another writer moved the revision".to_owned(),
                    ));
                }
                let Some(namespace) = self.settings.as_mut() else {
                    return Err(("SETTINGS_MISSING".to_owned(), "no namespace".to_owned()));
                };
                let user = namespace
                    .get_mut("user")
                    .map(|user| {
                        if !user.is_object() {
                            *user = serde_json::json!({});
                        }
                        user
                    })
                    .expect("fixture namespace user");
                for op in payload["ops"].as_array().into_iter().flatten() {
                    let path: Vec<&str> = op["path"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect();
                    match op["op"].as_str() {
                        Some("set") => set_path(user, &path, op["value"].clone()),
                        Some("unset") => unset_path(user, &path),
                        _ => {}
                    }
                }
                let value = user.clone();
                namespace["value"] = value;
                let revision = namespace["revision"].as_u64().unwrap_or(0) + 1;
                namespace["revision"] = serde_json::json!(revision);
                Ok(namespace.clone())
            }
            "pluginMarket/listIndexes" => Ok(serde_json::json!([{
                "url": "https://market.fixture/index.json",
                "name": "Fixture market",
            }])),
            "pluginMarket/search" => Ok(serde_json::json!({"entries": [{
                "name": "fixture-plugin",
                "description": "Fixture bundle",
                "versions": [{"version": "1.0.0", "install": {"spec": "fixture-plugin@1.0.0"}}],
            }]})),
            "pluginManager/inspect" => Ok(serde_json::json!({"status": "accepted"})),
            "pluginManager/installBundle" => {
                if self.fail_install {
                    return Err((
                        "PLUGIN_INSTALL_FAILED".to_owned(),
                        "the fixture refused this install".to_owned(),
                    ));
                }
                Ok(serde_json::json!({"application": "installed"}))
            }
            "pluginInventory/list" => Ok(serde_json::json!({"entries": []})),
            "officeToPdf/render" => Ok(serde_json::json!({
                "data": self.office_pdf.clone().unwrap_or_default(),
                "missingFonts": [],
            })),
            _ => Ok(Value::Null),
        }
    }
}

/// Writes one path operation into a JSON document.
fn set_path(target: &mut Value, path: &[&str], value: Value) {
    let Some((head, rest)) = path.split_first() else {
        *target = value;
        return;
    };
    if !target.is_object() {
        *target = serde_json::json!({});
    }
    let map = target.as_object_mut().expect("object just ensured");
    if rest.is_empty() {
        map.insert((*head).to_owned(), value);
        return;
    }
    set_path(map.entry((*head).to_owned()).or_insert(serde_json::json!({})), rest, value);
}

/// Removes one path from a JSON document.
fn unset_path(target: &mut Value, path: &[&str]) {
    let Some((head, rest)) = path.split_first() else {
        return;
    };
    let Some(map) = target.as_object_mut() else {
        return;
    };
    if rest.is_empty() {
        map.remove(*head);
        return;
    }
    if let Some(nested) = map.get_mut(*head) {
        unset_path(nested, rest);
    }
}

/// In-process Host that answers the RPCs a windowless test needs.
struct FixtureHost {
    url: String,
    requests: Arc<Mutex<Vec<FixtureRequest>>>,
    /// Host attach handshakes the interface attempted.
    connect_attempts: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    state: Arc<Mutex<FixtureState>>,
}

impl FixtureHost {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let url = format!(
            "http://{}/",
            listener.local_addr().expect("fixture address")
        );
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connect_attempts = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(FixtureState::default()));
        let worker_requests = Arc::clone(&requests);
        let worker_attempts = Arc::clone(&connect_attempts);
        let worker_state = Arc::clone(&state);
        let worker_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                let (stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let Some(call) = read_call(&stream) else {
                    drop(stream);
                    continue;
                };
                if call.head.starts_with("get /connect") {
                    // A refused attach: the interface must report the failure,
                    // never claim it reconnected.
                    worker_attempts.fetch_add(1, Ordering::AcqRel);
                    send_status(stream, 503);
                    continue;
                }
                let Some(request) = call.rpc else {
                    drop(stream);
                    continue;
                };
                let rpc_id = request.rpc_id.clone();
                let reply = {
                    let mut state = worker_state.lock().expect("fixture state");
                    state.reply(&request.method, &request.payload)
                };
                worker_requests
                    .lock()
                    .expect("fixture requests")
                    .push(request);
                send_response(stream, &rpc_id, reply);
            }
        });
        Self {
            url,
            requests,
            connect_attempts,
            stop,
            thread: Some(thread),
            state,
        }
    }

    /// Sets how many events one page reply carries.
    fn set_page_limit(&self, limit: usize) {
        self.state.lock().expect("fixture state").page_limit = limit;
    }

    /// Publishes one Session's durable history.
    fn set_page(&self, session_id: &str, records: Vec<Value>) {
        self.state
            .lock()
            .expect("fixture state")
            .pages
            .insert(session_id.to_owned(), records);
    }

    /// Publishes the redacted settings namespace `settings/describe` returns.
    fn set_settings_namespace(&self, namespace: Value) {
        self.state.lock().expect("fixture state").settings = Some(namespace);
    }

    /// Makes the next settings write fail with a revision conflict.
    fn fail_settings_write(&self) {
        self.state.lock().expect("fixture state").fail_settings_write = true;
    }

    /// Makes bundle installation fail after inspection succeeds.
    fn fail_install(&self) {
        self.state.lock().expect("fixture state").fail_install = true;
    }

    /// Sets the PDF bytes an Office conversion returns.
    fn set_office_pdf(&self, pdf: Vec<u8>) {
        self.state.lock().expect("fixture state").office_pdf = Some(pdf);
    }

    /// Session ids the fixture lists, in order.
    fn session_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("fixture state")
            .pages
            .keys()
            .cloned()
            .collect()
    }

    /// Host attach attempts the interface made.
    fn connect_attempts(&self) -> usize {
        self.connect_attempts.load(Ordering::Acquire)
    }

    /// Waits until the interface has issued `method`, then returns its payload.
    fn wait_for(&self, method: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(request) = self
                .requests
                .lock()
                .expect("fixture requests")
                .iter()
                .rev()
                .find(|request| request.method == method)
            {
                return request.payload.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the interface never issued {method}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for FixtureHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = std::net::TcpStream::connect(
            self.url.trim_start_matches("http://").trim_end_matches('/'),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One HTTP call, with the RPC envelope when the call carried one.
struct FixtureCall {
    head: String,
    rpc: Option<FixtureRequest>,
}

fn read_call(stream: &std::net::TcpStream) -> Option<FixtureCall> {
    let mut stream = stream;
    use std::io::Read;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = &buffer[header_end..buffer.len().min(header_end + length)];
    let rpc = serde_json::from_slice::<Value>(body).ok().and_then(|envelope| {
        Some(FixtureRequest {
            rpc_id: envelope.get("rpcId")?.as_str()?.to_owned(),
            method: envelope.get("method")?.as_str()?.to_owned(),
            payload: envelope.get("payload").cloned().unwrap_or(Value::Null),
        })
    });
    Some(FixtureCall { head, rpc })
}

fn send_response(
    mut stream: std::net::TcpStream,
    rpc_id: &str,
    reply: Result<Value, (String, String)>,
) {
    use std::io::Write;
    let result = match reply {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err((code, message)) => serde_json::json!({
            "ok": false,
            "error": {"code": code, "message": message, "details": {}},
        }),
    };
    let envelope = serde_json::json!({
        "type": "server-response",
        "rpcId": rpc_id,
        "result": result,
    });
    let body = serde_json::to_vec(&envelope).expect("fixture response");
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

fn send_status(mut stream: std::net::TcpStream, status: u16) {
    use std::io::Write;
    let body = b"unavailable";
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Service Unavailable\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

/// Most events one page returns, matching the viewer's window.
const PAGE_LIMIT: usize = 50;

/// One listed Session row; projection titles keep the identity visible in tests.
fn session_item(session_id: &str, created: Option<()>) -> Value {
    let title = if created.is_some() {
        "Created session"
    } else {
        session_id
    };
    serde_json::json!({
        "agentAvailable": true,
        "sessionId": session_id,
        "updatedAt": 0,
        "running": false,
        "blank": false,
        "projections": {"title": title},
    })
}

/// Builds the shipped app against the fixture Host and mux with deterministic state.
///
/// @returns The app plus the sender that feeds its fixture Remote Stream mux.
fn fixture(host: &FixtureHost) -> (DesktopApp, mpsc::Sender<Value>) {
    let url = &host.url;
    let directory = std::env::temp_dir().join(format!("dsh-kittest-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("fixture preferences directory");
    let mut app = DesktopApp::new(ShellServices {
        quitting: Arc::new(AtomicBool::new(false)),
        show_requested: Arc::new(AtomicBool::new(false)),
        hide_requested: Arc::new(AtomicBool::new(false)),
        hidden: Arc::new(AtomicBool::new(false)),
        viewport: Arc::new(Mutex::new(None)),
        tray_available: false,
        ui_preferences_path: directory.join("ui-preferences.json"),
        spawn_host: false,
    });
    app.transport = Some(HttpTransport::new(url).expect("fixture transport"));
    app.host_url = Some(url.to_owned());
    app.auto_connect = false;
    // Skip the fontconfig scan: the harness asserts layout, not installed fonts.
    app.font_catalog_snapshot = Some(fonts::FontCatalog::default());
    app.sessions = host
        .session_ids()
        .into_iter()
        .map(|session_id| SessionSummary {
            agent_available: true,
            format_status: None,
            projections: Some(serde_json::json!({"title": session_id})),
            updated_at: 0,
            running: false,
            blank: false,
            parent_session_id: None,
            origin: None,
            cwd: None,
            session_id,
        })
        .collect();
    app.session_title = "Fixture session".to_owned();
    app.model_selection = Some(ModelSelection {
        provider: "fixture".to_owned(),
        model: "fixture-model".to_owned(),
        reasoning_effort: None,
    });
    let (frames, receiver) = mpsc::channel();
    app.streams = Some(RemoteStreamMux::loopback(receiver));
    (app, frames)
}

fn user_record(seq: i64, text: &str) -> Value {
    serde_json::json!({
        "type": "user/message",
        "seq": seq,
        "time": 0,
        "data": {"content": [{"type": "text", "text": text}]},
    })
}

fn assistant_record(seq: i64, text: &str) -> Value {
    serde_json::json!({
        "type": "assistant/message",
        "seq": seq,
        "time": 0,
        "data": {"content": [{"type": "text", "text": text}]},
    })
}

fn tool_call_record(seq: i64, name: &str, arguments: &str) -> Value {
    serde_json::json!({
        "type": "assistant/message",
        "seq": seq,
        "time": 0,
        "data": {"content": [{
            "type": "tool-call",
            "name": name,
            "callId": format!("call-{seq}"),
            "arguments": arguments,
        }]},
    })
}

fn deliverable_record(seq: i64, path: &str) -> Value {
    serde_json::json!({
        "type": "deliverables/presented",
        "seq": seq,
        "time": 0,
        "data": {"files": [{"path": path, "description": "history report"}]},
    })
}

fn tool_result_record(seq: i64, text: &str) -> Value {
    serde_json::json!({
        "type": "tool/result",
        "seq": seq,
        "time": 0,
        "data": {"message": {"isError": false, "content": [{"type": "text", "text": text}]}},
    })
}

/// A snapshot carrying backlog records, as `session/follow` sends first.
fn snapshot_with(records: Vec<Value>) -> Value {
    let frames: Vec<Value> = records
        .into_iter()
        .map(|event| serde_json::json!({"type": "event", "event": event}))
        .collect();
    serde_json::json!({
        "type": "snapshot",
        "header": {
            "version": 1,
            "id": "fixture-session",
            "createdAt": 0,
            "isSeeded": false,
        },
        "cursor": 0,
        "records": frames,
        "hasMore": false,
        "projections": {},
    })
}

/// An empty `session/follow` snapshot, which a follow stream sends first.
fn snapshot_frame() -> Value {
    serde_json::json!({
        "type": "snapshot",
        "header": {
            "version": 1,
            "id": "fixture-session",
            "createdAt": 0,
            "isSeeded": false,
        },
        "cursor": 0,
        "records": [],
        "hasMore": false,
        "projections": {},
    })
}

/// One live assistant event, as the follow stream delivers it.
fn assistant_frame(seq: i64, text: &str) -> Value {
    serde_json::json!({
        "type": "event",
        "event": assistant_record(seq, text),
    })
}

/// Opens a Session through the sidebar and lets its load snapshot through.
fn open_session(
    harness: &mut Harness<'_, DesktopApp>,
    frames: &mpsc::Sender<Value>,
    label: &str,
    backlog: Vec<Value>,
    expected_rows: usize,
) {
    harness.get_by_label_contains(label).click();
    harness.run_steps(2);
    // Send unconditionally: the load stream may already have its snapshot from
    // the mux, and a duplicate merge changes nothing.
    frames.send(snapshot_with(backlog)).expect("load snapshot");
    pump(harness, "history renders", |app| {
        app.transcript.rows().len() >= expected_rows
    });
}

/// Runs frames until `condition` holds, so worker threads can settle.
fn pump(
    harness: &mut Harness<'_, DesktopApp>,
    what: &str,
    condition: impl Fn(&DesktopApp) -> bool,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition(harness.state()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the interface never reached: {what}; selected={:?} page_loading={} page_error={:?} rows={:?} error={:?} preview_error={:?}",
            harness.state().selected_session,
            harness.state().page_loading,
            harness.state().page_error,
            harness.state().transcript.rows().iter().map(|row| row.text.clone()).collect::<Vec<_>>(),
            harness.state().sessions_error,
            harness.state().preview_error,
        );
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    harness.run_steps(3);
}

fn harness(app: DesktopApp, width: f32, height: f32) -> Harness<'static, DesktopApp> {
    Harness::builder()
        .with_size(egui::vec2(width, height))
        .build_ui_state(
            |ui, app: &mut DesktopApp| {
                app.tick(ui.ctx());
                app.render(ui);
            },
            app,
        )
}

#[test]
fn semantic_create_compose_send_and_decide() {
    let host = FixtureHost::start();
    host.set_page(
        "fixture-session",
        vec![user_record(1, "Xin chào từ fixture")],
    );
    let (mut app, frames) = fixture(&host);
    let (command_sender, command_receiver) = mpsc::channel();
    app.approval_commands = Some(command_sender);
    let (approval_sender, approval_receiver) = mpsc::channel();
    app.approval_events_receiver = Some(approval_receiver);

    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    // Selecting the fixture Session opens its follow stream and loads the page.
    harness.get_by_label_contains("fixture-session").click();
    harness.run_steps(3);
    // The follow stream opens during load; feed it only once it exists.
    pump(&mut harness, "the Session page starts loading", |app| {
        app.page_loading
    });
    frames.send(snapshot_frame()).expect("follow snapshot");
    pump(&mut harness, "the Session history renders", |app| {
        app.transcript
            .rows()
            .iter()
            .any(|row| row.text.contains("Xin chào từ fixture"))
    });
    // History arrives through the same page payload the shipped shell renders.
    assert!(harness
        .query_by_label_contains("Xin chào từ fixture")
        .is_some());

    // New Session issues the RPC and selects the created Session.
    harness.get_by_label("New Session").click();
    harness.run_steps(3);
    host.wait_for("session/create");
    pump(&mut harness, "the created Session is selected", |app| {
        app.selected_session.as_deref() == Some("fixture-created")
    });

    // Typing reaches the composer buffer; Send issues the prompt with that text.
    let composer = harness.get_by_label("Composer");
    composer.focus();
    composer.type_text("việt nam");
    harness.run_steps(3);
    assert_eq!(harness.state().chat_text, "việt nam");
    harness.get_by_label("Send").click();
    harness.run_steps(3);
    let prompt = host.wait_for("session/prompt");
    let request = &prompt["args"]["request"];
    assert_eq!(request["content"][0]["text"], "việt nam");
    assert_eq!(request["sessionId"], "fixture-created");

    // The follow stream's assistant event reaches the transcript view.
    frames.send(snapshot_frame()).expect("live snapshot");
    frames
        .send(assistant_frame(2, "Phản hồi từ fixture"))
        .expect("assistant event");
    pump(&mut harness, "the assistant reply renders", |app| {
        app.transcript
            .rows()
            .iter()
            .any(|row| row.text.contains("Phản hồi từ fixture"))
    });
    assert!(harness
        .query_by_label_contains("Phản hồi từ fixture")
        .is_some());

    // A Host approval request renders and its decision leaves through the command channel.
    let mut request = serde_json::Map::new();
    request.insert("toolName".to_owned(), serde_json::json!("shell"));
    request.insert(
        "reason".to_owned(),
        serde_json::json!("run the fixture command"),
    );
    approval_sender
        .send(Ok(RemoteEvent::Waterfall(RemoteEventInvocation::for_test(
            "approval/request",
            request,
        ))))
        .expect("approval event");
    pump(&mut harness, "the approval request renders", |app| {
        !app.pending_approvals.is_empty()
    });
    harness.get_by_label("Allow once").click();
    harness.run_steps(3);
    match command_receiver.try_recv().expect("approval command") {
        ApprovalCommand::Answer(pending, decision) => {
            assert_eq!(pending.request.tool_name, "shell");
            assert_eq!(decision, ApprovalDecision::AllowedOnce);
        }
        ApprovalCommand::Delegate(_) | ApprovalCommand::AnswerQuestion(_, _) => {
            panic!("unexpected approval command")
        }
    }
    assert!(harness.state().pending_approvals.is_empty());
    assert!(harness.query_by_label("Allow once").is_none());

    // The same card offers the deny decision, which leaves as Rejected.
    let mut denied = serde_json::Map::new();
    denied.insert("toolName".to_owned(), serde_json::json!("shell"));
    approval_sender
        .send(Ok(RemoteEvent::Waterfall(RemoteEventInvocation::for_test(
            "approval/request",
            denied,
        ))))
        .expect("second approval event");
    pump(&mut harness, "the second approval renders", |app| {
        !app.pending_approvals.is_empty()
    });
    harness.get_by_label("Reject").click();
    harness.run_steps(3);
    match command_receiver.try_recv().expect("deny command") {
        ApprovalCommand::Answer(pending, decision) => {
            assert_eq!(pending.request.tool_name, "shell");
            assert_eq!(decision, ApprovalDecision::Rejected);
        }
        ApprovalCommand::Delegate(_) | ApprovalCommand::AnswerQuestion(_, _) => {
            panic!("unexpected deny command")
        }
    }
}

#[test]
fn opening_an_old_session_projects_history_into_the_transcript() {
    let host = FixtureHost::start();
    host.set_page(
        "old-session",
        vec![
            user_record(1, "Câu hỏi cũ"),
            tool_call_record(2, "shell", "{\"command\":\"ls\"}"),
            tool_result_record(3, "ls output"),
            assistant_record(4, "Câu trả lời cũ"),
            deliverable_record(5, "reports/history.md"),
        ],
    );
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    open_session(&mut harness, &frames, "old-session", Vec::new(), 5);

    let rows = harness.state().transcript.rows();
    assert_eq!(
        rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5]
    );
    assert!(rows.iter().any(|row| row.kind == RowKind::User));
    assert!(rows.iter().any(|row| row.kind == RowKind::Assistant));
    assert!(rows
        .iter()
        .any(|row| row.kind == RowKind::ToolResult && row.text.contains("ls output")));
    // The history is on screen through the same renderer live events use.
    assert!(harness.query_by_label_contains("Câu trả lời cũ").is_some());
    assert!(harness.query_by_label_contains("ls output").is_some());
    // History deliverables reach the panel too, not just live frames.
    assert_eq!(harness.state().deliverables.len(), 1);
    assert!(
        harness
            .query_all_by_label_contains("reports/history.md")
            .count()
            >= 1
    );
}

#[test]
fn a_stream_snapshot_during_load_merges_in_sequence_order() {
    let host = FixtureHost::start();
    host.set_page(
        "interleaved",
        vec![user_record(1, "một"), assistant_record(3, "ba")],
    );
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    // The follow snapshot arrives while the history page is in flight and
    // carries events the page does not (2 and 4), so both sources must merge
    // into one ordered log.
    open_session(
        &mut harness,
        &frames,
        "interleaved",
        vec![user_record(2, "hai"), assistant_record(4, "bốn")],
        4,
    );

    let rows = harness.state().transcript.rows();
    assert_eq!(
        rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(
        rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>(),
        ["một", "hai", "ba", "bốn"]
    );
}

#[test]
fn switching_sessions_replaces_the_transcript() {
    let host = FixtureHost::start();
    host.set_page(
        "session-a",
        vec![user_record(1, "A một"), assistant_record(2, "A hai")],
    );
    host.set_page(
        "session-b",
        vec![user_record(10, "B một"), assistant_record(11, "B hai")],
    );
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    open_session(&mut harness, &frames, "session-a", Vec::new(), 2);
    assert!(harness.query_by_label_contains("A hai").is_some());

    open_session(&mut harness, &frames, "session-b", Vec::new(), 2);
    let rows = harness.state().transcript.rows();
    assert_eq!(rows.iter().map(|row| row.seq).collect::<Vec<_>>(), [10, 11]);
    assert!(harness.query_by_label_contains("B hai").is_some());
    assert!(harness.query_by_label_contains("A hai").is_none());
}

#[test]
fn long_history_keeps_the_newest_rows_in_order() {
    let host = FixtureHost::start();
    host.set_page_limit(5_000);
    host.set_page(
        "long-session",
        (1..=2_100)
            .map(|seq| user_record(seq, &format!("row {seq}")))
            .collect(),
    );
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    open_session(&mut harness, &frames, "long-session", Vec::new(), 2_000);

    let rows = harness.state().transcript.rows();
    assert_eq!(rows.len(), crate::transcript::MAX_ROWS);
    assert_eq!(rows.first().map(|row| row.seq), Some(101));
    assert_eq!(rows.last().map(|row| row.seq), Some(2_100));
    assert!(rows.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

#[test]
fn pagination_prepends_older_history_without_duplicates() {
    let host = FixtureHost::start();
    host.set_page(
        "paged-session",
        (1..=120)
            .map(|seq| user_record(seq, &format!("row {seq}")))
            .collect(),
    );
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    open_session(&mut harness, &frames, "paged-session", Vec::new(), 50);
    assert_eq!(
        harness.state().transcript.oldest_seq(),
        Some(71),
        "the first page carries the newest window"
    );

    for expected_rows in [100_usize, 120] {
        harness.get_by_label("Load older events").click();
        harness.run_steps(3);
        pump(&mut harness, "older history merges", |app| {
            app.transcript.rows().len() == expected_rows
        });
    }
    let rows = harness.state().transcript.rows();
    assert_eq!(rows.first().map(|row| row.seq), Some(1));
    assert_eq!(rows.last().map(|row| row.seq), Some(120));
    assert!(rows.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

#[test]
fn layout_stays_inside_three_viewports() {
    let host = FixtureHost::start();
    let mut report = serde_json::Map::new();
    let mut viewports = Vec::new();
    for (width, height) in [(1280.0_f32, 860.0_f32), (1440.0, 900.0), (1920.0, 1080.0)] {
        host.set_page("fixture-session", vec![user_record(1, "layout")]);
        let (mut app, _frames) = fixture(&host);
        app.selected_session = Some("fixture-session".to_owned());
        let mut harness = harness(app, width, height);
        harness.run_ok();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        harness.get_by_label("Model");
        let composer = harness.get_by_label("Composer").rect();
        let send = harness.get_by_label("Send").rect();
        let new_session = harness.get_by_label("New Session").rect();
        assert!(
            viewport.contains_rect(composer),
            "composer escaped {width}x{height}: {composer:?}"
        );
        assert!(
            viewport.contains_rect(send),
            "Send escaped {width}x{height}"
        );
        assert!(
            viewport.contains_rect(new_session),
            "New Session escaped {width}x{height}"
        );
        assert!(
            composer.width() > 200.0,
            "composer collapsed at {width}x{height}: {composer:?}"
        );
        viewports.push(serde_json::json!({
            "width": width,
            "height": height,
            "composer": [composer.min.x, composer.min.y, composer.width(), composer.height()],
            "send": [send.min.x, send.min.y, send.width(), send.height()],
            "newSession": [new_session.min.x, new_session.min.y, new_session.width(), new_session.height()],
        }));
    }
    report.insert("viewports".to_owned(), Value::Array(viewports));
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/dsh-kittest");
    std::fs::create_dir_all(&directory).expect("report directory");
    let path = directory.join("layout.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&Value::Object(report)).expect("layout report"),
    )
    .expect("layout report write");
    assert!(path.exists(), "machine-readable layout report missing");
}

/// Snapshots the empty shell at one viewport; one harness per test keeps the
/// snapshot results in a single owner.
fn snapshot_empty_shell(width: f32, height: f32, name: &str) {
    let host = FixtureHost::start();
    host.set_page("fixture-session", Vec::new());
    let (mut app, _frames) = fixture(&host);
    app.sessions.clear();
    app.selected_session = None;
    app.transcript = Transcript::default();
    app.model_selection = None;
    let mut harness = harness(app, width, height);
    harness.run_ok();
    harness.snapshot(name);
}

#[test]
fn snapshot_empty_shell_1280() {
    snapshot_empty_shell(1280.0, 860.0, "shell-1280x860");
}

#[test]
fn snapshot_empty_shell_1920() {
    snapshot_empty_shell(1920.0, 1080.0, "shell-1920x1080");
}

#[test]
fn snapshot_empty_shell_1440() {
    snapshot_empty_shell(1440.0, 900.0, "shell-1440x900");
}

/// One redacted settings namespace the fixture Host reports.
fn settings_namespace() -> Value {
    serde_json::json!({
        "autoGenerate": false,
        "ns": "fixture.ns",
        "schema": {"type": "object"},
        "value": {"approval": "ask"},
        "base": {"approval": "ask"},
        "user": {"approval": "ask"},
        "applies": "user",
        "secrets": [],
        "revision": 3,
    })
}

/// A one-page PDF whose raster size is deterministic at 110 dpi.
fn minimal_pdf() -> Vec<u8> {
    br#"%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]/Contents 4 0 R>>endobj
4 0 obj<</Length 23>>stream
0 0 1 RG 10 10 50 50 re f
endstream
endobj
trailer<</Root 1 0 R>>
%%EOF
"#
    .to_vec()
}

/// Scratch path used by the Office preview worker for one Host path.
fn office_preview_scratch(path: &str) -> std::path::PathBuf {
    let fingerprint = path
        .bytes()
        .fold(0_u64, |hash, byte| hash.wrapping_mul(31).wrapping_add(u64::from(byte)));
    std::env::temp_dir().join(format!(
        "dsh-office-preview-{}-{fingerprint:x}",
        std::process::id()
    ))
}

/// Opens Settings, expands the Host section, and waits for its description.
fn open_host_settings(harness: &mut Harness<'_, DesktopApp>) {
    harness.get_by_label("Settings").click_accesskit();
    harness.run_steps(3);
    harness
        .get_by_label_contains("Host settings (agent and plugin configuration)")
        .click_accesskit();
    pump(harness, "the Host settings load", |app| {
        app.settings_description.is_some()
    });
}

/// Opens the draft editor and replaces its JSON with `text`.
fn edit_settings_json(harness: &mut Harness<'_, DesktopApp>, text: &str) {
    harness.get_by_label("Edit user overrides").click_accesskit();
    harness.run_steps(2);
    let editor = harness.get_by_label("Host settings JSON");
    editor.focus();
    harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]);
    editor.type_text(text);
    harness.run_steps(3);
    assert_eq!(
        harness
            .get_by_label("Host settings JSON")
            .value()
            .as_deref(),
        Some(text),
        "the draft editor must hold exactly the replacement text"
    );
}

#[test]
fn settings_write_reaches_the_host_and_reads_back() {
    let host = FixtureHost::start();
    host.set_settings_namespace(settings_namespace());
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);

    open_host_settings(&mut harness);
    assert_eq!(
        host.wait_for("settings/describe")["args"],
        serde_json::json!({})
    );

    // The draft is user overrides only; saving turns the change into a path op.
    edit_settings_json(&mut harness, r#"{"approval": "never"}"#);
    harness.get_by_label("Save settings").click_accesskit();
    harness.run_steps(3);
    let payload = host.wait_for("settings/mutate")["args"].clone();
    assert_eq!(payload["ns"], "fixture.ns");
    assert_eq!(payload["expectedRevision"], 3);
    assert_eq!(
        payload["ops"],
        serde_json::json!([{"op": "set", "path": ["approval"], "value": "never"}])
    );

    // The read-back is the Host's new revision, not the draft.
    pump(&mut harness, "the saved settings read back", |app| {
        app.settings_description
            .as_ref()
            .is_some_and(|description| {
                description.namespaces.first().is_some_and(|ns| ns.revision == 4)
            })
    });
    let description = harness
        .state()
        .settings_description
        .as_ref()
        .expect("settings description");
    assert_eq!(
        description.namespaces[0].user,
        Some(serde_json::json!({"approval": "never"}))
    );
    assert!(harness.query_by_label_contains("\"never\"").is_some());
    assert!(harness.query_by_label_contains("Settings saved.").is_some());
    // A successful save closes the draft.
    assert!(harness.query_by_label("Host settings JSON").is_none());
}

#[test]
fn settings_write_conflict_keeps_the_draft_and_reports_it() {
    let host = FixtureHost::start();
    host.set_settings_namespace(settings_namespace());
    host.fail_settings_write();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);

    open_host_settings(&mut harness);
    edit_settings_json(&mut harness, r#"{"approval": "never"}"#);
    harness.get_by_label("Save settings").click_accesskit();
    harness.run_steps(3);
    host.wait_for("settings/mutate");

    pump(&mut harness, "the conflict is reported", |app| {
        app.settings_update_receiver.is_none()
    });
    assert!(harness
        .query_by_label_contains("Settings conflict: another edit changed this namespace.")
        .is_some());
    // The Host value did not move, and the draft is still on screen.
    let description = harness
        .state()
        .settings_description
        .as_ref()
        .expect("settings description");
    assert_eq!(description.namespaces[0].revision, 3);
    assert_eq!(
        description.namespaces[0].user,
        Some(serde_json::json!({"approval": "ask"}))
    );
    let editor = harness
        .query_by_label("Host settings JSON")
        .expect("draft retained");
    assert!(editor.value().is_some_and(|text| text.contains("never")));
}

#[test]
fn marketplace_search_and_install_report_the_host_result() {
    let host = FixtureHost::start();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);

    // Opening the tab asks the Host for its configured indexes.
    harness.get_by_label("Marketplace").click_accesskit();
    const INDEX: &str = "https://market.fixture/index.json";
    pump(&mut harness, "market indexes load", |app| {
        app.market_indexes
            .iter()
            .any(|index| index["url"] == INDEX)
    });
    assert_eq!(
        host.wait_for("pluginMarket/listIndexes")["args"],
        serde_json::json!({})
    );

    let search = harness.get_by_label("Marketplace search");
    search.focus();
    search.type_text("fixture");
    harness.run_steps(2);
    harness.get_by_label("Search marketplace").click_accesskit();
    harness.run_steps(3);
    let payload = host.wait_for("pluginMarket/search")["args"].clone();
    assert_eq!(payload["url"], INDEX);
    assert_eq!(payload["query"]["text"], "fixture");
    pump(&mut harness, "market results render", |app| {
        !app.market_results.is_empty()
    });
    assert!(harness
        .query_by_label_contains("fixture-plugin")
        .is_some());

    // Install inspects the exact spec, then installs it and reports the result.
    harness.get_by_label("Install").click_accesskit();
    harness.run_steps(3);
    let inspect = host.wait_for("pluginManager/inspect")["args"].clone();
    assert_eq!(inspect["spec"], "fixture-plugin@1.0.0");
    let install = host.wait_for("pluginManager/installBundle")["args"].clone();
    assert_eq!(install["spec"], "fixture-plugin@1.0.0");
    assert_eq!(install["options"], serde_json::json!({}));
    pump(&mut harness, "the install result renders", |app| {
        app.market_status
            .as_deref()
            .is_some_and(|status| status.contains("-> installed"))
    });
    assert!(harness
        .query_by_label_contains("fixture-plugin@1.0.0 -> installed")
        .is_some());
}

#[test]
fn marketplace_install_failure_is_reported_not_claimed() {
    let host = FixtureHost::start();
    host.fail_install();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);

    harness.get_by_label("Marketplace").click_accesskit();
    pump(&mut harness, "market indexes load", |app| {
        !app.market_indexes.is_empty()
    });
    harness.get_by_label("Search marketplace").click_accesskit();
    pump(&mut harness, "market results render", |app| {
        !app.market_results.is_empty()
    });
    harness.get_by_label("Install").click_accesskit();
    harness.run_steps(3);
    host.wait_for("pluginManager/inspect");
    host.wait_for("pluginManager/installBundle");

    pump(&mut harness, "the install failure is reported", |app| {
        app.market_status
            .as_deref()
            .is_some_and(|status| status.starts_with("Marketplace failed:"))
    });
    assert!(harness.query_by_label_contains("Marketplace failed:").is_some());
    assert!(harness
        .query_by_label_contains("-> installed")
        .is_none());
}

#[test]
fn retry_connection_attempts_a_host_attach_and_stays_disconnected() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    // The disconnected state the shell shows after the Host control channel closes.
    app.transport = None;
    app.streams = None;
    app.sessions.clear();
    app.host = Some(Host::for_test(&format!("{}?token=fixture", host.url)));
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);

    assert!(harness.query_by_label("Retry connection").is_some());
    harness.get_by_label("Retry connection").click_accesskit();
    pump(&mut harness, "the attach attempt fails", |app| {
        app.status.starts_with("Authentication failed")
    });
    // The click issued a real Host handshake, and a refusal leaves no transport.
    assert_eq!(host.connect_attempts(), 1);
    assert!(harness.state().transport.is_none());
    assert!(harness.query_by_label("Retry connection").is_some());

    // A second click is another attempt rather than a dead control.
    harness.get_by_label("Retry connection").click_accesskit();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while host.connect_attempts() < 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "the second attach attempt never happened"
        );
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn office_preview_renders_the_converted_pdf() {
    let host = FixtureHost::start();
    host.set_page("fixture-session", Vec::new());
    host.set_office_pdf(minimal_pdf());
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);
    harness.get_by_label_contains("fixture-session").click();
    harness.run_steps(3);
    frames.send(snapshot_frame()).expect("session snapshot");
    pump(&mut harness, "the Session opens", |app| {
        app.selected_session.as_deref() == Some("fixture-session")
    });

    harness.get_by_label("Document").click_accesskit();
    harness.run_steps(2);
    let path = harness.get_by_label("Document path");
    path.focus();
    path.type_text("out/report.docx");
    harness.run_steps(2);
    harness.get_by_label("Preview").click_accesskit();
    harness.run_steps(3);

    let payload = host.wait_for("officeToPdf/render")["args"].clone();
    assert_eq!(payload["path"], "out/report.docx");
    assert_eq!(payload["priority"], "foreground");
    assert_eq!(payload["workspaceFileScopeId"], "fixture-session");

    // The local raster shows the Host-converted PDF, not a placeholder.
    pump(&mut harness, "the PDF page rasterizes", |app| {
        app.preview_image.is_some()
    });
    let image = harness.state().preview_image.as_ref().expect("preview");
    assert_eq!(image.size, [306, 153]);
    assert_eq!(image.name, "report.docx");
    assert!(harness
        .query_by_label_contains("Image: report.docx")
        .is_some());
    assert!(harness.query_by_label_contains("306 × 153 px").is_some());
    assert!(!office_preview_scratch("out/report.docx").exists());
}

#[test]
fn office_preview_reports_a_conversion_that_is_not_a_pdf() {
    let host = FixtureHost::start();
    host.set_page("fixture-session", Vec::new());
    host.set_office_pdf(b"not a pdf".to_vec());
    let (app, frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(3);
    harness.get_by_label_contains("fixture-session").click();
    harness.run_steps(3);
    frames.send(snapshot_frame()).expect("session snapshot");
    pump(&mut harness, "the Session opens", |app| {
        app.selected_session.as_deref() == Some("fixture-session")
    });

    harness.get_by_label("Document").click_accesskit();
    harness.run_steps(2);
    let path = harness.get_by_label("Document path");
    path.focus();
    path.type_text("out/broken.docx");
    harness.run_steps(2);
    harness.get_by_label("Preview").click_accesskit();
    harness.run_steps(3);
    host.wait_for("officeToPdf/render");

    pump(&mut harness, "the raster failure is reported", |app| {
        app.preview_error.is_some()
    });
    assert!(harness.state().preview_image.is_none());
    assert!(harness
        .query_by_label_contains("Document read failed:")
        .is_some());
    assert!(!office_preview_scratch("out/broken.docx").exists());
}
