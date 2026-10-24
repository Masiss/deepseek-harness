//! Restart the outer AppImage only after the old shell releases its process lock.
//!
//! The Host owns signature verification, task admission, and atomic replacement.
//! This helper never downloads or replaces executable bytes.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Resolve the installed image, not the binary in its temporary mount.
pub fn executable(appimage: Option<PathBuf>) -> std::io::Result<PathBuf> {
    match appimage {
        Some(path) if path.is_absolute() && path.is_file() => Ok(path),
        Some(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "APPIMAGE must name an existing absolute file; restart the installation manually",
        )),
        None => std::env::current_exe(),
    }
}

/// Start a bounded waiter; a spawn failure leaves the running application intact.
/// The caller must dispose its Host and exit after this succeeds. The waiter exits
/// without launching if the old shell remains alive for sixty seconds.
#[cfg(unix)]
pub fn schedule(path: &Path, pid: u32) -> std::io::Result<Child> {
    if pid == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid restart pid",
        ));
    }
    Command::new("sh")
        .args(["-c", &format!(
            "remaining=300; while kill -0 {pid} 2>/dev/null; do remaining=$((remaining - 1)); [ \"$remaining\" -gt 0 ] || exit 1; sleep 0.2; done; exec \"$0\""
        )])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
}

#[cfg(not(unix))]
pub fn schedule(_path: &Path, _pid: u32) -> std::io::Result<Child> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "native restart is only supported on Unix",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_image_does_not_fall_back_to_the_mounted_executable() {
        assert!(executable(Some(PathBuf::from("relative.AppImage"))).is_err());
        assert!(executable(Some(PathBuf::from("/missing-dsh-native-image"))).is_err());
        assert!(executable(None).is_ok());
    }

    #[test]
    fn refuses_process_group_zero() {
        assert!(schedule(Path::new("/unused"), 0).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn restart_runs_the_replaced_outer_image_after_the_old_process_exits() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        struct Process(Child);
        impl Drop for Process {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let directory = loop {
            let path = std::env::temp_dir().join(format!(
                "dsh-native-restart-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => break Directory(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("temporary directory: {error}"),
            }
        };
        let image = directory.0.join("outer ' $(exit 99).AppImage");
        std::fs::write(&image, "#!/bin/sh\nexit 88\n").unwrap();
        let staged = directory.0.join("staged");
        std::fs::write(&staged, "#!/bin/sh\nprintf new > \"$0.marker\"\n").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(staged, &image).unwrap();
        let selected = executable(Some(image.clone())).unwrap();
        let mut old = Process(
            Command::new("sh")
                .args(["-c", "echo ready; read -r done"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut ready = String::new();
        BufReader::new(old.0.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        let mut waiter = Process(schedule(&selected, old.0.id()).unwrap());
        assert!(old.0.try_wait().unwrap().is_none());
        assert!(waiter.0.try_wait().unwrap().is_none());
        drop(old.0.stdin.take());
        old.0.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = waiter.0.try_wait().unwrap() {
                assert!(status.success(), "replacement did not execute: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "restart waiter did not settle");
            std::thread::sleep(Duration::from_millis(10));
        }
        let marker = PathBuf::from(format!("{}.marker", image.display()));
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "new");
    }
}
