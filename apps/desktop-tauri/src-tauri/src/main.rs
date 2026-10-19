//! Tauri shell for the DeepSeek Harness desktop application.
//!
//! The shell supervises the Desktop Host as a sidecar and hands its authenticated
//! Web URL to a system WebView. Phase 0 keeps the surface deliberately small: one
//! window, one control channel, and no desktop bridge yet.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

/// Prefix that separates a control frame from unrelated Host output.
const FRAME_PREFIX: &str = "@@DSH@@";

/// The running Host sidecar, kept alive for the lifetime of the window.
struct HostProcess(Mutex<Child>);

/// Spawn the Host binary named by `DSH_DESKTOP_HOST_BIN`.
fn spawn_host() -> std::io::Result<Child> {
    let binary = std::env::var("DSH_DESKTOP_HOST_BIN").unwrap_or_else(|_| "dsh-host".to_owned());
    let mut command = Command::new(binary);
    if let (Ok(runtime), Ok(project)) = (
        std::env::var("DSH_DESKTOP_RUNTIME_DIR"),
        std::env::var("DSH_DESKTOP_PROJECT_DIR"),
    ) {
        command.arg(runtime).arg(project);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
}

/// Read Host frames until `ready`, then drain the rest on a background thread.
fn await_ready(mut child: Child) -> Result<(Child, String), Box<dyn std::error::Error>> {
    let stdout = child.stdout.take().ok_or("desktop host stdout is unavailable")?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err("desktop host closed before reporting a URL".into());
        }
        let Some(payload) = line.trim().strip_prefix(FRAME_PREFIX) else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else { continue };
        match value.get("type").and_then(|kind| kind.as_str()) {
            Some("ready") => {
                let url = value
                    .get("url")
                    .and_then(|url| url.as_str())
                    .ok_or("desktop host ready frame has no URL")?;
                let url = url.to_owned();
                // The Host keeps writing frames (sessions, shutdown); drain them so a
                // full pipe can never block it.
                std::thread::spawn(move || {
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) => {
                                let Some(payload) = line.trim().strip_prefix(FRAME_PREFIX) else { continue };
                                let Ok(frame) = serde_json::from_str::<serde_json::Value>(payload) else { continue };
                                // Frames after `ready` are Host state reports (sessions,
                                // shutdown). The shell does not consume them yet; only a
                                // failure is worth surfacing.
                                if frame.get("type").and_then(|kind| kind.as_str()) == Some("fatal") {
                                    eprintln!("desktop host fatal: {}", line.trim());
                                }
                            }
                        }
                    }
                });
                return Ok((child, url));
            },
            Some("fatal") => {
                let message = value
                    .get("message")
                    .and_then(|message| message.as_str())
                    .unwrap_or("unknown failure");
                return Err(format!("desktop host failed: {message}").into());
            },
            _ => {},
        }
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let child = spawn_host()?;
            let (child, url) = await_ready(child)?;
            app.manage(HostProcess(Mutex::new(child)));
            let target = tauri::Url::parse(&url)?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(target))
                .title("DeepSeek Harness")
                .inner_size(1280.0, 860.0)
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the desktop shell")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<HostProcess>() {
                    if let Ok(mut child) = state.0.lock() {
                        if let Some(stdin) = child.stdin.as_mut() {
                            let _ = writeln!(stdin, "{FRAME_PREFIX}{{\"type\":\"shutdown\"}}");
                        }
                        // The Host reports `shutdown-complete` and exits on its own; the
                        // timeout only bounds a Host that cannot finish cleanup.
                        for _ in 0..50 {
                            match child.try_wait() {
                                Ok(Some(_)) => return,
                                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                                Err(_) => break,
                            }
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
            }
        });
}
