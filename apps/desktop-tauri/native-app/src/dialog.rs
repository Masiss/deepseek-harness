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
    spawn()
}

/// Starts a directory picker on a worker thread.
///
/// @returns A receiver that yields the chosen directory, or `None` when cancelled.
pub fn open_folder_async() -> Receiver<Option<PathBuf>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let dialog = rfd::FileDialog::new();
        let path = dialog.pick_folder();
        let _ = sender.send(path);
    });
    receiver
}

fn spawn() -> Receiver<Option<PathBuf>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let path = rfd::FileDialog::new().pick_file();
        let _ = sender.send(path);
    });
    receiver
}
