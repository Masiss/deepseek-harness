//! Native file dialogs.
//!
//! The picker blocks its calling thread until the desktop answers, and a
//! slow portal must not stall the interface, so callers start one on a worker
//! thread and poll the returned channel from the frame loop.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

/// Starts a file-open picker on a worker thread.
///
/// @returns A receiver that yields the chosen path, or `None` when cancelled.
pub fn open_file_async() -> Receiver<Option<PathBuf>> {
    spawn(true)
}

/// Starts a file-save picker on a worker thread.
///
/// @returns A receiver that yields the chosen path, or `None` when cancelled.
pub fn save_file_async() -> Receiver<Option<PathBuf>> {
    spawn(false)
}

fn spawn(open: bool) -> Receiver<Option<PathBuf>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let dialog = rfd::FileDialog::new();
        let path = if open {
            dialog.pick_file()
        } else {
            dialog.save_file()
        };
        let _ = sender.send(path);
    });
    receiver
}
