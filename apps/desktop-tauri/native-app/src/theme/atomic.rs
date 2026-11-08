//! Atomic file replacement shared by theme stores and caches.
//!
//! A caller writes a sibling temporary file, syncs it, then renames it over the
//! destination. A surviving temporary file is removed when any step fails.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path` with a sibling temporary file and rename.
///
/// @param path — destination file; its parent directory is created when needed.
/// @param bytes — complete replacement contents.
/// @returns the filesystem failure, if any.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".theme.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
