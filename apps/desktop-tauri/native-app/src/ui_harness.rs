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
use std::net::TcpListener;

/// One RPC the interface issued, as the fixture Host received it.
#[derive(Debug, Clone)]
struct FixtureRequest {
    rpc_id: String,
    method: String,
    payload: Value,
}

/// In-process Host that answers the RPCs a windowless test needs.
struct FixtureHost {
    url: String,
    requests: Arc<Mutex<Vec<FixtureRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
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
        let stop = Arc::new(AtomicBool::new(false));
        let worker_requests = Arc::clone(&requests);
        let worker_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            // A created Session joins later listings, as a real Host would report.
            let mut created: Vec<String> = Vec::new();
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => match read_request(&stream) {
                        Some(request) => {
                            let reply = response_value(&request.method, &mut created);
                            let rpc_id = request.rpc_id.clone();
                            worker_requests
                                .lock()
                                .expect("fixture requests")
                                .push(request);
                            send_response(stream, &rpc_id, reply);
                        }
                        None => drop(stream),
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
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

fn read_request(stream: &std::net::TcpStream) -> Option<FixtureRequest> {
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
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
    let length: usize = headers
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
    let envelope: Value = serde_json::from_slice(body).ok()?;
    Some(FixtureRequest {
        rpc_id: envelope.get("rpcId")?.as_str()?.to_owned(),
        method: envelope.get("method")?.as_str()?.to_owned(),
        payload: envelope.get("payload").cloned().unwrap_or(Value::Null),
    })
}

fn send_response(mut stream: std::net::TcpStream, rpc_id: &str, value: Value) {
    use std::io::Write;
    let envelope = serde_json::json!({
        "type": "server-response",
        "rpcId": rpc_id,
        "result": {"ok": true, "value": value},
    });
    let body = serde_json::to_vec(&envelope).expect("fixture response");
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

/// Deterministic wire reply for one method; unknown methods return `null`.
fn response_value(method: &str, created: &mut Vec<String>) -> Value {
    match method {
        "session/list" => {
            let mut items = vec![serde_json::json!({
                "agentAvailable": true,
                "sessionId": "fixture-session",
                "updatedAt": 0,
                "running": false,
                "blank": false,
                "projections": {"title": "Fixture session"},
            })];
            items.extend(created.iter().map(|id| {
                serde_json::json!({
                    "agentAvailable": true,
                    "sessionId": id,
                    "updatedAt": 0,
                    "running": false,
                    "blank": false,
                    "projections": {"title": "Created session"},
                })
            }));
            serde_json::json!({"items": items})
        }
        "session/create" => {
            created.push("fixture-created".to_owned());
            serde_json::json!({
                "sessionId": "fixture-created",
                "agentPreset": null,
            })
        }
        "session/prompt" => serde_json::json!({"accepted": true}),
        "session/page" => serde_json::json!({
            "records": [
                {"type": "event", "event": user_record(1, "Xin chào từ fixture")},
            ],
            "hasMore": false,
        }),
        "model/catalog" => serde_json::json!({
            "default": {"provider": "fixture", "model": "fixture-model"},
            "routableProviders": ["fixture"],
            "groups": [{
                "id": "fixture",
                "name": "Fixture provider",
                "models": [{"id": "fixture-model", "name": "Fixture model"}],
            }],
            "failures": [],
        }),
        _ => Value::Null,
    }
}

/// Builds the shipped app against the fixture Host and mux with deterministic state.
///
/// @returns The app plus the sender that feeds its fixture Remote Stream mux.
fn fixture(url: &str) -> (DesktopApp, mpsc::Sender<Value>) {
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
    app.sessions = vec![SessionSummary {
        agent_available: true,
        format_status: None,
        session_id: "fixture-session".to_owned(),
        updated_at: 0,
        running: false,
        blank: false,
        parent_session_id: None,
        origin: None,
        cwd: None,
        projections: Some(serde_json::json!({"title": "Fixture session"})),
    }];
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
            "the interface never reached: {what}; selected={:?} page_loading={} page_error={:?} rows={:?} error={:?}",
            harness.state().selected_session,
            harness.state().page_loading,
            harness.state().page_error,
            harness.state().transcript.rows().iter().map(|row| row.text.clone()).collect::<Vec<_>>(),
            harness.state().sessions_error,
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
    let (mut app, frames) = fixture(&host.url);
    let (command_sender, command_receiver) = mpsc::channel();
    app.approval_commands = Some(command_sender);
    let (approval_sender, approval_receiver) = mpsc::channel();
    app.approval_events_receiver = Some(approval_receiver);

    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    // Selecting the fixture Session opens its follow stream and loads the page.
    harness.get_by_label_contains("Fixture session").click();
    harness.run_steps(3);
    // The follow stream opens during load; feed it only once it exists.
    pump(&mut harness, "the Session page starts loading", |app| {
        app.page_loading
    });
    frames.send(snapshot_frame()).expect("follow snapshot");
    pump(&mut harness, "the Session page renders", |app| {
        app.session_page.is_some()
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
fn layout_stays_inside_three_viewports() {
    let host = FixtureHost::start();
    let mut report = serde_json::Map::new();
    let mut viewports = Vec::new();
    for (width, height) in [(1280.0_f32, 860.0_f32), (1440.0, 900.0), (1920.0, 1080.0)] {
        let (mut app, _frames) = fixture(&host.url);
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
    let (mut app, _frames) = fixture(&host.url);
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
