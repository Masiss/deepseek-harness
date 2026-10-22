//! Native file dialogs.

use std::path::PathBuf;

/// Opens a native file picker and returns the selected path.
pub fn open_file() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_file()
}

/// Opens a native save picker and returns the selected path.
pub fn save_file() -> Option<PathBuf> {
    rfd::FileDialog::new().save_file()
}
