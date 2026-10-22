//! Plain HTTP transport owns cancellation and decoding so clients share identical request semantics.

use crate::{
    envelope::{RequestEnvelope, ResponseEnvelope, ResponseResult},
    Error,
};
use serde_json::Value;
use std::sync::mpsc;
use std::thread;

pub struct HttpTransport {
    client: reqwest::blocking::Client,
    base_url: reqwest::Url,
    next_id: std::sync::atomic::AtomicU64,
}
impl HttpTransport {
    pub fn new(base_url: &str) -> Result<Self, Error> {
        Self::with_client(base_url, reqwest::blocking::Client::builder().cookie_store(true).build()?)
    }

    /// Builds a transport from an already configured cookie-preserving client.
    pub fn with_client(base_url: &str, client: reqwest::blocking::Client) -> Result<Self, Error> {
        Ok(Self {
            client,
            base_url: reqwest::Url::parse(base_url)
                .map_err(|source| Error::InvalidUrl { source })?,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Exchanges a Host launch URL and returns the clean redirected base URL plus transport.
    pub fn authenticate(launch_url: &str) -> Result<(String, Self), Error> {
        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        let response = client.get(launch_url).send()?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus(response.status().as_u16()));
        }
        let base_url = response.url().to_string();
        let base_url = base_url.trim_end_matches('/').to_owned();
        Ok((base_url.clone(), Self::with_client(&base_url, client)?))
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
            .join(method)
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
        let envelope: ResponseEnvelope = response.json()?;
        match envelope.validate(&id)? {
            ResponseResult::Success { value, .. } => Ok(value.clone()),
            ResponseResult::Failure { error, .. } => Err(Error::Remote {
                code: error.code.clone(),
                message: error.message.clone(),
            }),
        }
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
                        base.join(&method)
                            .map_err(|source| Error::InvalidUrl { source })?,
                    )
                    .json(&RequestEnvelope::new(id.clone(), &method, payload))
                    .send()?;
                if response.status().is_success() {
                    let envelope: ResponseEnvelope = response.json()?;
                    match envelope.validate(&id)? {
                        ResponseResult::Success { value, .. } => Ok(value.clone()),
                        ResponseResult::Failure { error, .. } => Err(Error::Remote {
                            code: error.code.clone(),
                            message: error.message.clone(),
                        }),
                    }
                } else {
                    Err(Error::HttpStatus(response.status().as_u16()))
                }
            })();
            let _ = done_tx.send(result);
        });
        (done_rx, cancel_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
