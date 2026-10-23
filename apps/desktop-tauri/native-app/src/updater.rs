//! Optional metadata check. Executable replacement waits for signed release metadata.

use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct UpdateInfo {
    version: String,
    url: String,
}

/// Checks for a newer version without installing unverified executable bytes.
pub fn check() {
    let Ok(endpoint) = std::env::var("DSH_UPDATE_CHECK_URL") else {
        return;
    };
    let current = std::env::var("DSH_DESKTOP_VERSION").unwrap_or_else(|_| "0.0.0".to_owned());
    let result = (|| -> Result<UpdateInfo, Box<dyn std::error::Error>> {
        let endpoint = reqwest::Url::parse(&endpoint)?;
        if endpoint.scheme() != "https" {
            return Err("update metadata URL must use HTTPS".into());
        }
        let client = reqwest::blocking::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(client.get(endpoint).send()?.error_for_status()?.json()?)
    })();
    match result {
        Ok(update) if is_newer(&current, &update.version) => {
            eprintln!(
                "desktop update available: {} ({})",
                update.version, update.url
            );
            if std::env::var("DSH_AUTO_UPDATE").as_deref() == Ok("1") {
                eprintln!("desktop auto-update disabled: release signature verification is not configured");
            }
        }
        Ok(_) => eprintln!("desktop is up to date ({current})"),
        Err(error) => eprintln!("desktop update check failed: {error}"),
    }
}

fn is_newer(current: &str, offered: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(offered),
    ) {
        (Ok(current), Ok(offered)) => offered > current,
        _ => {
            eprintln!("desktop update check skipped: invalid semantic version metadata");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn update_check_rejects_same_older_and_invalid_versions() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.3", "1.2.2"));
        assert!(!is_newer("1.2.3", "not-a-version"));
    }
}
