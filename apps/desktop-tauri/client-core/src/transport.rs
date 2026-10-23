//! Plain HTTP transport owns cancellation and decoding so clients share identical request semantics.

use crate::{
    auth::{credential_from_redirect, Credential},
    envelope::{RequestEnvelope, ResponseEnvelope, ResponseResult},
    Error,
};
use serde_json::Value;
use std::sync::mpsc;
use std::thread;

#[derive(Clone)]
pub struct HttpTransport {
    client: reqwest::blocking::Client,
    base_url: reqwest::Url,
    next_id: std::sync::Arc<std::sync::atomic::AtomicU64>,
    credential: Option<Credential>,
}
impl HttpTransport {
    pub fn new(base_url: &str) -> Result<Self, Error> {
        Self::with_client(
            base_url,
            reqwest::blocking::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(30))
                .cookie_store(true)
                .build()?,
        )
    }

    /// Builds a transport from an already configured cookie-preserving client.
    pub fn with_client(base_url: &str, client: reqwest::blocking::Client) -> Result<Self, Error> {
        Ok(Self {
            client,
            base_url: reqwest::Url::parse(base_url)
                .map_err(|source| Error::InvalidUrl { source })?,
            next_id: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
            credential: None,
        })
    }

    /// Exchanges a Host launch URL and returns the clean redirected base URL plus transport.
    pub fn authenticate(launch_url: &str) -> Result<(String, Self), Error> {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let mut connect_url =
            reqwest::Url::parse(launch_url).map_err(|source| Error::InvalidUrl { source })?;
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
        let response = client.get(connect_url).send()?;
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
    pub fn base_url(&self) -> &reqwest::Url {
        &self.base_url
    }

    /// Returns the session cookie required by the Gateway WebSocket mux.
    pub fn credential(&self) -> Result<&Credential, Error> {
        self.credential.as_ref().ok_or(Error::MissingCredential)
    }
    pub fn request(
        &self,
        method: &str,
        payload: Value,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, Error> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let id = format!(
            "native-{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let url = self
            .base_url
            .join(&format!("api/{method}"))
            .map_err(|source| Error::InvalidUrl { source })?;
        let response = self
            .client
            .post(url)
            .json(&RequestEnvelope::new(id.clone(), method, payload))
            .send()?;
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        if !response.status().is_success() {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        decode_response(response, &id)
    }
    pub fn request_cancellable(
        &self,
        method: String,
        payload: Value,
    ) -> (mpsc::Receiver<Result<Value, Error>>, mpsc::Sender<()>) {
        let id = format!(
            "native-{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let (done_tx, done_rx) = mpsc::channel();
        let (cancel_tx, cancel_rx) = mpsc::channel();
        let client = self.client.clone();
        let base = self.base_url.clone();
        let method = method.clone();
        thread::spawn(move || {
            if cancel_rx.try_recv().is_ok() {
                let _ = done_tx.send(Err(Error::Cancelled));
                return;
            }
            let result = (|| {
                let response = client
                    .post(
                        base.join(&format!("api/{method}"))
                            .map_err(|source| Error::InvalidUrl { source })?,
                    )
                    .json(&RequestEnvelope::new(id.clone(), &method, payload))
                    .send()?;
                if response.status().is_success() {
                    decode_response(response, &id)
                } else {
                    Err(Error::HttpStatus(response.status().as_u16()))
                }
            })();
            let _ = done_tx.send(result);
        });
        (done_rx, cancel_tx)
    }
}

fn decode_response(response: reqwest::blocking::Response, rpc_id: &str) -> Result<Value, Error> {
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
        let bytes = response.bytes()?;
        return Ok(crate::binary_response::parse_binary_response(
            &full_content_type,
            &bytes,
            rpc_id,
        )?);
    }
    let envelope: ResponseEnvelope = response.json()?;
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
        let flag = std::sync::atomic::AtomicBool::new(true);
        assert!(matches!(
            transport.request("api", serde_json::json!({}), &flag),
            Err(Error::Cancelled)
        ));
    }
}
