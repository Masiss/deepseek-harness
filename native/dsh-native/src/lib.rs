//! Native PTY provider for the Desktop runtime.
//!
//! The Harness terminal service calls a small slice of `node-pty`:
//! `spawn`, `pid`, `write`, `resize`, `pause`, `resume`, `kill`, `onData`, and
//! `onExit`. This crate implements exactly that slice on top of `portable-pty`
//! so the Desktop payload carries one Rust native module instead of the
//! node-pty C++ addon, and so `packages/` keeps its existing provider.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;
use nix::sys::signal::{kill, killpg, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

/// Fork options the terminal service passes to `node-pty`.
#[napi(object)]
pub struct PtyOptions {
    /// Terminal type recorded in the environment (`TERM`).
    pub name: Option<String>,
    pub cols: Option<u32>,
    pub rows: Option<u32>,
    pub cwd: Option<String>,
    pub env: Option<HashMap<String, String>>,
}

/// Top-level exit as `node-pty` publishes it.
#[napi(object)]
pub struct PtyExit {
    pub exit_code: i32,
    pub signal: Option<i32>,
}

/// One allocated PTY and its child.
#[napi]
pub struct Pty {
    pid: i32,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    paused: Arc<AtomicBool>,
}

fn reason(error: impl std::fmt::Display) -> Error {
    Error::from_reason(error.to_string())
}

fn signal_from_name(name: Option<&str>) -> Signal {
    match name {
        Some("SIGINT") | Some("SIGINT2") => Signal::SIGINT,
        Some("SIGQUIT") => Signal::SIGQUIT,
        Some("SIGKILL") => Signal::SIGKILL,
        Some("SIGTERM") => Signal::SIGTERM,
        Some("SIGSTOP") => Signal::SIGSTOP,
        Some("SIGCONT") => Signal::SIGCONT,
        Some("SIGUSR1") => Signal::SIGUSR1,
        Some("SIGUSR2") => Signal::SIGUSR2,
        // `node-pty` hangs up a terminal when no signal is named.
        _ => Signal::SIGHUP,
    }
}

/// Spawn `file` on a new PTY and stream its output and exit.
///
/// The reader starts immediately; the JavaScript wrapper buffers output until
/// a listener attaches, which is what makes `spawn` followed by `onData`
/// lossless like `node-pty`.
#[napi]
pub fn spawn(
    file: String,
    args: Vec<String>,
    options: Option<PtyOptions>,
    on_data: ThreadsafeFunction<String>,
    on_exit: ThreadsafeFunction<PtyExit>,
) -> Result<Pty> {
    let options = options.unwrap_or(PtyOptions {
        name: None,
        cols: None,
        rows: None,
        cwd: None,
        env: None,
    });
    let cols = options.cols.unwrap_or(80).max(1) as u16;
    let rows = options.rows.unwrap_or(24).max(1) as u16;
    let pair = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(reason)?;
    let mut command = CommandBuilder::new(file);
    command.args(args);
    if let Some(cwd) = &options.cwd {
        command.cwd(cwd);
    }
    if let Some(env) = &options.env {
        for (name, value) in env {
            command.env(name, value);
        }
    }
    let mut child = pair.slave.spawn_command(command).map_err(reason)?;
    // Closing the slave keeps the master's reads ending at child exit.
    drop(pair.slave);
    let pid = child.process_id().unwrap_or(0) as i32;
    // The waiter below reaps this pid directly so the exit report carries the
    // exact signal; the handle holds no ownership after spawn.
    drop(child);
    let mut reader = pair.master.try_clone_reader().map_err(reason)?;
    let writer = pair.master.take_writer().map_err(reason)?;
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
    let paused = Arc::new(AtomicBool::new(false));

    let reader_paused = Arc::clone(&paused);
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            while reader_paused.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                    if on_data
                        .call(Ok(text), ThreadsafeFunctionCallMode::NonBlocking)
                        != Status::Ok
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    thread::spawn(move || {
        let exit = match waitpid(Pid::from_raw(pid), None) {
            Ok(WaitStatus::Exited(_, code)) => PtyExit {
                exit_code: code,
                signal: None,
            },
            Ok(WaitStatus::Signaled(_, signal, _)) => PtyExit {
                exit_code: 128 + signal as i32,
                signal: Some(signal as i32),
            },
            Ok(_) | Err(_) => PtyExit {
                exit_code: -1,
                signal: None,
            },
        };
        let _ = on_exit.call(Ok(exit), ThreadsafeFunctionCallMode::NonBlocking);
    });

    Ok(Pty {
        pid,
        writer: Arc::new(Mutex::new(writer)),
        master,
        paused,
    })
}

#[napi]
impl Pty {
    /// Child process id, `0` when the platform did not report one.
    #[napi(getter)]
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Write input to the terminal.
    #[napi]
    pub fn write(&self, data: String) -> Result<()> {
        let mut writer = self.writer.lock().map_err(reason)?;
        writer.write_all(data.as_bytes()).map_err(reason)?;
        writer.flush().map_err(reason)
    }

    /// Resize the pty window.
    #[napi]
    pub fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        let master = self.master.lock().map_err(reason)?;
        master
            .resize(PtySize {
                rows: rows.max(1) as u16,
                cols: cols.max(1) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(reason)
    }

    /// Stop delivering output; the pty buffer applies backpressure.
    #[napi]
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resume delivery.
    #[napi]
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// Signal the terminal's process group, falling back to the child itself.
    #[napi]
    pub fn kill(&self, signal: Option<String>) -> Result<()> {
        let signal = signal_from_name(signal.as_deref());
        let pid = Pid::from_raw(self.pid);
        if killpg(pid, signal).is_err() {
            let _ = kill(pid, signal);
        }
        Ok(())
    }
}
