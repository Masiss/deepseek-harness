//! Typed prompt and cancellation requests for the Host session controller.
//!
//! The request identifiers are client-owned and stable across retries so the
//! Host can apply prompt admission idempotently.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::{transport::HttpTransport, Error};

/// A prompt part accepted by `session/prompt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PromptContentPart {
    /// User-authored text.
    #[serde(rename = "text")]
    Text { text: String },
    /// An inline image encoded as base64 data.
    #[serde(rename = "image")]
    Image {
        #[serde(rename = "mediaType")]
        media_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// An opaque receipt from a preceding file upload.
    #[serde(rename = "file")]
    File {
        #[serde(rename = "receiptId")]
        receipt_id: String,
    },
}

/// Prompt delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptMode {
    Queue,
    Steer,
}

/// Wire request for `session/prompt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub request_id: String,
    pub session_id: String,
    pub mode: PromptMode,
    pub content: Vec<PromptContentPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_time_zone: Option<String>,
}

/// Wire acceptance returned by prompt and cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedResponse {
    pub accepted: bool,
}

/// Wire request for `session/cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRequest {
    pub session_id: String,
}

/// Native chat client for prompt admission and active-turn cancellation.
pub struct ChatClient<'a> {
    transport: &'a HttpTransport,
    next_request_id: Arc<AtomicU64>,
}

impl<'a> ChatClient<'a> {
    /// Creates a chat client with request identifiers independent of transport RPC ids.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self {
            transport,
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Generates a unique request id for this chat client.
    pub fn next_request_id(&self) -> String {
        format!(
            "native-chat-{}",
            self.next_request_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Submits one prompt and returns the Host acceptance receipt.
    pub fn prompt(&self, request: PromptRequest) -> Result<AcceptedResponse, Error> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let value =
            self.transport
                .request("session/prompt", serde_json::to_value(request)?, &cancel)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Cancels the active turn for a session.
    pub fn cancel(&self, session_id: impl Into<String>) -> Result<AcceptedResponse, Error> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let value = self.transport.request(
            "session/cancel",
            json!(CancelRequest {
                session_id: session_id.into()
            }),
            &cancel,
        )?;
        Ok(serde_json::from_value(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_prompt_request() {
        let request = PromptRequest {
            request_id: "request-1".into(),
            session_id: "session-1".into(),
            mode: PromptMode::Queue,
            content: vec![
                PromptContentPart::Text {
                    text: "hello".into(),
                },
                PromptContentPart::Image {
                    media_type: "image/png".into(),
                    data: "AQI=".into(),
                    name: Some("a.png".into()),
                },
            ],
            client_time_zone: Some("UTC".into()),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "requestId":"request-1", "sessionId":"session-1", "mode":"queue",
                "content":[{"type":"text","text":"hello"},{"type":"image","mediaType":"image/png","data":"AQI=","name":"a.png"}],
                "clientTimeZone":"UTC"
            })
        );
    }

    #[test]
    fn serializes_accepted_response() {
        assert_eq!(
            serde_json::to_string(&AcceptedResponse { accepted: true }).unwrap(),
            r#"{"accepted":true}"#
        );
    }

    #[test]
    fn serializes_cancel_request() {
        assert_eq!(
            serde_json::to_value(CancelRequest {
                session_id: "s".into()
            })
            .unwrap(),
            json!({"session_id":"s"})
        );
    }

    #[test]
    fn request_ids_are_unique_per_client() {
        let transport = HttpTransport::new("http://127.0.0.1/").unwrap();
        let client = ChatClient::new(&transport);
        assert_ne!(client.next_request_id(), client.next_request_id());
    }
}
