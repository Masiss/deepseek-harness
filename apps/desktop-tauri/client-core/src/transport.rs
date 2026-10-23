//! Plain HTTP transport owns cancellation and decoding so clients share identical request semantics.

use crate::{
    auth::{credential_from_redirect, Credential},
    envelope::{RequestEnvelope, ResponseEnvelope, ResponseResult},
    Error,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;
use url::Url;

const CANCEL_POLL: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: Url,
    next_id: Arc<std::sync::atomic::AtomicU64>,
    credential: Option<Credential>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl HttpTransport {
    pub fn new(base_url: &str) -> Result<Self, Error> {
        Self::with_client(base_url, default_client()?)
    }

    /// Builds a transport from an already configured cookie-preserving client.
    pub fn with_client(base_url: &str, client: reqwest::Client) -> Result<Self, Error> {
        Ok(Self {
            client,
            base_url: Url::parse(base_url).map_err(|source| Error::InvalidUrl { source })?,
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            credential: None,
            runtime: Arc::new(runtime()?),
        })
    }

    /// Exchanges a Host launch URL and returns the clean redirected base URL plus transport.
    pub fn authenticate(launch_url: &str) -> Result<(String, Self), Error> {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()?;
        let mut connect_url = Url::parse(launch_url).map_err(|source| Error::InvalidUrl { source })?;
        let tokens: Vec<_> = connect_url
            .query_pairs()
            .filter(|(key, _)| key == "token")
            .map(|(_, value)| value.into_owned())
            .collect();
        if tokens.len() != 1 {
            return Err(Error::InvalidCredential);
        }
        connect_url.set_path("/connect");
        connect_url.set_query(None);
        connect_url
            .query_pairs_mut()
            .append_pair("token", &tokens[0])
            .append_pair("interactions", "answer");
        let runtime = Arc::new(runtime()?);
        let response = runtime.block_on(async { client.get(connect_url).send().await })?;
        if !response.status().is_redirection() {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        let credential = credential_from_redirect(response.headers())?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .ok_or(Error::InvalidCredential)?;
        let target = response
            .url()
            .join(location.to_str().map_err(|_| Error::InvalidCredential)?)
            .map_err(|source| Error::InvalidUrl { source })?;
        if target.origin() != response.url().origin() || !target.path().ends_with("/api/remote.mux")
        {
            return Err(Error::InvalidCredential);
        }
        let mut base = response.url().clone();
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        let base_url = base.to_string();
        let mut transport = Self::with_client(&base_url, client)?;
        transport.credential = Some(credential);
        Ok((base_url, transport))
    }

    /// Returns the clean Host base URL established during authentication.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Returns the session cookie required by the Gateway WebSocket mux.
    pub fn credential(&self) -> Result<&Credential, Error> {
        self.credential.as_ref().ok_or(Error::MissingCredential)
    }

    /// Sends one unary RPC; setting `cancel` aborts an in-flight request at the next poll point.
    pub fn request(
        &self,
        method: &str,
        payload: Value,
        cancel: &AtomicBool,
    ) -> Result<Value, Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let id = format!(
            "native-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let url = self
            .base_url
            .join(&format!("api/{method}"))
            .map_err(|source| Error::InvalidUrl { source })?;
        let envelope = RequestEnvelope::new(id.clone(), method, payload);
        self.runtime.block_on(async {
            let exchange = async {
                let response = self
                    .client
                    .post(url)
                    .json(&envelope)
                    .send()
                    .await?;
                if !response.status().is_success() {
                    return Err(Error::HttpStatus(response.status().as_u16()));
                }
                decode_response(response, &id).await
            };
            tokio::select! {
                biased;
                () = wait_for_cancel(cancel) => Err(Error::Cancelled),
                result = exchange => result,
            }
        })
    }

    /// Posts a raw body to an authenticated route and returns the decoded JSON body.
    pub fn post_json(
        &self,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
        cancel: &AtomicBool,
    ) -> Result<Value, Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|source| Error::InvalidUrl { source })?;
        self.runtime.block_on(async {
            let exchange = async {
                let response = self
                    .client
                    .post(url)
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(body)
                    .send()
                    .await?;
                let status = response.status();
                if !status.is_success() {
                    return Err(Error::HttpStatus(status.as_u16()));
                }
                Ok(response.json().await?)
            };
            tokio::select! {
                biased;
                () = wait_for_cancel(cancel) => Err(Error::Cancelled),
                result = exchange => result,
            }
        })
    }

    /// Runs a request on a worker thread and lets the caller abort it by sending or dropping the handle.
    pub fn request_cancellable(
        &self,
        method: String,
        payload: Value,
    ) -> (mpsc::Receiver<Result<Value, Error>>, mpsc::Sender<()>) {
        let (done_tx, done_rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let transport = self.clone();
        thread::spawn(move || {
            // A signal or a dropped handle both abort the in-flight request.
            while !worker_cancel.load(Ordering::Acquire) {
                match cancel_rx.recv_timeout(CANCEL_POLL) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        worker_cancel.store(true, Ordering::Release);
                        return;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        thread::spawn(move || {
            let result = transport.request(&method, payload, &cancel);
            // Release the cancellation watcher once the request settles.
            cancel.store(true, Ordering::Release);
            let _ = done_tx.send(result);
        });
        (done_rx, cancel_tx)
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(Error::Runtime)?)
}

fn default_client() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .cookie_store(true)
        .build()?)
}

async fn wait_for_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(CANCEL_POLL).await;
    }
}

async fn decode_response(response: reqwest::Response, rpc_id: &str) -> Result<Value, Error> {
    let full_content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let content_type = full_content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if content_type.eq_ignore_ascii_case("multipart/form-data") {
        let bytes = response.bytes().await?;
        return Ok(crate::binary_response::parse_binary_response(
            &full_content_type,
            &bytes,
            rpc_id,
        )?);
    }
    let envelope: ResponseEnvelope = response.json().await?;
    match envelope.validate(rpc_id)? {
        ResponseResult::Success { value, .. } => Ok(value.clone()),
        ResponseResult::Failure { error, .. } => Err(Error::Remote {
            code: error.code.clone(),
            message: error.message.clone(),
            details: error.details.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_exchanges_launch_token_for_answering_api_cookie() {
        use httptest::{matchers::*, responders::status_code, Expectation, Server};

        let server = Server::run();
        server.expect(
            Expectation::matching(request::method_path("GET", "/connect")).respond_with(
                status_code(302)
                    .append_header("Location", "/api/remote.mux")
                    .append_header("Set-Cookie", "dsh-auth-test=operator; Path=/; HttpOnly"),
            ),
        );
        let launch_url = format!("{}?token=launch", server.url_str("/"));
        let (base_url, transport) = HttpTransport::authenticate(&launch_url).unwrap();
        assert_eq!(base_url, server.url_str("/"));
        assert_eq!(
            transport.credential().unwrap().cookie,
            "dsh-auth-test=operator"
        );
    }

    #[test]
    fn cancelled_request_fails_before_network() {
        let transport = HttpTransport::new("http://127.0.0.1/").unwrap();
        let flag = AtomicBool::new(true);
        assert!(matches!(
            transport.request("api", serde_json::json!({}), &flag),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn cancellation_aborts_a_blocked_request() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Instant;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            // Hold the response open until the client aborts the connection.
            let mut sink = [0_u8; 1024];
            let _ = stream.read(&mut sink);
            let _ = stream.write_all(b"HTTP/1.1 500 Late\r\n\r\n");
        });
        let transport = HttpTransport::new(&format!("http://{address}")).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let started = Instant::now();
        let worker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            worker_cancel.store(true, Ordering::Release);
        });
        let result = transport.request("slow", serde_json::json!({}), &cancel);
        assert!(matches!(result, Err(Error::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
        worker.join().unwrap();
        server.join().unwrap();
    }
}
