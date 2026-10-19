//! Sidecar supervisor for the Desktop Host.
//!
//! Frames are newline-delimited JSON with an `@@DSH@@` prefix, so unrelated
//! output on the same stream is skipped instead of parsed. Requests carry a
//! `requestId` that the Host echoes in its response.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

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
}

impl HostClient {
    /// Spawn the Host and read frames until it reports `ready`.
    ///
    /// @param binary - Host executable or launcher script.
    /// @param args - positional arguments after the executable name.
    /// @returns the client and the authenticated Web URL.
    pub fn spawn(
        binary: &str,
        args: &[String],
    ) -> Result<(Arc<Self>, String), Box<dyn std::error::Error>> {
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
        });

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let url = loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Err("desktop host closed before reporting a URL".into());
            }
            let Some(frame) = parse_frame(&line) else { continue };
            match frame.get("type").and_then(Value::as_str) {
                Some("ready") => {
                    let url = frame
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or("desktop host ready frame has no URL")?;
                    break url.to_owned();
                }
                Some("fatal") => {
                    let message = frame
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown failure");
                    return Err(format!("desktop host failed: {message}").into());
                }
                _ => {}
            }
        };

        let dispatch = Arc::clone(&client);
        std::thread::spawn(move || dispatch.read_frames(reader));
        Ok((client, url))
    }

    /// Drain frames for the rest of the Host lifetime.
    fn read_frames(&self, mut reader: BufReader<ChildStdout>) {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let Some(frame) = parse_frame(&line) else { continue };
                    if frame.get("type").and_then(Value::as_str) == Some("fatal") {
                        eprintln!("desktop host fatal: {}", line.trim());
                    }
                    if let Some(id) = frame.get("requestId").and_then(Value::as_u64) {
                        let waiter = self
                            .pending
                            .lock()
                            .ok()
                            .and_then(|mut pending| pending.remove(&id));
                        if let Some(waiter) = waiter {
                            let _ = waiter.send(frame);
                        }
                    }
                }
            }
        }
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
        let Ok(mut child) = self.child.lock() else { return };
        let steps = SHUTDOWN_GRACE.as_millis() / 100;
        for _ in 0..steps {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
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
