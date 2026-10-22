//! Native providers for the Desktop runtime.
//!
//! The Harness terminal service calls a small slice of `node-pty`:
//! `spawn`, `pid`, `write`, `resize`, `pause`, `resume`, `kill`, `onData`, and
//! `onExit`. This crate implements exactly that slice on top of `portable-pty`
//! so the Desktop payload carries one Rust native module instead of the
//! node-pty C++ addon, and so `packages/` keeps its existing provider.
//!
//! The session lease takes an exclusive, nonblocking POSIX flock on a
//! descriptor the caller owns. [`try_lock_exclusive`] performs that syscall
//! and reports the errno the kernel returned.

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

mod image;
mod image_meta;
#[cfg(test)]
mod image_meta_tests;
mod image_scan;
#[cfg(test)]
mod image_scan_tests;
#[cfg(test)]
mod pty_tests;

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

/// The length of the leading bytes that form complete UTF-8 sequences.
/// @param bytes - Bytes read so far.
/// @returns The byte count that can be decoded without splitting a code point.
fn utf8_complete_prefix(bytes: &[u8]) -> usize {
    let mut index = bytes.len();
    let mut inspected = 0;
    while index > 0 && inspected < 4 {
        index -= 1;
        inspected += 1;
        let byte = bytes[index];
        if byte & 0b1100_0000 == 0b1000_0000 {
            // A continuation byte: the sequence starts earlier.
            continue;
        }
        let width = if byte < 0x80 {
            1
        } else if byte >= 0xF0 {
            4
        } else if byte >= 0xE0 {
            3
        } else {
            2
        };
        let end = index + width;
        if end > bytes.len() {
            // The sequence is incomplete: keep it for the next read.
            return index;
        }
        if bytes[index + 1..end]
            .iter()
            .all(|value| value & 0b1100_0000 == 0b1000_0000)
        {
            return end;
        }
        // Not a valid sequence: the lossy decoder reports it as it is.
        return bytes.len();
    }
    bytes.len()
}

fn reason(error: impl std::fmt::Display) -> Error {
    Error::from_reason(error.to_string())
}

/// Attempt an exclusive, nonblocking POSIX flock on the caller's descriptor.
///
/// The descriptor stays the caller's: the syscall neither opens, duplicates,
/// nor closes it. Closing the locked descriptor releases the lock once every
/// descriptor for its open file description is closed.
/// @param fd - Open file descriptor to lock.
/// @returns `0` when the lock was taken, otherwise the positive errno the
///   kernel reported. `EAGAIN` means another descriptor holds the lock.
#[napi]
pub fn try_lock_exclusive(fd: i32) -> Result<i32> {
    // The lock lives with the descriptor: it is released when every
    // descriptor for its open file description is closed, so the syscall runs
    // without the guard that would unlock on drop.
    let locked = unsafe { nix::libc::flock(fd, nix::libc::LOCK_EX | nix::libc::LOCK_NB) };
    if locked == 0 {
        return Ok(0);
    }
    Ok(std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc_errno_fallback()))
}

/// Errno the kernel reported when the error carries none.
fn libc_errno_fallback() -> i32 {
    nix::libc::EIO
}

// Errno of the last failed syscall on this thread.
thread_local! {
    static LAST_ERRNO: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// Record the errno the kernel reported and return it.
fn record_errno() -> i32 {
    let errno = std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc_errno_fallback());
    LAST_ERRNO.with(|cell| cell.set(errno));
    errno
}

/// Errno of the last failed syscall on this thread.
/// @returns The positive errno, or `0` when no syscall failed yet.
#[napi]
pub fn last_errno() -> i32 {
    LAST_ERRNO.with(|cell| cell.get())
}

/// Replace the process image with the given program.
///
/// The call replaces the process when the kernel accepts it, so the caller
/// never returns. A failure leaves the process running and reports the errno
/// through {@link last_errno}.
/// @param file - Path of the program to execute.
/// @param argv - Arguments, without the terminating null the caller passes.
/// @param envp - Environment entries in `key=value` form.
/// @returns `-1` when the kernel refused the exec.
#[napi]
pub fn execve(file: String, argv: Vec<String>, envp: Vec<String>) -> Result<i32> {
    let path = std::ffi::CString::new(file).map_err(|_| Error::from_reason("execve: path contains a null byte"))?;
    let arguments = c_string_array(argv)?;
    let environment = c_string_array(envp)?;
    let mut argument_pointers: Vec<*const std::ffi::c_char> =
        arguments.iter().map(|value| value.as_ptr()).collect();
    argument_pointers.push(std::ptr::null());
    let mut environment_pointers: Vec<*const std::ffi::c_char> =
        environment.iter().map(|value| value.as_ptr()).collect();
    environment_pointers.push(std::ptr::null());
    let replaced = unsafe {
        nix::libc::execve(
            path.as_ptr(),
            argument_pointers.as_ptr(),
            environment_pointers.as_ptr(),
        )
    };
    if replaced == -1 {
        record_errno();
    }
    Ok(replaced as i32)
}

/// Control a file descriptor's flags.
/// @param fd - Descriptor to control.
/// @param cmd - `fcntl` command, such as `F_GETFD` or `F_SETFD`.
/// @param arg - Command argument.
/// @returns The command's result, or `-1` when the kernel refused it.
#[napi]
pub fn fcntl(fd: i32, cmd: i32, arg: i32) -> Result<i32> {
    let result = unsafe { nix::libc::fcntl(fd, cmd, arg) };
    if result == -1 {
        record_errno();
    }
    Ok(result as i32)
}

/// Null-terminated C strings for the argument and environment arrays.
fn c_string_array(values: Vec<String>) -> Result<Vec<std::ffi::CString>> {
    values
        .into_iter()
        .map(|value| {
            std::ffi::CString::new(value)
                .map_err(|_| Error::from_reason("execve: argument contains a null byte"))
        })
        .collect()
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
    let child = pair.slave.spawn_command(command).map_err(reason)?;
    // Closing the slave keeps the master's reads ending at child exit.
    drop(pair.slave);
    // Pid 0 would make `waitpid`/`killpg` address the caller's own process
    // group, so a child without an id is refused rather than guessed.
    let pid = child
        .process_id()
        .ok_or_else(|| Error::from_reason("spawn: the child reported no process id"))? as i32;
    // The waiter below reaps this pid directly so the exit report carries the
    // exact signal; the handle holds no ownership after spawn.
    drop(child);
    let mut reader = pair.master.try_clone_reader().map_err(reason)?;
    let writer = pair.master.take_writer().map_err(reason)?;
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
    let paused = Arc::new(AtomicBool::new(false));

    // The terminal's output is drained before the exit is reported, so a
    // consumer never sees the exit ahead of the last output.
    let (drained, drained_waiter) = std::sync::mpsc::channel::<()>();
    let reader_paused = Arc::clone(&paused);
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        // A read can end inside a UTF-8 sequence; the incomplete tail waits for
        // the next read so a split code point is not decoded twice.
        let mut pending: Vec<u8> = Vec::new();
        loop {
            while reader_paused.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
            }
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    let complete = utf8_complete_prefix(&pending);
                    let text = String::from_utf8_lossy(&pending[..complete]).into_owned();
                    pending.drain(..complete);
                    if !text.is_empty()
                        && on_data.call(Ok(text), ThreadsafeFunctionCallMode::NonBlocking) != Status::Ok
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if !pending.is_empty() {
            let text = String::from_utf8_lossy(&pending).into_owned();
            let _ = on_data.call(Ok(text), ThreadsafeFunctionCallMode::NonBlocking);
        }
        let _ = drained.send(());
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
        let _ = drained_waiter.recv_timeout(Duration::from_secs(5));
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
        // A terminal without a recorded pid never signals: pid 0 would address
        // the Host's own process group.
        if self.pid <= 0 {
            return Ok(());
        }
        let signal = signal_from_name(signal.as_deref());
        let pid = Pid::from_raw(self.pid);
        if killpg(pid, signal).is_err() {
            let _ = kill(pid, signal);
        }
        Ok(())
    }
}
