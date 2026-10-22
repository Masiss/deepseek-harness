//! Optional startup update check and best-effort replacement installer.

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Deserialize)]
struct UpdateInfo {
    version: String,
    url: String,
}

/// Checks for an update and optionally installs it when explicitly enabled.
pub fn check() {
    let Ok(endpoint) = std::env::var("DSH_UPDATE_CHECK_URL") else {
        return;
    };
    let current = std::env::var("DSH_DESKTOP_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let result = reqwest::blocking::get(&endpoint)
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.json::<UpdateInfo>());
    match result {
        Ok(update) if update.version != current => {
            eprintln!(
                "desktop update available: {} ({})",
                update.version, update.url
            );
            if std::env::var("DSH_AUTO_UPDATE").as_deref() == Ok("1") {
                install(&update.url);
            }
        }
        Ok(_) => eprintln!("desktop is up to date ({current})"),
        Err(error) => eprintln!("desktop update check failed: {error}"),
    }
}

fn install(url: &str) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let response = reqwest::blocking::get(url)?.error_for_status()?;
        let bytes = response.bytes()?;
        if bytes.is_empty() {
            return Err("downloaded update is empty".into());
        }
        let current = std::env::current_exe()?;
        let mut downloaded = std::env::temp_dir();
        downloaded.push(format!("dsh-update-{}", std::process::id()));
        fs::write(&downloaded, &bytes)?;
        let old = PathBuf::from(format!("{}.old", current.display()));
        fs::rename(&current, &old)?;
        if let Err(error) = fs::rename(&downloaded, &current) {
            let _ = fs::rename(&old, &current);
            return Err(error.into());
        }
        let _ = fs::remove_file(&old);
        Command::new(&current).spawn()?;
        std::process::exit(0);
    })();
    if let Err(error) = result {
        eprintln!("desktop update install failed: {error}");
    }
}
