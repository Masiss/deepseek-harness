//! Native Host sidecar discovery and launch-token handling.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Host launch configuration read from the packaged Desktop environment.
#[derive(Clone, Debug)]
pub struct HostConfig {
    pub binary: String,
    pub args: Vec<String>,
}

impl HostConfig {
    /// Reads the documented launch variables; missing binary means disconnected mode.
    pub fn from_env() -> Option<Self> {
        let binary = std::env::var("DSH_DESKTOP_HOST_BIN").ok()?;
        let mut args = Vec::new();
        if let (Ok(runtime), Ok(project)) = (
            std::env::var("DSH_DESKTOP_RUNTIME_DIR"),
            std::env::var("DSH_DESKTOP_PROJECT_DIR"),
        ) {
            args.extend([runtime, project]);
        }
        match (
            std::env::var("DSH_DESKTOP_PRIMARY_RUNTIME").ok(),
            std::env::var("DSH_DESKTOP_PNPM_ENTRY").ok(),
            std::env::var("DSH_DESKTOP_NODE_BIN").ok(),
        ) {
            (None, None, None) => {}
            (Some(primary), Some(pnpm), Some(node)) => args.extend([primary, pnpm, node]),
            _ => return None,
        }
        if let Ok(version) = std::env::var("DSH_DESKTOP_VERSION") {
            args.push(version);
        }
        if let Ok(version) = std::env::var("DSH_CLIENT_VERSION") {
            args.push(version);
        }
        Some(Self { binary, args })
    }
}

/// Host sidecar handle containing the discovered authenticated URL.
pub struct Host {
    launch_url: Option<String>,
    host_url: Option<String>,
    error: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

impl Host {
    /// Starts the Host and waits briefly for its launch URL.
    pub fn start() -> Self {
        let Some(config) = HostConfig::from_env() else {
            return Self {
                launch_url: None,
                host_url: None,
                error: None,
                child: None,
                stdin: None,
            };
        };
        let mut child = match Command::new(&config.binary)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => return Self::failed(format!("Host spawn failed: {error}")),
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Self::failed("Host stdout unavailable".to_owned());
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Self::failed("Host stderr unavailable".to_owned());
        };
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Self::failed("Host stdin unavailable".to_owned());
        };
        let (tx, rx) = mpsc::channel();
        let tx_stderr = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(url) = find_launch_url(&line) {
                    let _ = tx_stderr.send(url);
                }
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(url) = find_launch_url(&line) {
                    let _ = tx.send(url);
                }
            }
        });
        match rx.recv_timeout(READY_TIMEOUT) {
            Ok(url) => Self {
                host_url: Some(base_url(&url)),
                launch_url: Some(url),
                error: None,
                child: Some(child),
                stdin: Some(stdin),
            },
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Self::failed(format!("Host did not become ready: {error}"))
            }
        }
    }

    fn failed(error: String) -> Self {
        Self {
            launch_url: None,
            host_url: None,
            error: Some(error),
            child: None,
            stdin: None,
        }
    }

    fn shutdown(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            if let Err(error) = stdin
                .write_all(b"@@DSH@@{\"type\":\"shutdown\"}\n")
                .and_then(|()| stdin.flush())
            {
                eprintln!("Host shutdown request failed: {error}");
            }
        }
        if let Some(mut child) = self.child.take() {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Ok(None) => {
                        if let Err(error) = child.kill() {
                            eprintln!("Host forced shutdown failed: {error}");
                        }
                        if let Err(error) = child.wait() {
                            eprintln!("Host wait failed: {error}");
                        }
                        return;
                    }
                    Err(error) => {
                        eprintln!("Host status check failed: {error}");
                        let _ = child.kill();
                        let _ = child.wait();
                        return;
                    }
                }
            }
        }
    }

    pub fn host_url(&self) -> Option<&str> {
        self.host_url.as_deref()
    }
    pub fn launch_url(&self) -> Option<&str> {
        self.launch_url.as_deref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn find_launch_url(line: &str) -> Option<String> {
    let marker = if let Some(index) = line.find("dsh: web:") {
        index + 9
    } else if let Some(index) = line.find("dsh web:") {
        index + 8
    } else {
        return None;
    };
    let value = line[marker..].trim();
    if (value.starts_with("http://") || value.starts_with("https://")) && value.contains("?token=")
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn base_url(url: &str) -> String {
    url.split_once('?').map_or_else(
        || url.to_owned(),
        |(base, _)| base.trim_end_matches('/').to_owned(),
    )
}
