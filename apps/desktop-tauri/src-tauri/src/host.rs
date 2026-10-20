//! Sidecar supervisor for the Desktop Host.
//!
//! Frames are newline-delimited JSON with an `@@DSH@@` prefix, so unrelated
//! output on the same stream is skipped instead of parsed. Requests carry a
//! `requestId` that the Host echoes in its response.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

/// Marker that separates a control frame from unrelated output.
const FRAME_PREFIX: &str = "@@DSH@@";

/// How long a request may take before the shell treats the Host as unresponsive.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a shutdown may take before the Host is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// A running Host process plus its framed control channel.
pub struct HostClient {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Value>>>,
    next_request: AtomicU64,
    /// Application handle for user-facing update decisions; attached after spawn.
    app: Mutex<Option<AppHandle>>,
    /// Whether the Host reported its authenticated Web URL.
    ready: AtomicBool,
    /// Set when the Host confirms its shutdown; the process exits right after.
    shutdown_complete: (Mutex<bool>, Condvar),
}

impl HostClient {
    /// Spawn the Host and report its authenticated Web URL when it is ready.
    ///
    /// The Host answers on its own thread, so the shell shows its window before
    /// the Host is ready and navigates to the URL when it arrives. A Host that
    /// fails or never answers reaches `on_failure` instead of blocking the
    /// shell.
    ///
    /// @param binary - Host executable or launcher script.
    /// @param args - positional arguments after the executable name.
    /// @param on_ready - called with the authenticated Web URL.
    /// @param on_failure - called with the reason the Host cannot serve.
    /// @returns the client.
    pub fn spawn(
        binary: &str,
        args: &[String],
        on_ready: impl Fn(String) + Send + 'static,
        on_failure: impl Fn(String) + Send + 'static,
    ) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or("desktop host stdin is unavailable")?;
        let stdout = child.stdout.take().ok_or("desktop host stdout is unavailable")?;

        let client = Arc::new(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            pending: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            app: Mutex::new(None),
            ready: AtomicBool::new(false),
            shutdown_complete: (Mutex::new(false), Condvar::new()),
        });

        let dispatch = Arc::clone(&client);
        let reader = BufReader::new(stdout);
        std::thread::spawn(move || {
            HostClient::read_frames(&dispatch, reader, on_ready, on_failure);
        });
        Ok(client)
    }

    /// Whether the Host reported its authenticated Web URL.
    ///
    /// @returns true once the Host is ready to serve.
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// Give the reader thread an application handle for native update dialogs.
    ///
    /// @param app - handle used to show dialogs and relaunch the process.
    pub fn attach(&self, app: AppHandle) {
        if let Ok(mut slot) = self.app.lock() {
            *slot = Some(app);
        }
    }

    /// Drain frames for the rest of the Host lifetime.
    fn read_frames(
        self: &Arc<Self>,
        mut reader: BufReader<ChildStdout>,
        on_ready: impl Fn(String) + Send + 'static,
        on_failure: impl Fn(String) + Send + 'static,
    ) {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    if !self.ready.load(Ordering::SeqCst) {
                        on_failure("the Host stopped before it was ready".to_owned());
                    }
                    break;
                }
                Ok(_) => {
                    let Some(frame) = parse_frame(&line) else { continue };
                    match frame.get("type").and_then(Value::as_str) {
                        Some("ready") => {
                            let Some(url) = frame.get("url").and_then(Value::as_str) else {
                                on_failure("the Host reported readiness without a URL".to_owned());
                                break;
                            };
                            self.ready.store(true, Ordering::SeqCst);
                            on_ready(url.to_owned());
                        }
                        Some("fatal") => {
                            let message = frame
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown failure");
                            on_failure(format!("the Host failed: {message}"));
                            break;
                        }
                        Some("shutdown-complete") => {
                            let (lock, condvar) = &self.shutdown_complete;
                            if let Ok(mut done) = lock.lock() {
                                *done = true;
                                condvar.notify_all();
                            }
                        }
                        Some("update-prompt") => self.prompt_update(&frame),
                        Some("restart") => self.relaunch(),
                        _ => {}
                    }
                }
            }
        }
    }
    /// Ask the user to authorize one update step.
    ///
    /// The answer travels back over the same channel with `promptId`, which is
    /// separate from the shell-to-Host `requestId` space, so a Host that gave up
    /// waiting finds no waiter and its answer is ignored.
    ///
    /// @param frame - `update-prompt` request carrying `promptId`, `action`, and `version`.
    fn prompt_update(self: &Arc<Self>, frame: &Value) {
        let Some(id) = frame.get("promptId").and_then(Value::as_u64) else { return };
        let action = frame.get("action").and_then(Value::as_str).unwrap_or("download");
        let version = frame.get("version").and_then(Value::as_str).unwrap_or("");
        let Some(app) = self.app.lock().ok().and_then(|slot| slot.clone()) else { return };
        let (title, message, confirm) = if action == "install" {
            (
                crate::locale::text("updateDownloadedTitle").replace("{version}", version),
                crate::locale::text("updateDownloadedDetail").to_owned(),
                crate::locale::text("updateAcknowledge"),
            )
        } else {
            (
                crate::locale::text("updateAvailable").replace("{version}", version),
                crate::locale::text("updateDetail").to_owned(),
                crate::locale::text("updateDownload"),
            )
        };
        let client = Arc::clone(self);
        app.dialog()
            .message(message)
            .title(title)
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                confirm.to_owned(),
                crate::locale::text("updateLater").to_owned(),
            ))
            .show(move |accepted| {
                let _ = client.send(&json!({
                    "type": "update-prompt",
                    "promptId": id,
                    "accepted": accepted,
                }));
            });
    }

    /// Relaunch the application after the Host replaced its installation.
    ///
    /// An AppImage runs from a temporary mount of the file it was started from,
    /// so re-executing the running binary would start the replaced image from
    /// memory. The outer `$APPIMAGE` is the file the installer rewrote.
    fn relaunch(self: &Arc<Self>) {
        let Some(app) = self.app.lock().ok().and_then(|slot| slot.clone()) else { return };
        let executable = std::env::var("APPIMAGE")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok());
        if let Some(executable) = executable {
            // The single-instance plugin owns a lock until this process exits,
            // so the replacement waits for this pid instead of racing it.
            let script = format!(
                "while kill -0 {} 2>/dev/null; do sleep 0.2; done; exec \"$0\"",
                std::process::id()
            );
            let _ = Command::new("sh")
                .arg("-c")
                .arg(script)
                .arg(executable)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        app.exit(0);
    }

    /// Send one frame to the Host.
    ///
    /// @param frame - JSON object carrying at least a `type`.
    pub fn send(&self, frame: &Value) -> Result<(), String> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| "desktop host stdin lock poisoned".to_owned())?;
        writeln!(stdin, "{FRAME_PREFIX}{frame}").map_err(|error| error.to_string())?;
        stdin.flush().map_err(|error| error.to_string())
    }

    /// Send a request and wait for the response with the same `requestId`.
    ///
    /// @param kind - request `type`, such as `quit-inspection`.
    /// @returns the response frame, or an error when the Host never answers.
    pub fn request(&self, kind: &str) -> Result<Value, String> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "desktop host pending lock poisoned".to_owned())?
            .insert(id, sender);
        let sent = self.send(&json!({ "type": kind, "requestId": id }));
        if let Err(error) = sent {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }
        match receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(frame) => Ok(frame),
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                Err(format!("desktop host did not answer {kind}"))
            }
        }
    }

    /// Ask the Host to shut down, then bound the wait and kill it if needed.
    pub fn shutdown(&self) {
        let _ = self.send(&json!({ "type": "shutdown" }));
        // The Host confirms its shutdown once the profile is disposed, and the
        // process exits right after. A forced kill before that confirmation can
        // cut a session write short, so the confirmation decides the wait.
        let confirmed = self.wait_for_shutdown(SHUTDOWN_GRACE);
        let Ok(mut child) = self.child.lock() else { return };
        if confirmed {
            for _ in 0..10 {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => break,
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Wait for the Host's shutdown confirmation.
    ///
    /// @param limit - How long to wait for the confirmation.
    /// @returns whether the Host confirmed before the limit.
    fn wait_for_shutdown(&self, limit: Duration) -> bool {
        let (lock, condvar) = &self.shutdown_complete;
        let Ok(guard) = lock.lock() else { return false };
        if *guard {
            return true;
        }
        let (guard, _) = condvar
            .wait_timeout(guard, limit)
            .unwrap_or_else(|error| error.into_inner());
        *guard
    }
}

/// Decode one framed line, ignoring unrelated output and malformed JSON.
fn parse_frame(line: &str) -> Option<Value> {
    let payload = line.trim().strip_prefix(FRAME_PREFIX)?;
    serde_json::from_str(payload).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_frame;

    #[test]
    fn decodes_only_framed_json_objects() {
        let frame = parse_frame("@@DSH@@{\"type\":\"ready\",\"url\":\"http://x\"}")
            .expect("a framed object must decode");
        assert_eq!(frame.get("type").and_then(serde_json::Value::as_str), Some("ready"));
        assert!(parse_frame("the host printed a line").is_none());
        assert!(parse_frame("@@DSH@@{").is_none());
        assert!(parse_frame("@@DSH@@{\"url\":\"x\"}").is_some());
    }
}
