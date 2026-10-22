//! Optional startup update metadata check.

use serde::Deserialize;

#[derive(Deserialize)]
struct UpdateInfo {
    version: String,
    url: String,
}

/// Checks the configured update endpoint without installing anything.
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
        }
        Ok(_) => eprintln!("desktop is up to date ({current})"),
        Err(error) => eprintln!("desktop update check failed: {error}"),
    }
}
