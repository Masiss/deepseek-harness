//! Native Host sidecar discovery and launch-token handling.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Lifecycle notifications sent by the Host control channel after readiness.
pub enum HostEvent {
    Restart,
    Failed(String),
    UpdatePrompt(UpdatePrompt),
}

/// A Host request for user confirmation before downloading or installing an update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePrompt {
    pub prompt_id: u64,
    pub action: String,
    pub version: String,
}

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
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    events: Option<mpsc::Receiver<HostEvent>>,
}

impl Host {
    /// A handle with a known launch URL and no child process.
    ///
    /// @param launch_url - Authenticated attach URL the shell would have read from the Host.
    /// @returns A handle whose shutdown has nothing to terminate.
    #[cfg(test)]
    pub fn for_test(launch_url: &str) -> Self {
        Self {
            launch_url: Some(launch_url.to_owned()),
            host_url: None,
            error: None,
            child: None,
            stdin: None,
            events: None,
        }
    }

    /// Starts the Host and waits briefly for its launch URL.
    pub fn start() -> Self {
        let Some(config) = HostConfig::from_env() else {
            return Self {
                launch_url: None,
                host_url: None,
                error: None,
                child: None,
                stdin: None,
                events: None,
            };
        };
        Self::launch(config)
    }

    fn launch(config: HostConfig) -> Self {
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
        std::thread::spawn(move || {
            // Stderr is not a control channel and may contain launch tokens.
            let _ = std::io::copy(&mut BufReader::new(stderr), &mut std::io::sink());
        });
        let stdin = Arc::new(Mutex::new(stdin));
        let control_stdin = Arc::clone(&stdin);
        let (event_tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut ready = false;
            for line in BufReader::new(stdout).lines() {
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        let message = format!("Host control channel read failed: {error}");
                        let _ = tx.send(Err(message.clone()));
                        let _ = event_tx.send(HostEvent::Failed(message));
                        return;
                    }
                };
                let Some(frame) = control_frame(&line) else {
                    continue;
                };
                match frame.get("type").and_then(Value::as_str) {
                    Some("ready") if !ready => {
                        if let Some(url) = frame
                            .get("url")
                            .and_then(Value::as_str)
                            .filter(|url| valid_launch_url(url))
                        {
                            ready = true;
                            let _ = tx.send(Ok(url.to_owned()));
                        } else {
                            let _ = tx.send(Err(
                                "Host ready frame has no valid loopback launch URL".to_owned(),
                            ));
                            return;
                        }
                    }
                    Some("fatal") => {
                        let message = frame
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown failure")
                            .to_owned();
                        let _ = tx.send(Err(message.clone()));
                        let _ = event_tx.send(HostEvent::Failed(message));
                        return;
                    }
                    Some("restart") if ready => {
                        let _ = event_tx.send(HostEvent::Restart);
                    }
                    Some("update-prompt") => {
                        if let Some(prompt_id) = frame.get("promptId").and_then(Value::as_u64) {
                            let action = frame.get("action").and_then(Value::as_str);
                            let version = frame.get("version").and_then(Value::as_str);
                            if let (Some(action), Some(version)) = (action, version) {
                                if matches!(action, "download" | "install") && !version.is_empty() {
                                    let _ = event_tx.send(HostEvent::UpdatePrompt(UpdatePrompt {
                                        prompt_id,
                                        action: action.to_owned(),
                                        version: version.to_owned(),
                                    }));
                                    continue;
                                }
                            }
                            if let Err(error) =
                                send_update_response(&control_stdin, prompt_id, false)
                            {
                                let message = format!("Host update response failed: {error}");
                                let _ = tx.send(Err(message.clone()));
                                let _ = event_tx.send(HostEvent::Failed(message));
                                return;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let _ = tx.send(Err("Host stopped before readiness".to_owned()));
            let _ = event_tx.send(HostEvent::Failed("Host control channel closed".to_owned()));
        });
        match rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(url)) => Self {
                host_url: Some(base_url(&url)),
                launch_url: Some(url),
                error: None,
                child: Some(child),
                stdin: Some(stdin),
                events: Some(events),
            },
            Ok(Err(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                Self::failed(error)
            }
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
            events: None,
        }
    }

    fn shutdown(&mut self) {
        if let Some(stdin) = self.stdin.take() {
            if let Ok(mut stdin) = stdin.lock() {
                if let Err(error) = stdin
                    .write_all(b"@@DSH@@{\"type\":\"shutdown\"}\n")
                    .and_then(|()| stdin.flush())
                {
                    eprintln!("Host shutdown request failed: {error}");
                }
            };
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

    /// Answers one outstanding Host update confirmation prompt.
    pub fn respond_to_update_prompt(&self, prompt_id: u64, accepted: bool) -> std::io::Result<()> {
        let Some(stdin) = self.stdin.as_ref().filter(|_| self.error.is_none()) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Host control input is unavailable",
            ));
        };
        send_update_response(stdin, prompt_id, accepted)
    }

    /// Drain one lifecycle event without blocking the native event loop.
    pub fn event(&mut self) -> Option<HostEvent> {
        let event = self.events.as_ref()?.try_recv().ok()?;
        if let HostEvent::Failed(message) = &event {
            self.error = Some(message.clone());
            self.host_url = None;
            self.launch_url = None;
        }
        Some(event)
    }

    /// Stops the owned sidecar through its control frame for packaged lifecycle tests.
    #[cfg(test)]
    pub(crate) fn shutdown_for_test(&mut self) {
        self.shutdown();
    }
}

fn send_update_response<W: Write>(
    stdin: &Arc<Mutex<W>>,
    prompt_id: u64,
    accepted: bool,
) -> std::io::Result<()> {
    let mut input = stdin.lock().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "Host control input lock is unavailable",
        )
    })?;
    let response = json!({"type":"update-prompt", "promptId":prompt_id, "accepted":accepted});
    writeln!(input, "@@DSH@@{response}")?;
    input.flush()
}

impl Drop for Host {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn control_frame(line: &str) -> Option<Value> {
    if line.len() > 1024 * 1024 {
        return None;
    }
    serde_json::from_str(line.strip_prefix("@@DSH@@")?).ok()
}

fn valid_launch_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .query_pairs()
            .any(|(key, value)| key == "token" && !value.is_empty())
}

fn base_url(url: &str) -> String {
    url.split_once('?').map_or_else(
        || url.to_owned(),
        |(base, _)| base.trim_end_matches('/').to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn launch_script(script: &str) -> Host {
        Host::launch(HostConfig {
            binary: "sh".to_owned(),
            args: vec!["-c".to_owned(), script.to_owned()],
        })
    }

    #[cfg(unix)]
    fn next_event(host: &mut Host) -> HostEvent {
        // Put the received event back so assertions also exercise Host::event state updates.
        let receiver = host.events.take().expect("Host event receiver");
        let event = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(event)
            .unwrap_or_else(|_| panic!("event receiver closed"));
        host.events = Some(rx);
        let event = host.event().expect("queued Host event");
        host.events = Some(receiver);
        event
    }

    #[test]
    fn update_responses_preserve_prompt_identity_and_decision() {
        for accepted in [true, false] {
            let input = Arc::new(Mutex::new(Vec::new()));
            send_update_response(&input, 42, accepted).unwrap();
            let bytes = input.lock().unwrap();
            assert_eq!(bytes.last(), Some(&b'\n'));
            let response = control_frame(std::str::from_utf8(&bytes).unwrap().trim_end()).unwrap();
            assert_eq!(
                response,
                json!({"type":"update-prompt", "promptId":42, "accepted":accepted})
            );
        }
    }

    #[test]
    fn update_response_reports_write_and_flush_failures() {
        struct FailingInput {
            fail_write: bool,
        }
        impl Write for FailingInput {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.fail_write {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "write failed",
                    ))
                } else {
                    Ok(bytes.len())
                }
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "flush failed",
                ))
            }
        }
        for fail_write in [true, false] {
            let input = Arc::new(Mutex::new(FailingInput { fail_write }));
            let error = send_update_response(&input, 42, true).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            assert_eq!(
                error.to_string(),
                if fail_write {
                    "write failed"
                } else {
                    "flush failed"
                }
            );
        }
        let host = Host::failed("not running".to_owned());
        assert_eq!(
            host.respond_to_update_prompt(42, true).unwrap_err().kind(),
            std::io::ErrorKind::NotConnected
        );
    }

    #[cfg(unix)]
    #[test]
    fn accepted_and_declined_download_and_install_prompts_reach_restart() {
        for (action, accepted) in [
            ("download", true),
            ("download", false),
            ("install", true),
            ("install", false),
        ] {
            let mut host = launch_script(&format!(
                r#"
                printf '%s\n' '@@DSH@@{{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}}'
                printf '%s\n' '@@DSH@@{{"type":"update-prompt","promptId":42,"action":"{action}","version":"2.0.0"}}'
                IFS= read -r decision
                [ "$decision" = '@@DSH@@{{"accepted":{accepted},"promptId":42,"type":"update-prompt"}}' ] || exit 9
                printf '%s\n' '@@DSH@@{{"type":"restart"}}'
                IFS= read -r shutdown
            "#
            ));
            let HostEvent::UpdatePrompt(prompt) = next_event(&mut host) else {
                panic!("expected update prompt");
            };
            assert_eq!(prompt.action, action);
            assert_eq!(prompt.prompt_id, 42);
            host.respond_to_update_prompt(prompt.prompt_id, accepted)
                .unwrap();
            assert!(matches!(next_event(&mut host), HostEvent::Restart));
            assert!(host.error().is_none());
            assert!(host.launch_url().is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn malformed_prompt_fields_are_declined_without_showing_a_prompt() {
        for fields in [
            r#""action":"delete","version":"2.0.0""#,
            r#""action":"install","version":"""#,
            r#""action":"install","version":2"#,
            r#""action":null,"version":"2.0.0""#,
            r#""version":"2.0.0""#,
            r#""action":"download""#,
        ] {
            let mut host = launch_script(&format!(
                r#"
                printf '%s\n' '@@DSH@@{{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}}'
                printf '%s\n' '@@DSH@@{{"type":"update-prompt","promptId":42,{fields}}}'
                IFS= read -r decision
                [ "$decision" = '@@DSH@@{{"accepted":false,"promptId":42,"type":"update-prompt"}}' ] || exit 9
                printf '%s\n' '@@DSH@@{{"type":"restart"}}'
                IFS= read -r shutdown
            "#
            ));
            assert!(
                matches!(next_event(&mut host), HostEvent::Restart),
                "{fields}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn invalid_prompt_ids_and_unframed_prompts_do_not_answer_other_prompts() {
        let mut host = launch_script(
            r#"
            printf '%s\n' '@@DSH@@{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}'
            printf '%s\n' '@@DSH@@{' '{"type":"update-prompt","promptId":1,"action":"install","version":"2.0.0"}'
            for id in 'null' '-1' '1.5' '"42"'; do
                printf '%s\n' "@@DSH@@{\"type\":\"update-prompt\",\"promptId\":$id,\"action\":\"install\",\"version\":\"2.0.0\"}"
            done
            printf '%s\n' '@@DSH@@{"type":"update-prompt","action":"install","version":"2.0.0"}'
            printf '%s\n' '@@DSH@@{"type":"update-prompt","promptId":42,"action":"install","version":"2.0.0"}'
            IFS= read -r decision
            [ "$decision" = '@@DSH@@{"accepted":false,"promptId":42,"type":"update-prompt"}' ] || exit 9
            printf '%s\n' '@@DSH@@{"type":"restart"}'
            IFS= read -r shutdown
        "#,
        );
        let HostEvent::UpdatePrompt(prompt) = next_event(&mut host) else {
            panic!("expected valid update prompt");
        };
        assert_eq!(prompt.prompt_id, 42);
        host.respond_to_update_prompt(prompt.prompt_id, false)
            .unwrap();
        assert!(matches!(next_event(&mut host), HostEvent::Restart));
    }

    #[cfg(unix)]
    #[test]
    fn fatal_and_control_channel_closure_clear_authenticated_urls() {
        for (ending, expected) in [
            (
                r#"printf '%s\n' '@@DSH@@{"type":"fatal","message":"update failed"}' '@@DSH@@{"type":"update-prompt","promptId":42,"action":"install","version":"2.0.0"}' '@@DSH@@{"type":"restart"}'"#,
                "update failed",
            ),
            ("exit 0", "Host control channel closed"),
        ] {
            let mut host = launch_script(&format!(
                r#"
                printf '%s\n' '@@DSH@@{{"type":"restart"}}'
                printf '%s\n' '@@DSH@@{{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}}'
                {ending}
            "#
            ));
            let HostEvent::Failed(message) = next_event(&mut host) else {
                panic!("expected Host failure");
            };
            assert_eq!(message, expected);
            assert_eq!(host.error(), Some(expected));
            assert!(host.host_url().is_none());
            assert!(host.launch_url().is_none());
            assert_eq!(
                host.respond_to_update_prompt(42, true).unwrap_err().kind(),
                std::io::ErrorKind::NotConnected
            );
            assert!(matches!(
                host.events
                    .as_ref()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn automatic_decline_reports_a_closed_host_input() {
        let mut host = launch_script(
            r#"
            exec 0<&-
            printf '%s\n' '@@DSH@@{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}'
            printf '%s\n' '@@DSH@@{"type":"update-prompt","promptId":42,"action":"invalid","version":"2.0.0"}'
        "#,
        );
        let HostEvent::Failed(message) = next_event(&mut host) else {
            panic!("expected response failure");
        };
        assert!(
            message.starts_with("Host update response failed:"),
            "{message}"
        );
        assert!(host.host_url().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_control_output_reports_a_read_failure() {
        let mut host = launch_script(
            r#"
            printf '%s\n' '@@DSH@@{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}'
            printf '\377\n'
        "#,
        );
        let HostEvent::Failed(message) = next_event(&mut host) else {
            panic!("expected control read failure");
        };
        assert!(
            message.starts_with("Host control channel read failed:"),
            "{message}"
        );
        assert!(host.host_url().is_none());
    }

    #[test]
    fn only_framed_ready_messages_are_authoritative() {
        assert!(control_frame("dsh web: http://127.0.0.1:1234/?token=not-ready").is_none());
        assert!(control_frame("@@DSH@@{").is_none());
        let ready = control_frame(
            "@@DSH@@{\"type\":\"ready\",\"url\":\"http://127.0.0.1:1234/?token=ok\"}",
        )
        .unwrap();
        assert!(valid_launch_url(ready["url"].as_str().unwrap()));
    }

    #[test]
    fn launch_url_requires_loopback_and_a_token() {
        for invalid in [
            "https://example.com/?token=ok",
            "http://localhost/?token=ok",
            "http://127.0.0.1/",
            "http://127.0.0.1/?token=",
            "http://user@127.0.0.1/?token=ok",
        ] {
            assert!(!valid_launch_url(invalid), "{invalid}");
        }
        assert!(valid_launch_url("http://[::1]:1234/?token=ok"));
    }

    #[cfg(unix)]
    #[test]
    fn host_startup_reports_fatal_without_waiting_for_a_launch_log() {
        let host = Host::launch(HostConfig {
            binary: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "printf '%s\\n' '@@DSH@@{\"type\":\"fatal\",\"message\":\"profile invalid\"}'"
                    .to_owned(),
            ],
        });
        assert_eq!(host.error(), Some("profile invalid"));
        assert!(host.launch_url().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn host_ready_then_restart_and_shutdown_use_control_frames() {
        let mut host = Host::launch(HostConfig {
            binary: "sh".to_owned(),
            args: vec!["-c".to_owned(), r#"
                printf '%s\n' '@@DSH@@{"type":"ready","url":"http://127.0.0.1:1234/?token=ok"}'
                printf '%s\n' '@@DSH@@{"type":"update-prompt","promptId":7,"action":"install","version":"2.0.0"}'
                IFS= read -r decision
                [ "$decision" = '@@DSH@@{"accepted":false,"promptId":7,"type":"update-prompt"}' ] || exit 9
                printf '%s\n' '@@DSH@@{"type":"restart"}'
                IFS= read -r shutdown
                [ "$shutdown" = '@@DSH@@{"type":"shutdown"}' ] || exit 10
            "#.to_owned()],
        });
        assert_eq!(host.host_url(), Some("http://127.0.0.1:1234"));
        let event = host
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let HostEvent::UpdatePrompt(prompt) = event else {
            panic!("expected update confirmation request")
        };
        assert_eq!(prompt.prompt_id, 7);
        assert_eq!(prompt.action, "install");
        assert_eq!(prompt.version, "2.0.0");
        host.respond_to_update_prompt(prompt.prompt_id, false)
            .unwrap();
        let event = host
            .events
            .as_ref()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert!(matches!(event, HostEvent::Restart));
        host.shutdown();
        assert!(host.child.is_none());
    }
}
