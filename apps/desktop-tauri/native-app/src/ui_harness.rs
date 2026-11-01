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

mod audit;

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
    fail_model_initialization: bool,
    has_model_provider: bool,
    fail_install: bool,
    plugin_enabled: bool,
    schedule_deleted: bool,
    goal: Option<Value>,
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
            fail_model_initialization: false,
            has_model_provider: true,
            fail_install: false,
            plugin_enabled: true,
            schedule_deleted: false,
            goal: None,
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
                let mut items: Vec<Value> =
                    self.pages.keys().map(|id| session_item(id, None)).collect();
                items.extend(self.created.iter().map(|id| session_item(id, Some(()))));
                Ok(serde_json::json!({"items": items}))
            }
            "session/search" => Ok(serde_json::json!({
                "items": [{
                    "sessionId": "search-match",
                    "snippet": "matched fixture message",
                }],
                "hasMore": false,
            })),
            "workspaceFiles/list" => {
                let path = payload["path"].as_str().unwrap_or(".");
                match path {
                    "." => Ok(serde_json::json!({
                        "path": "",
                        "entries": [
                            {"name": "src", "type": "directory"},
                            {"name": "README.md", "type": "file", "size": 29},
                            {"name": "image.png", "type": "file", "size": 128},
                        ],
                        "truncated": false,
                    })),
                    "src" => Ok(serde_json::json!({
                        "path": "src",
                        "entries": [{"name": "main.rs", "type": "file", "size": 27}],
                        "truncated": false,
                    })),
                    _ => Err((
                        "workspace-file/not-found".to_owned(),
                        format!("no fixture directory at {path}"),
                    )),
                }
            }
            "workspaceFiles/read" => match payload["path"].as_str().unwrap_or_default() {
                "README.md" => Ok(serde_json::json!({
                    "absolutePath": "/fixture/workspace/README.md",
                    "version": "readme-v1",
                    "bytes": 29,
                    "offset": 1,
                    "text": "fixture read-only source\n",
                    "lines": 1,
                    "eof": true,
                })),
                "src/main.rs" => Ok(serde_json::json!({
                    "absolutePath": "/fixture/workspace/src/main.rs",
                    "version": "main-v1",
                    "bytes": 27,
                    "offset": 1,
                    "text": "fn fixture_source() {}\n",
                    "lines": 1,
                    "eof": true,
                })),
                "image.png" => Err((
                    "workspace-file/not-text".to_owned(),
                    "file contains binary data".to_owned(),
                )),
                path => Err((
                    "workspace-file/not-found".to_owned(),
                    format!("no fixture file at {path}"),
                )),
            },
            "terminal/environment" => Ok(serde_json::json!({
                "cwd": "/fixture/workspace",
                "maxInputBytes": 1048576,
                "maxBufferedBytes": 4194304,
                "maxCols": 240,
                "maxRows": 80,
                "scrollback": 5000,
            })),
            "terminal/shells" => Ok(serde_json::json!([{
                "path": "/bin/sh",
                "args": [],
                "name": "sh",
            }])),
            "terminal/list" => Ok(serde_json::json!([])),
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
            "session/modelCatalog" => {
                if self.has_model_provider {
                    Ok(serde_json::json!({
                        "default": {"provider": "fixture", "model": "fixture-model"},
                        "routableProviders": ["fixture"],
                        "groups": [{
                            "id": "fixture",
                            "name": "Fixture provider",
                            "models": [{"id": "fixture-model", "name": "Fixture model"}],
                        }],
                        "failures": [],
                    }))
                } else {
                    Ok(serde_json::json!({
                        "default": {"provider": "", "model": ""},
                        "routableProviders": [],
                        "groups": [],
                        "failures": [],
                    }))
                }
            }
            "session/initializeDefaultModel" => {
                if self.fail_model_initialization {
                    Err((
                        "gateway/internal".to_owned(),
                        "fixture default-model initialization failure".to_owned(),
                    ))
                } else {
                    Ok(Value::Null)
                }
            }
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
                "versions": [{"version": "1.0.0", "install": {
                    "spec": "fixture-plugin@1.0.0",
                    "registry": "http://market.fixture/registry/",
                }}],
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
            "pluginInventory/list" => Ok(
                serde_json::json!({"managementAvailable": true, "entries": [{
                    "entryId": "fixture-entry",
                    "moduleName": "fixture-module",
                    "enabled": self.plugin_enabled,
                    "fiberPhase": if self.plugin_enabled { "active" } else { "pending" },
                }]}),
            ),
            "pluginManager/listPlugins" => Ok(serde_json::json!([{
                "entryId": "fixture-entry",
                "moduleName": "fixture-module",
                "enabled": self.plugin_enabled,
                "fiberPhase": if self.plugin_enabled { "active" } else { "pending" },
                "patchId": "fixture-plugin-row",
            }])),
            "pluginManager/listBundles" => Ok(serde_json::json!([])),
            "pluginManager/setPluginEnabled" => {
                self.plugin_enabled = payload["enabled"].as_bool().unwrap_or(self.plugin_enabled);
                Ok(serde_json::json!({
                    "changed": true,
                    "application": "applied",
                    "stage": "enable",
                    "target": "fixture-entry",
                    "enabled": self.plugin_enabled,
                }))
            }
            "goals/get" => Ok(self.goal.clone().unwrap_or(Value::Null)),
            "goals/create" => {
                let request = &payload["request"];
                let revision = 1;
                self.goal = Some(serde_json::json!({
                    "id": "fixture-goal",
                    "revision": revision,
                    "objective": request["objective"],
                    "phase": "active",
                    "blockedReason": null,
                    "maxGoalRounds": request["maxGoalRounds"].as_u64().unwrap_or(5),
                    "roundsStarted": 0,
                    "createdAt": 1,
                    "updatedAt": 1,
                    "activation": "armed",
                }));
                Ok(serde_json::json!({"ref":{"id":"fixture-goal","revision":revision}}))
            }
            "goals/edit" => {
                if let Some(goal) = self.goal.as_mut() {
                    if let Some(objective) = payload["request"]["objective"].as_str() {
                        goal["objective"] = serde_json::json!(objective);
                    }
                    if let Some(rounds) = payload["request"]["maxGoalRounds"].as_u64() {
                        goal["maxGoalRounds"] = serde_json::json!(rounds);
                    }
                    goal["revision"] =
                        serde_json::json!(goal["revision"].as_u64().unwrap_or(0) + 1);
                    goal["updatedAt"] = serde_json::json!(2);
                    Ok(goal.clone())
                } else {
                    Err(("goal/missing".to_owned(), "no goal exists".to_owned()))
                }
            }
            "goals/pause" | "goals/resume" | "goals/complete" => {
                if let Some(goal) = self.goal.as_mut() {
                    let (phase, activation) = match method {
                        "goals/pause" => ("paused", "disarmed"),
                        "goals/resume" => ("active", "armed"),
                        _ => ("complete", "disarmed"),
                    };
                    goal["phase"] = serde_json::json!(phase);
                    goal["activation"] = serde_json::json!(activation);
                    goal["revision"] =
                        serde_json::json!(goal["revision"].as_u64().unwrap_or(0) + 1);
                    goal["updatedAt"] = serde_json::json!(2);
                    Ok(goal.clone())
                } else {
                    Err(("goal/missing".to_owned(), "no goal exists".to_owned()))
                }
            }
            "goals/clear" => {
                let revision = self
                    .goal
                    .as_ref()
                    .and_then(|goal| goal["revision"].as_u64())
                    .unwrap_or(0)
                    + 1;
                self.goal = None;
                Ok(serde_json::json!({"id":"fixture-goal","revision":revision}))
            }
            "schedule/list" => Ok(if self.schedule_deleted {
                serde_json::json!([])
            } else {
                serde_json::json!([{
                    "id": "fixture-schedule",
                    "title": "Workspace summary",
                    "kind": "at",
                    "scheduledAt": "2026-10-27T09:00:00Z",
                }])
            }),
            "schedule/delete" => {
                self.schedule_deleted = true;
                Ok(serde_json::json!({"id":"fixture-schedule","deleted":true}))
            }
            "skills/list" => Ok(serde_json::json!({"skills": [
                {
                    "name": "Code review",
                    "description": "Review a focused change against repository conventions.",
                    "whenToUse": "When reviewing a proposed code change.",
                    "modelInvocable": true,
                    "path": ".agents/skills/code-review/SKILL.md",
                },
                {
                    "name": "Debug inference",
                    "description": "Diagnose provider and model-server connectivity.",
                    "whenToUse": "When inference cannot reach its attached provider.",
                    "modelInvocable": true,
                    "path": ".agents/skills/debug-inference/SKILL.md",
                }
            ]})),
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
    set_path(
        map.entry((*head).to_owned())
            .or_insert(serde_json::json!({})),
        rest,
        value,
    );
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
    /// Host update-flow endpoints the interface opened.
    update_open_requests: Arc<AtomicUsize>,
    fail_update_open: Arc<AtomicBool>,
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
        let update_open_requests = Arc::new(AtomicUsize::new(0));
        let fail_update_open = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(FixtureState::default()));
        let worker_requests = Arc::clone(&requests);
        let worker_attempts = Arc::clone(&connect_attempts);
        let worker_updates = Arc::clone(&update_open_requests);
        let worker_fail_update = Arc::clone(&fail_update_open);
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
                if call.head.starts_with("post /__dsh-desktop/updates-open") {
                    worker_updates.fetch_add(1, Ordering::AcqRel);
                    if worker_fail_update.load(Ordering::Acquire) {
                        send_status(stream, 503);
                    } else {
                        send_no_content(stream);
                    }
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
            update_open_requests,
            fail_update_open,
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

    fn set_goal(&self, goal: Option<Value>) {
        self.state.lock().expect("fixture state").goal = goal;
    }

    /// Publishes the redacted settings namespace `settings/describe` returns.
    fn set_settings_namespace(&self, namespace: Value) {
        self.state.lock().expect("fixture state").settings = Some(namespace);
    }

    /// Makes the next settings write fail with a revision conflict.
    fn fail_settings_write(&self) {
        self.state
            .lock()
            .expect("fixture state")
            .fail_settings_write = true;
    }

    /// Makes profile-default initialization fail before changing model state.
    fn fail_model_initialization(&self) {
        self.state
            .lock()
            .expect("fixture state")
            .fail_model_initialization = true;
    }

    /// Makes the Host report an empty routable model catalog.
    fn remove_model_providers(&self) {
        self.state.lock().expect("fixture state").has_model_provider = false;
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

    /// Fails the Host update bridge with an HTTP error.
    fn fail_update_open(&self) {
        self.fail_update_open.store(true, Ordering::Release);
    }

    /// Number of update-flow bridge calls received.
    fn update_open_requests(&self) -> usize {
        self.update_open_requests.load(Ordering::Acquire)
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

    /// Waits for the `index`th request of one method in this fixture.
    fn wait_for_nth(&self, method: &str, index: usize) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let request = self
                .requests
                .lock()
                .expect("fixture requests")
                .iter()
                .filter(|request| request.method == method)
                .nth(index)
                .cloned();
            if let Some(request) = request {
                return request.payload;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the interface never issued request {index} for {method}"
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
    let rpc = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|envelope| {
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

fn send_no_content(mut stream: std::net::TcpStream) {
    use std::io::Write;
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n");
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
            updated_at: now_millis(),
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
    app.refresh_models();
    (app, frames)
}

#[test]
fn ready_host_is_authenticated_automatically_by_default() {
    assert!(auto_connect_enabled(None));
    assert!(!auto_connect_enabled(Some("0")));
    let host = FixtureHost::start();
    let (mut app, _) = fixture(&host);
    app.transport = None;
    app.streams = None;
    app.host_url = None;
    app.host = None;
    app.auto_connect = auto_connect_enabled(None);
    let (ready_sender, ready_receiver) = mpsc::channel();
    app.host_receiver = Some(ready_receiver);
    ready_sender
        .send(Host::for_test(&format!("{}?token=fixture", host.url)))
        .expect("send Host ready");

    let mut harness = harness(app, 1024.0, 768.0);
    pump(
        &mut harness,
        "automatic Host authentication attempt",
        |_| host.connect_attempts.load(Ordering::Acquire) == 1,
    );
    pump(&mut harness, "Host authentication settles", |app| {
        app.connect_receiver.is_none()
    });

    assert!(harness.state().status.starts_with("Authentication failed:"));
    assert_eq!(host.connect_attempts.load(Ordering::Acquire), 1);
}

/// Builds the real panel renderer over an authenticated packaged Host.
fn packaged_host_app(session_id: Option<&str>) -> DesktopApp {
    let host = Host::start();
    let launch_url = host
        .launch_url()
        .expect("packaged Host launch URL")
        .to_owned();
    let (base_url, transport) =
        HttpTransport::authenticate(&launch_url).expect("authenticate with packaged Host");
    let credential = transport.credential().expect("Host API credential");
    let streams = RemoteStreamMux::connect(transport.base_url().as_str(), credential)
        .expect("connect to packaged Host stream mux");
    let directory = std::env::var_os("DSH_HOME")
        .map(std::path::PathBuf::from)
        .expect("isolated Host home")
        .join("native-ui-test");
    std::fs::create_dir_all(&directory).expect("native UI preferences directory");
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
    app.host = Some(host);
    app.host_url = Some(base_url);
    app.transport = Some(transport);
    app.streams = Some(streams);
    app.refresh_models();
    if let Some(session_id) = session_id {
        app.selected_session = Some(session_id.to_owned());
        app.session_title = "Packaged UI journey".to_owned();
    }
    app.auto_connect = false;
    app.font_catalog_snapshot = Some(fonts::FontCatalog::default());
    app
}

/// Starts the native shell against the packaged Host used by the update prompt probe.
fn packaged_update_app() -> DesktopApp {
    let directory = std::env::var_os("DSH_HOME")
        .map(std::path::PathBuf::from)
        .expect("isolated update home")
        .join("native-update-ui-test");
    std::fs::create_dir_all(&directory).expect("native UI preferences directory");
    let mut app = DesktopApp::new(ShellServices {
        quitting: Arc::new(AtomicBool::new(false)),
        show_requested: Arc::new(AtomicBool::new(false)),
        hide_requested: Arc::new(AtomicBool::new(false)),
        hidden: Arc::new(AtomicBool::new(false)),
        viewport: Arc::new(Mutex::new(None)),
        tray_available: false,
        ui_preferences_path: directory.join("ui-preferences.json"),
        spawn_host: true,
    });
    app.auto_connect = true;
    app
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
    pump_for(harness, what, std::time::Duration::from_secs(5), condition);
}

/// Runs frames until `condition` holds or the operation's live budget expires.
fn pump_for(
    harness: &mut Harness<'_, DesktopApp>,
    what: &str,
    timeout: std::time::Duration,
    condition: impl Fn(&DesktopApp) -> bool,
) {
    let deadline = std::time::Instant::now() + timeout;
    while !condition(harness.state()) {
        assert!(
            std::time::Instant::now() < deadline,
            "the interface never reached: {what}; selected={:?} page_loading={} page_error={:?} model_loading={} model_error={:?} rows={:?} error={:?} preview_error={:?} market_loading={} market_status={:?}",
            harness.state().selected_session,
            harness.state().page_loading,
            harness.state().page_error,
            harness.state().model_receiver.is_some(),
            harness.state().model_error,
            harness.state().transcript.rows().iter().map(|row| row.text.clone()).collect::<Vec<_>>(),
            harness.state().sessions_error,
            harness.state().preview_error,
            harness.state().market_loading,
            harness.state().market_status,
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
    harness.get_by_label("Allow once").click_accesskit();
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
    harness.get_by_label("Reject").click_accesskit();
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
fn first_run_onboarding_creates_a_session_with_the_host_catalog_available() {
    let host = FixtureHost::start();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(4);

    pump(&mut harness, "the first-run model catalog loads", |app| {
        app.model_receiver.is_none() && app.model_catalog.is_some()
    });

    assert!(harness.query_by_label("First run").is_some());
    assert!(harness.query_by_label("Start your first Session").is_some());
    assert!(harness
        .query_by_label("Model selection is available from the composer after opening a Session.")
        .is_some());
    harness
        .get_by_label("Create your first Session")
        .click_accesskit();
    harness.run_steps(3);
    host.wait_for("session/create");
    pump(&mut harness, "the first Session is selected", |app| {
        app.selected_session.as_deref() == Some("fixture-created")
    });
}

#[test]
fn settings_view_keeps_client_preferences_separate_from_host_settings() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.active_view = ActiveView::Settings;
    app.open_settings();
    let mut harness = harness(app, 1280.0, 860.0);
    pump(&mut harness, "Host settings description loads", |app| {
        app.settings_description.is_some()
    });

    assert!(harness
        .query_by_label_contains("Local interface preferences")
        .is_some());
    assert!(harness.query_all_by_label("Appearance").count() >= 2);
    assert!(harness.query_by_label("Layout").is_some());
    assert!(harness.query_by_label("Shortcuts").is_some());
    harness.snapshot("settings-view");
}

#[test]
fn empty_model_catalog_keeps_session_creation_available_and_links_host_settings() {
    let host = FixtureHost::start();
    host.remove_model_providers();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(4);

    pump(&mut harness, "the empty Host model catalog loads", |app| {
        app.model_receiver.is_none() && app.model_catalog.is_some()
    });

    assert!(harness.query_by_label("No provider").is_some());
    assert!(harness
        .query_by_label("No model provider is available")
        .is_some());
    assert!(harness.query_by_label("Host settings").is_some());
    harness
        .get_by_label("Create your first Session")
        .click_accesskit();
    harness.run_steps(3);
    host.wait_for("session/create");
    pump(
        &mut harness,
        "the Session can be created without a provider",
        |app| app.selected_session.as_deref() == Some("fixture-created"),
    );
}

#[test]
fn profile_default_model_reads_back_the_host_catalog_and_keeps_state_on_failure() {
    for fail in [false, true] {
        let host = FixtureHost::start();
        host.set_page("fixture-session", Vec::new());
        if fail {
            host.fail_model_initialization();
        }
        let (mut app, _frames) = fixture(&host);
        app.selected_session = Some("fixture-session".to_owned());
        let mut harness = harness(app, 1280.0, 860.0);
        harness.run_steps(4);
        pump(&mut harness, "the Session model catalog loads", |app| {
            app.model_receiver.is_none() && app.model_catalog.is_some()
        });

        assert!(harness.state().transport.is_some());
        assert!(harness.state().model_init_receiver.is_none());
        assert!(harness.state().model_select_receiver.is_none());
        harness.get_by_label("Model").click_accesskit();
        harness.run_steps(2);
        harness
            .get_by_label("Initialize profile default")
            .click_accesskit();
        harness.run_steps(3);
        assert_eq!(
            host.wait_for("session/initializeDefaultModel")["args"],
            serde_json::json!({})
        );
        if fail {
            pump(
                &mut harness,
                "model initialization failure is reported",
                |app| {
                    app.model_error
                        .as_deref()
                        .is_some_and(|error| error.contains("Default model selection failed"))
                },
            );
            assert_eq!(
                harness
                    .state()
                    .model_selection
                    .as_ref()
                    .map(|selection| selection.model.as_str()),
                Some("fixture-model"),
                "a failed initialization keeps the prior Session model"
            );
        } else {
            pump(
                &mut harness,
                "profile default is read back from the catalog",
                |app| {
                    app.model_init_receiver.is_none()
                        && app.model_receiver.is_none()
                        && app.model_catalog.as_ref().is_some_and(|catalog| {
                            catalog.default.provider == "fixture"
                                && catalog.default.model == "fixture-model"
                                && app.model_selection.as_ref() == Some(&catalog.default)
                        })
                },
            );
            assert_eq!(
                harness
                    .state()
                    .model_selection
                    .as_ref()
                    .map(|selection| selection.model.as_str()),
                Some("fixture-model")
            );
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
    harness.snapshot("session-conversation");
}

#[test]
fn session_search_uses_the_host_and_selects_the_returned_match() {
    let host = FixtureHost::start();
    host.set_page("listed-session", vec![user_record(1, "listed message")]);
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(5);

    let search = harness.get_by_label("Search sessions");
    search.focus();
    search.type_text("needle");
    harness.get_by_label("Search").click_accesskit();
    harness.run_steps(3);

    let request = host.wait_for("session/search");
    assert_eq!(request["args"]["request"]["query"], "needle");
    pump(&mut harness, "Host search results render", |app| {
        app.session_search
            .as_ref()
            .is_some_and(|results| results.items.len() == 1)
    });
    assert!(harness.query_by_label_contains("search-match").is_some());
    harness
        .get_by_label_contains("search-match")
        .click_accesskit();
    pump(&mut harness, "the selected search result opens", |app| {
        app.selected_session.as_deref() == Some("search-match")
    });
}

#[test]
fn workspace_explorer_lists_the_session_tree_and_reads_text_without_local_paths() {
    let host = FixtureHost::start();
    host.set_page("explorer-session", Vec::new());
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("explorer-session".to_owned());
    let mut harness = harness(app, 1440.0, 900.0);
    harness.run_steps(3);

    let root = host.wait_for_nth("workspaceFiles/list", 0);
    assert_eq!(root["args"]["workspaceFileScopeId"], "explorer-session");
    assert_eq!(root["args"]["path"], ".");
    pump(&mut harness, "workspace root renders", |app| {
        app.explorer.has_directory("")
    });
    harness.get_by_label("src directory").click_accesskit();
    harness.run_steps(2);
    assert!(harness.state().explorer.is_expanded("src"));
    let source_directory = host.wait_for_nth("workspaceFiles/list", 1);
    assert_eq!(source_directory["args"]["path"], "src");
    pump(&mut harness, "source directory renders", |app| {
        app.explorer.has_directory("src")
    });
    harness.get_by_label("Preview main.rs").click_accesskit();
    harness.run_steps(2);
    assert_eq!(
        harness.state().explorer.selected_path(),
        Some("src/main.rs")
    );
    let source_read = host.wait_for_nth("workspaceFiles/read", 0);
    assert_eq!(
        source_read["args"]["workspaceFileScopeId"],
        "explorer-session"
    );
    assert_eq!(source_read["args"]["path"], "src/main.rs");
    pump(&mut harness, "read-only workspace source renders", |app| {
        app.explorer.preview_contains("fn fixture_source() {}")
    });
    assert!(harness
        .query_by_label_contains("fn fixture_source() {}")
        .is_some());
    harness.snapshot("workspace-explorer-source");

    harness.get_by_label("Preview image.png").click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().explorer.selected_path(), Some("image.png"));
    host.wait_for_nth("workspaceFiles/read", 1);
    pump(&mut harness, "binary source is refused as text", |app| {
        app.explorer
            .read_error()
            .is_some_and(|error| error.contains("not UTF-8 text"))
    });
    assert!(harness
        .query_by_label_contains("Use Document preview for supported images and Office files.")
        .is_some());
}

#[test]
fn narrow_workspace_explorer_switches_between_the_tree_and_selected_file() {
    let host = FixtureHost::start();
    host.set_page("narrow-explorer-session", Vec::new());
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("narrow-explorer-session".to_owned());
    app.active_view = ActiveView::Explorer;
    app.explorer_fullscreen = true;
    let mut harness = harness(app, 800.0, 720.0);
    harness.run_steps(4);

    host.wait_for_nth("workspaceFiles/list", 0);
    pump(&mut harness, "narrow Explorer tree loads", |app| {
        app.explorer.has_directory("")
    });
    assert!(harness.query_by_label("Preview README.md").is_some());
    harness.get_by_label("Preview README.md").click_accesskit();
    harness.run_steps(2);
    host.wait_for_nth("workspaceFiles/read", 0);
    pump(
        &mut harness,
        "narrow Explorer shows the selected file pane",
        |app| {
            app.explorer.is_showing_file()
                && app.explorer.preview_contains("fixture read-only source")
        },
    );
    assert!(harness.query_by_label("‹ Files").is_some());

    harness.get_by_label("‹ Files").click_accesskit();
    harness.run_steps(2);
    assert!(!harness.state().explorer.is_showing_file());
    assert!(harness.query_by_label("Preview README.md").is_some());
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
    harness.get_by_label("Skills").click_accesskit();
    pump(
        &mut harness,
        "skills load for the selected Session",
        |app| app.skills_receiver.is_none() && app.skills.is_some(),
    );
    assert!(harness.state().active_view == ActiveView::Skills);

    open_session(&mut harness, &frames, "session-b", Vec::new(), 2);
    let rows = harness.state().transcript.rows();
    assert_eq!(rows.iter().map(|row| row.seq).collect::<Vec<_>>(), [10, 11]);
    assert!(harness.state().active_view == ActiveView::Sessions);
    assert!(harness.state().skills.is_none());
    assert!(harness.state().selected_skill.is_none());
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

#[test]
fn snapshot_empty_shell_1366() {
    snapshot_empty_shell(1366.0, 768.0, "shell-1366x768");
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
    let fingerprint = path.bytes().fold(0_u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u64::from(byte))
    });
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

/// Selects a Host settings namespace through its accessible combo box.
fn select_host_settings_namespace(harness: &mut Harness<'_, DesktopApp>, namespace: &str) {
    harness.get_by_label("Settings namespace").click_accesskit();
    harness.run_steps(2);
    harness
        .get_all_by_label(namespace)
        .last()
        .expect("namespace option")
        .click_accesskit();
    harness.run_steps(2);
}

/// Opens the draft editor and replaces its JSON with `text`.
fn edit_settings_json(harness: &mut Harness<'_, DesktopApp>, text: &str) {
    harness
        .get_by_label("Edit user overrides")
        .click_accesskit();
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
                description
                    .namespaces
                    .first()
                    .is_some_and(|ns| ns.revision == 4)
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
fn skills_view_loads_filters_and_opens_the_selected_skill() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    harness.get_by_label("Skills").click_accesskit();
    pump(&mut harness, "Session skills load", |app| {
        app.skills_receiver.is_none() && app.skills.is_some()
    });
    assert_eq!(
        host.wait_for("skills/list")["args"]["request"]["sessionId"],
        "fixture-session"
    );
    assert_eq!(
        harness.state().selected_skill.as_deref(),
        Some("Code review")
    );
    harness.get_by_label("Debug inference").click_accesskit();
    harness.run_steps(2);
    assert_eq!(
        harness.state().selected_skill.as_deref(),
        Some("Debug inference")
    );
    assert!(
        harness
            .query_all_by_label_contains("Diagnose provider and model-server connectivity.")
            .count()
            >= 2
    );

    let filter = harness.get_by_label("Filter skills");
    filter.focus();
    filter.type_text("no matching skill");
    harness.run_steps(2);
    assert!(harness
        .query_by_label("No skills match this filter.")
        .is_some());
}

#[test]
fn plugins_view_reads_the_host_and_applies_enablement_changes() {
    let host = FixtureHost::start();
    let (app, _frames) = fixture(&host);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    harness.get_by_label("Plugins").click_accesskit();
    pump(&mut harness, "the plugin inventory loads", |app| {
        app.plugins_receiver.is_none() && app.plugins_snapshot.is_some()
    });
    assert!(harness
        .query_by_label("fixture-module · enabled · active")
        .is_some());
    harness.get_by_label("Disable").click_accesskit();
    harness.run_steps(3);
    let change = host.wait_for("pluginManager/setPluginEnabled");
    assert_eq!(
        change["args"],
        serde_json::json!({
            "id": "fixture-entry",
            "enabled": false,
        })
    );
    pump(
        &mut harness,
        "the disabled plugin state reads back",
        |app| {
            app.plugin_write_receiver.is_none()
                && app.plugins_receiver.is_none()
                && app.plugins_snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .entries
                        .iter()
                        .any(|entry| entry.entry_id == "fixture-entry" && !entry.enabled)
                })
        },
    );
    assert!(harness.query_by_label("Enable").is_some());
}

#[test]
fn schedule_panel_lists_and_deletes_the_selected_session_schedule() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.start_schedules();
    app.bottom_tool = Some(BottomTool::Schedules);
    let mut harness = harness(app, 1280.0, 860.0);
    pump(&mut harness, "the Session schedule loads", |app| {
        app.schedules_receiver.is_none() && app.schedules.len() == 1
    });

    harness
        .get_by_label("Delete schedule Workspace summary")
        .click_accesskit();
    harness.run_steps(3);
    let request = host.wait_for("schedule/delete");
    assert_eq!(
        request["args"]["request"],
        serde_json::json!({
            "sessionId": "fixture-session",
            "id": "fixture-schedule",
        })
    );
    pump(
        &mut harness,
        "the deleted schedule disappears after read-back",
        |app| {
            app.schedule_delete_receiver.is_none()
                && app.schedules_receiver.is_none()
                && app.schedules.is_empty()
        },
    );
    assert!(harness
        .query_by_label_contains("Workspace summary")
        .is_none());
}

#[test]
fn session_goal_create_pause_resume_and_complete_controls_use_host_revisions() {
    let host = FixtureHost::start();
    host.set_goal(None);
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.active_view = ActiveView::Goals;
    app.show_goals = true;
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    let objective = harness.get_by_label("Goal objective");
    objective.focus();
    objective.type_text("Keep settings defaults explicit");
    harness.run_steps(2);
    harness.get_by_label("Create goal").click_accesskit();
    harness.run_steps(3);
    let create = host.wait_for("goals/create");
    assert_eq!(
        create["args"]["request"]["objective"],
        "Keep settings defaults explicit"
    );
    pump(&mut harness, "the created goal reads back", |app| {
        !app.goal_mutation_loading
            && app
                .goal
                .as_ref()
                .is_some_and(|goal| goal.id == "fixture-goal")
    });

    harness.get_by_label("Pause").click_accesskit();
    harness.run_steps(3);
    host.wait_for("goals/pause");
    pump(&mut harness, "the paused goal reads back", |app| {
        !app.goal_mutation_loading
            && app
                .goal
                .as_ref()
                .is_some_and(|goal| goal.phase == GoalPhase::Paused)
    });

    harness.get_by_label("Resume").click_accesskit();
    harness.run_steps(3);
    host.wait_for("goals/resume");
    pump(&mut harness, "the resumed goal reads back", |app| {
        !app.goal_mutation_loading
            && app
                .goal
                .as_ref()
                .is_some_and(|goal| goal.activation == GoalActivation::Armed)
    });

    harness.get_by_label("Complete").click_accesskit();
    harness.run_steps(3);
    host.wait_for("goals/complete");
    pump(&mut harness, "the completed goal reads back", |app| {
        !app.goal_mutation_loading
            && app
                .goal
                .as_ref()
                .is_some_and(|goal| goal.phase == GoalPhase::Complete)
    });
}

#[test]
fn job_panel_selects_a_job_and_opens_its_live_output_follow() {
    use dsh_client_core::streams::{JobOutput, JobView};

    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.jobs = vec![JobView {
        id: "fixture-job".to_owned(),
        kind: "command".to_owned(),
        label: "Native UI checks".to_owned(),
        owner: None,
        output_limit_bytes: Some(4096),
        status: "running".to_owned(),
        progress: None,
        detail: None,
        started_at: 1,
        finished_at: None,
        output: JobOutput {
            total: 0,
            earliest: 0,
            spill_paths: None,
        },
    }];
    app.bottom_tool = Some(BottomTool::Jobs);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    harness
        .get_by_label("Native UI checks · command · running")
        .click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().job_selected.as_deref(), Some("fixture-job"));
    assert_eq!(harness.state().job_status, "Opening job…");
    assert!(harness.query_by_label("Close Jobs panel").is_some());
}

#[test]
fn deliverable_link_opens_the_document_preview_for_its_session_path() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.deliverables.push(DeliverableFile {
        path: "README.md".to_owned(),
        description: Some("Read the workspace notes".to_owned()),
    });
    app.bottom_tool = Some(BottomTool::Deliverables);
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    harness.get_by_label("README.md").click_accesskit();
    harness.run_steps(3);
    assert!(harness.state().show_document);
    assert!(harness.state().active_view == ActiveView::Document);
    assert_eq!(harness.state().preview_path, "README.md");
    host.wait_for("workspaceFiles/read");
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
        app.market_indexes.iter().any(|index| index["url"] == INDEX)
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
    assert!(harness.query_by_label_contains("fixture-plugin").is_some());

    // Install inspects the exact spec, then installs it and reports the result.
    harness.get_by_label("Install").click_accesskit();
    harness.run_steps(3);
    let inspect = host.wait_for("pluginManager/inspect")["args"].clone();
    assert_eq!(inspect["spec"], "fixture-plugin@1.0.0");
    assert_eq!(
        inspect["options"]["registry"],
        "http://market.fixture/registry/"
    );
    let install = host.wait_for("pluginManager/installBundle")["args"].clone();
    assert_eq!(install["spec"], "fixture-plugin@1.0.0");
    assert_eq!(
        install["options"]["registry"],
        "http://market.fixture/registry/"
    );
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
    assert!(harness
        .query_by_label_contains("Marketplace failed:")
        .is_some());
    assert!(harness.query_by_label_contains("-> installed").is_none());
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
fn native_update_check_uses_the_host_flow_and_reports_failure() {
    for fail in [false, true] {
        let host = FixtureHost::start();
        if fail {
            host.fail_update_open();
        }
        let (app, _frames) = fixture(&host);
        let mut harness = harness(app, 1280.0, 860.0);
        harness.run_steps(3);
        harness.get_by_label("Settings").click_accesskit();
        harness.run_steps(2);
        harness.get_by_label("Desktop updates").click_accesskit();
        harness.run_steps(2);
        harness.get_by_label("Check for updates").click_accesskit();
        harness.run_steps(2);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while harness.state().update_open_receiver.is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "update request did not settle"
            );
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(host.update_open_requests(), 1);
        if fail {
            assert_eq!(
                harness.state().update_status.as_deref(),
                Some("Could not check for updates. Check the Host connection and retry.")
            );
            assert!(harness
                .query_by_label_contains("Could not check for updates")
                .is_some());
        } else {
            assert_eq!(
                harness.state().update_status.as_deref(),
                Some("Update check completed.")
            );
            assert!(harness
                .query_by_label_contains("Update check completed.")
                .is_some());
        }
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

#[test]
fn bottom_tool_buttons_switch_and_toggle_utility_panels() {
    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    let mut harness = harness(app, 1280.0, 860.0);
    harness.run_steps(2);

    for (label, tool) in [
        ("Terminal", BottomTool::Terminal),
        ("Jobs", BottomTool::Jobs),
        ("Schedules", BottomTool::Schedules),
        ("Deliverables", BottomTool::Deliverables),
    ] {
        harness.get_by_label(label).click_accesskit();
        harness.run_steps(2);
        assert_eq!(harness.state().bottom_tool, Some(tool));
    }

    harness
        .get_by_label("Close Deliverables panel")
        .click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().bottom_tool, None);

    harness.get_by_label("Jobs").click_accesskit();
    harness.run_steps(2);
    assert!(harness.query_by_label_contains("Jobs (").is_some());
    assert!(harness.query_by_label("Schedule").is_none());
    assert!(harness
        .query_by_label("No jobs for this Session.")
        .is_some());
    harness.get_by_label("Close Jobs panel").click_accesskit();
    harness.run_steps(2);
    assert_eq!(harness.state().bottom_tool, None);
}

#[test]
fn terminal_tui_bracketed_paste_sends_the_program_protocol_bytes() {
    use dsh_client_core::terminal::{
        TerminalEnvironment, TerminalInput, TerminalInputLimits, TerminalWriter,
    };

    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.terminal.session = Some("fixture-session".to_owned());
    app.terminal.environment = Some(TerminalEnvironment {
        cwd: "/tmp".to_owned(),
        max_input_bytes: 4096,
        max_buffered_bytes: 16384,
        max_cols: 200,
        max_rows: 60,
        scrollback: 1000,
    });
    app.terminal.cols = 80;
    app.terminal.rows = 24;
    app.terminal
        .push_output("\u{1b}[?1049h\u{1b}[?2004h\u{1b}[?1002h\u{1b}[?1006hTUI ready");
    let (input, inputs) = mpsc::channel();
    app.terminal.input = Some(TerminalWriter::new(
        input,
        TerminalInputLimits {
            max_input_bytes: 4096,
            max_buffered_bytes: 16384,
            max_cols: 200,
            max_rows: 60,
        },
    ));
    app.terminal.writable = true;
    let mut harness = harness(app, 1280.0, 860.0);
    harness.get_by_label("Terminal").click_accesskit();
    harness.run_steps(2);
    pump(&mut harness, "the TUI modes are applied", |app| {
        app.terminal.modes.bracketed_paste
            && app.terminal.modes.mouse_reporting()
                == crate::terminal_ansi::MouseReporting::ButtonDrag
            && app.terminal.modes.sgr_mouse
    });
    assert!(harness.query_by_label("Terminal screen").is_some());

    let command = harness.get_by_label("Terminal command");
    command.focus();
    command.type_text("first line\nsecond line");
    harness.run_steps(2);
    assert_eq!(harness.state().terminal.text, "first line\nsecond line");
    harness.get_by_label("Send as paste").click_accesskit();
    harness.run_steps(2);
    assert_eq!(
        inputs.try_recv(),
        Ok(TerminalInput::Write {
            sequence: 1,
            data: "\u{1b}[200~first line\nsecond line\u{1b}[201~".to_owned(),
        })
    );
    assert!(harness.state().terminal.text.is_empty());
    assert_eq!(harness.state().terminal.pending, Some(1));
}

#[test]
fn terminal_tui_mouse_click_is_encoded_and_ack_order_is_preserved() {
    use dsh_client_core::terminal::{
        TerminalAckFrame, TerminalEnvironment, TerminalFrame, TerminalInput, TerminalInputLimits,
        TerminalWriter,
    };

    let host = FixtureHost::start();
    let (mut app, _frames) = fixture(&host);
    app.selected_session = Some("fixture-session".to_owned());
    app.terminal.session = Some("fixture-session".to_owned());
    app.terminal.environment = Some(TerminalEnvironment {
        cwd: "/tmp".to_owned(),
        max_input_bytes: 4096,
        max_buffered_bytes: 16384,
        max_cols: 80,
        max_rows: 24,
        scrollback: 1000,
    });
    app.terminal.cols = 80;
    app.terminal.rows = 24;
    app.terminal
        .push_output("\u{1b}[?1002h\u{1b}[?1006hTUI mouse ready");
    let (input, inputs) = mpsc::channel();
    let (updates, update_receiver) = mpsc::channel();
    app.terminal.input = Some(TerminalWriter::new(
        input,
        TerminalInputLimits {
            max_input_bytes: 4096,
            max_buffered_bytes: 16384,
            max_cols: 80,
            max_rows: 24,
        },
    ));
    app.terminal.updates = Some(update_receiver);
    app.terminal.writable = true;
    let mut harness = harness(app, 1280.0, 860.0);
    harness.get_by_label("Terminal").click_accesskit();
    harness.run_steps(2);
    pump(&mut harness, "the TUI mouse mode is applied", |app| {
        app.terminal.modes.mouse_reporting() == crate::terminal_ansi::MouseReporting::ButtonDrag
            && app.terminal.modes.sgr_mouse
    });

    // An AccessKit click represents a TUI mouse click, not merely a parser test.
    harness.get_by_label("Terminal screen").click();
    harness.run_steps(2);
    let click = inputs.try_recv().expect("encoded mouse click");
    let TerminalInput::Write { sequence: 1, data } = click else {
        panic!("mouse input must be a write");
    };
    let (press, release) = data
        .strip_prefix("\u{1b}[<0;")
        .and_then(|data| data.split_once("M\u{1b}[<0;"))
        .expect("SGR press then release");
    let (press_x, press_y) = press.split_once(';').expect("press coordinates");
    let (release_x, release_y) = release
        .strip_suffix('m')
        .and_then(|coordinate| coordinate.split_once(';'))
        .expect("release coordinates");
    assert_eq!(press_x, release_x);
    assert_eq!(press_y, release_y);
    assert!(press_x.parse::<usize>().is_ok_and(|value| value > 0));
    assert!(press_y.parse::<usize>().is_ok_and(|value| value > 0));
    assert_eq!(harness.state().terminal.pending, Some(1));

    // A wheel event over the TUI is intercepted and queued while the click awaits ack.
    let screen = harness.get_by_label("Terminal screen").rect();
    harness.event(egui::Event::PointerMoved(screen.center()));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 1.0),
        modifiers: egui::Modifiers::default(),
        phase: egui::TouchPhase::Move,
    });
    harness.run_steps(1);
    assert_eq!(harness.state().terminal.queued_input.len(), 1);
    assert_eq!(inputs.try_recv(), Err(mpsc::TryRecvError::Empty));
    updates
        .send(TerminalUpdate::Frame(Box::new(TerminalFrame::Ack(
            TerminalAckFrame {
                sequence: 1,
                refusal: None,
            },
        ))))
        .expect("ack channel");
    harness.run_steps(2);
    let wheel = inputs.try_recv().expect("queued wheel event");
    let TerminalInput::Write { sequence: 2, data } = wheel else {
        panic!("mouse wheel input must be a write");
    };
    let (wheel_x, wheel_y) = data
        .strip_prefix("\u{1b}[<64;")
        .and_then(|coordinates| coordinates.strip_suffix('M'))
        .and_then(|coordinates| coordinates.split_once(';'))
        .expect("SGR wheel coordinates");
    assert_eq!(wheel_x, press_x);
    assert_eq!(wheel_y, press_y);
    assert_eq!(harness.state().terminal.pending, Some(2));
}

#[test]
#[ignore = "requires the packaged Host, Office component, and loopback market from native-journey-probe.sh panels"]
fn packaged_host_marketplace_office_and_terminal_panels_round_trip() {
    use dsh_client_core::plugins::PluginsClient;

    let session_id =
        std::env::var("DSH_NATIVE_UI_PANEL_SESSION").expect("packaged Office session id");
    let market_package =
        std::env::var("DSH_NATIVE_UI_PANEL_PACKAGE").expect("loopback market package name");
    let mut harness = harness(packaged_host_app(Some(&session_id)), 1280.0, 860.0);
    harness.run_steps(5);

    pump_for(
        &mut harness,
        "the packaged Session workspace loads in Explorer",
        std::time::Duration::from_secs(30),
        |app| app.explorer.has_directory(""),
    );
    assert!(harness.query_by_label("Preview probe.txt").is_some());
    harness.get_by_label("Preview probe.txt").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged text file renders read-only in Explorer",
        std::time::Duration::from_secs(30),
        |app| {
            app.explorer
                .preview_contains("Workspace Explorer read-only preview fixture.")
        },
    );
    assert!(harness
        .query_by_label("Workspace Explorer read-only preview fixture.")
        .is_some());

    // The shipped profile deliberately leaves full-text indexing disabled.
    // Verify that a search reports that Host policy and that Refresh still
    // reads the ordinary session inventory successfully.
    let search = harness.get_by_label("Search sessions");
    search.focus();
    search.type_text("no packaged message matches this query");
    harness.get_by_label("Search").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged Host reports its disabled search policy",
        std::time::Duration::from_secs(30),
        |app| app.sessions_error.is_some(),
    );
    assert!(harness
        .state()
        .sessions_error
        .as_deref()
        .is_some_and(|error| error.contains("session search is disabled")));
    harness.get_by_label("Refresh sessions").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged Host restores the session list",
        std::time::Duration::from_secs(30),
        |app| {
            app.session_search.is_none() && app.sessions.iter().any(|s| s.session_id == session_id)
        },
    );

    // Preview an actual Office file in the prepared Session. The Office plugin
    // and its component are installed by the existing packaged probe.
    harness.get_by_label("Document").click_accesskit();
    harness.run_steps(2);
    let path = harness.get_by_label("Document path");
    path.focus();
    path.type_text("probe.docx");
    harness.run_steps(2);
    harness.get_by_label("Preview").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged Office conversion renders in the panel",
        std::time::Duration::from_secs(240),
        |app| app.preview_image.is_some() || app.preview_error.is_some(),
    );
    assert!(
        harness.state().preview_error.is_none(),
        "packaged Office preview failed: {:?}",
        harness.state().preview_error
    );
    let image = harness
        .state()
        .preview_image
        .as_ref()
        .expect("Office preview raster");
    assert_eq!(image.name, "probe.docx");
    assert!(image.size[0] > 0 && image.size[1] > 0);
    assert!(harness
        .query_by_label_contains("Image: probe.docx")
        .is_some());

    // The PTY program enables the same DEC modes real full-screen TUIs use,
    // consumes pasted bytes and a pointer report, then echoes acknowledgements.
    harness.get_by_label("Terminal").click_accesskit();
    harness.run_steps(2);
    pump_for(
        &mut harness,
        "the packaged terminal inventory loads",
        std::time::Duration::from_secs(60),
        |app| app.terminal.environment.is_some() && app.terminal.inventory.is_none(),
    );
    harness.get_by_label("Create terminal").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged Host creates and selects a terminal",
        std::time::Duration::from_secs(60),
        |app| app.terminal.selected.is_some() && app.terminal.inventory.is_none(),
    );
    harness.get_by_label("Attach").click_accesskit();
    pump_for(
        &mut harness,
        "the native panel attaches to the packaged PTY",
        std::time::Duration::from_secs(60),
        |app| app.terminal.writable && app.terminal.input.is_some(),
    );
    let command = harness.get_by_label("Terminal command");
    command.focus();
    command.type_text("node tui_probe.js\n");
    harness.run_steps(2);
    harness.get_by_label("Send exact text").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged TUI advertises its input modes",
        std::time::Duration::from_secs(30),
        |app| {
            app.terminal.modes.bracketed_paste
                && app.terminal.modes.mouse_reporting()
                    == crate::terminal_ansi::MouseReporting::ButtonDrag
                && app.terminal.modes.sgr_mouse
                && app.terminal.output.contains("TUI_READY")
        },
    );

    let command = harness.get_by_label("Terminal command");
    command.focus();
    command.type_text("native paste\nround trip");
    harness.run_steps(2);
    harness.get_by_label("Send as paste").click_accesskit();
    pump_for(
        &mut harness,
        "the real PTY program consumes the bracketed paste",
        std::time::Duration::from_secs(30),
        |app| app.terminal.output.contains("PASTE_OK"),
    );
    harness.get_by_label("Terminal screen").click();
    pump_for(
        &mut harness,
        "the real PTY program consumes the SGR mouse click",
        std::time::Duration::from_secs(30),
        |app| app.terminal.output.contains("MOUSE_OK") && app.terminal.output.contains("TUI_DONE"),
    );
    assert!(harness.state().terminal.error.is_none());

    // The UI asks the packaged Host for a configured index, searches it, then
    // performs a registry install. A follow-up Host read proves the side effect.
    harness.get_by_label("Marketplace").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged market index loads",
        std::time::Duration::from_secs(30),
        |app| !app.market_indexes.is_empty(),
    );
    let search = harness.get_by_label("Marketplace search");
    search.focus();
    search.type_text("native panel probe");
    harness.run_steps(2);
    harness.get_by_label("Search marketplace").click_accesskit();
    pump_for(
        &mut harness,
        "the packaged Host returns a market entry",
        std::time::Duration::from_secs(60),
        |app| !app.market_results.is_empty() && !app.market_loading,
    );
    assert!(harness.query_by_label_contains(&market_package).is_some());
    harness.get_by_label("Install").click_accesskit();
    harness.run_steps(3);
    assert!(
        harness.state().market_loading,
        "Install click did not start the Host operation"
    );
    pump_for(
        &mut harness,
        "the packaged Host settles the market bundle operation",
        std::time::Duration::from_secs(60),
        |app| !app.market_loading,
    );
    let install_status = harness.state().market_status.as_deref().unwrap_or_default();
    assert!(
        install_status.contains("-> applied") || install_status.contains("-> restart-required"),
        "the packaged Host did not apply the market bundle: {install_status}"
    );
    let transport = harness
        .state()
        .transport
        .as_ref()
        .expect("authenticated packaged transport");
    let bundles = PluginsClient::new(transport)
        .list_bundles()
        .expect("read back installed bundles from the packaged Host");
    assert!(bundles
        .iter()
        .any(|bundle| bundle.name == market_package && bundle.installed));

    // Persist a harmless account model-list cache value through the real
    // Settings UI. The Host read-back owns the observable success receipt.
    open_host_settings(&mut harness);
    let initial_settings = harness
        .state()
        .settings_description
        .as_ref()
        .expect("packaged settings description");
    assert!(initial_settings.writable);
    let initial_revision = initial_settings
        .namespaces
        .iter()
        .find(|namespace| namespace.ns == "llm-deepseek-account")
        .unwrap_or_else(|| {
            panic!(
                "packaged account-model settings missing; available namespaces: {:?}",
                initial_settings
                    .namespaces
                    .iter()
                    .map(|namespace| &namespace.ns)
                    .collect::<Vec<_>>()
            )
        })
        .revision;
    select_host_settings_namespace(&mut harness, "llm-deepseek-account");
    edit_settings_json(&mut harness, r#"{"modelsMaxAgeMs": 5001}"#);
    harness.get_by_label("Save settings").click_accesskit();
    harness.run_steps(3);
    assert!(harness.state().settings_update_receiver.is_some());
    pump_for(
        &mut harness,
        "the packaged Host settings mutation reads back",
        std::time::Duration::from_secs(30),
        |app| {
            app.settings_update_receiver.is_none()
                && app
                    .settings_description
                    .as_ref()
                    .is_some_and(|description| {
                        description.namespaces.iter().any(|namespace| {
                            namespace.ns == "llm-deepseek-account"
                                && namespace.revision > initial_revision
                                && namespace
                                    .user
                                    .as_ref()
                                    .and_then(|user| user["modelsMaxAgeMs"].as_u64())
                                    == Some(5001)
                        })
                    })
        },
    );
    assert!(harness.query_by_label_contains("Settings saved.").is_some());

    // Stop and restart the exact packaged Host through the shell's owned
    // control path, then verify it reauthenticates and reloads the same Session.
    harness
        .state_mut()
        .host
        .as_mut()
        .expect("packaged Host handle")
        .shutdown_for_test();
    pump_for(
        &mut harness,
        "the native UI reports the packaged Host disconnect",
        std::time::Duration::from_secs(30),
        |app| app.transport.is_none(),
    );
    assert_eq!(
        harness.state().selected_session.as_deref(),
        Some(session_id.as_str()),
        "the selected Session stays in place while Host reconnects"
    );
    assert_eq!(
        harness.state().explorer.selected_path(),
        Some("probe.txt"),
        "the workspace selection is retained across the disconnect"
    );
    harness.get_by_label("Retry connection").click_accesskit();
    harness.run_steps(3);
    pump_for(
        &mut harness,
        "the native UI authenticates to a restarted packaged Host",
        std::time::Duration::from_secs(120),
        |app| app.transport.is_some() && app.streams.is_some(),
    );
    pump_for(
        &mut harness,
        "the selected Session reloads after Host recovery",
        std::time::Duration::from_secs(60),
        |app| app.selected_session.as_deref() == Some(session_id.as_str()) && !app.page_loading,
    );
}

#[test]
#[ignore = "requires a loopback /models fixture and isolated account grant from native-journey-probe.sh model-default"]
fn packaged_profile_default_model_round_trips_through_the_native_picker() {
    let mut harness = harness(packaged_host_app(None), 1280.0, 860.0);
    harness.run_steps(5);

    harness.get_by_label("New Session").click_accesskit();
    harness.run_steps(3);
    pump_for(
        &mut harness,
        "the packaged Host creates a Session for model selection",
        std::time::Duration::from_secs(60),
        |app| app.selected_session.is_some(),
    );
    pump_for(
        &mut harness,
        "the packaged model catalog loads",
        std::time::Duration::from_secs(30),
        |app| app.model_receiver.is_none() && app.model_catalog.is_some(),
    );
    harness.get_by_label("Model").click_accesskit();
    harness.run_steps(2);
    harness
        .get_by_label("Initialize profile default")
        .click_accesskit();
    harness.run_steps(3);
    pump_for(
        &mut harness,
        "the packaged Host default appears in the model picker",
        std::time::Duration::from_secs(30),
        |app| {
            app.model_catalog.as_ref().is_some_and(|catalog| {
                catalog.default.provider == "deepseek-account"
                    && !catalog.groups.is_empty()
                    && app.model_selection.as_ref() == Some(&catalog.default)
            })
        },
    );
    let transport = harness
        .state()
        .transport
        .as_ref()
        .expect("authenticated packaged transport");
    let catalog = ModelClient::new(transport)
        .catalog()
        .expect("read back the account model catalog from the packaged Host");
    assert_eq!(
        harness.state().model_selection.as_ref(),
        Some(&catalog.default),
        "the native picker reflects the default the packaged Host now reports"
    );
}

#[test]
#[cfg(unix)]
#[ignore = "requires a generated signed feed and fake AppImage from update-flow-probe.mjs --gui"]
fn signed_appimage_swap_is_confirmed_through_the_native_update_modal() {
    use std::os::unix::fs::PermissionsExt;

    let image = std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .expect("isolated fake AppImage");
    let expected = std::env::var_os("DSH_NATIVE_UI_UPDATE_EXPECTED")
        .map(std::path::PathBuf::from)
        .expect("expected update artifact bytes");
    let mut harness = harness(packaged_update_app(), 1280.0, 860.0);
    pump_for(
        &mut harness,
        "the native app authenticates with the packaged Host",
        std::time::Duration::from_secs(60),
        |app| app.transport.is_some(),
    );
    harness.get_by_label("Settings").click_accesskit();
    harness.run_steps(2);
    harness.get_by_label("Desktop updates").click_accesskit();
    harness.run_steps(2);
    harness.get_by_label("Check for updates").click_accesskit();
    harness.run_steps(2);
    pump_for(
        &mut harness,
        "the native update control reaches the signed feed and requests download confirmation",
        std::time::Duration::from_secs(120),
        |app| {
            app.update_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.action == "download")
        },
    );
    assert!(harness.query_by_label("Download").is_some());
    assert!(harness
        .query_by_label_contains("Version 2.0.0 is available.")
        .is_some());
    harness.get_by_label("Download").click_accesskit();
    pump_for(
        &mut harness,
        "the downloaded signed artifact requests install confirmation",
        std::time::Duration::from_secs(180),
        |app| {
            app.update_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.action == "install")
        },
    );
    assert!(harness.query_by_label("Install and restart").is_some());
    harness
        .get_by_label("Install and restart")
        .click_accesskit();
    pump_for(
        &mut harness,
        "the Host replaces the image and requests restart",
        std::time::Duration::from_secs(180),
        |app| app.quitting.load(Ordering::Acquire),
    );

    assert_eq!(
        std::fs::read(&image).expect("installed fake AppImage"),
        std::fs::read(&expected).expect("signed artifact fixture")
    );
    assert_ne!(
        std::fs::metadata(&image)
            .expect("installed image metadata")
            .permissions()
            .mode()
            & 0o111,
        0,
        "the replacement stays executable"
    );
    assert!(harness.state().update_prompt.is_none());
}
