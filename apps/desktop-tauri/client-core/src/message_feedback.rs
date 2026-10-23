//! Typed message-feedback RPCs for the Desktop chat surface.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// The human's overall judgment of one assistant message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MessageFeedbackRating {
    Positive,
    Negative,
}

/// One current feedback value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFeedbackItem {
    pub message_id: String,
    pub rating: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub version: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Request to create or replace feedback for one assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFeedbackPutRequest {
    pub session_id: String,
    pub message_id: String,
    pub rating: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub if_version: Option<String>,
}

/// Request to delete feedback for one assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageFeedbackDeleteRequest {
    pub session_id: String,
    pub message_id: String,
    pub if_version: String,
}

/// Native client for message feedback.
pub struct MessageFeedbackClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> MessageFeedbackClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists current feedback items for one session.
    pub fn list(&self, session_id: impl Into<String>) -> Result<Vec<MessageFeedbackItem>, Error> {
        let value = self.call(
            "messageFeedback/list",
            serde_json::json!({"request": {"sessionId": session_id.into()}}),
        )?;
        let items = value
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        items
            .into_iter()
            .map(|item| serde_json::from_value(item).map_err(Error::from))
            .collect()
    }

    /// Creates or replaces feedback for one assistant message.
    pub fn put(
        &self,
        request: MessageFeedbackPutRequest,
    ) -> Result<MessageFeedbackItem, Error> {
        let value = self.call("messageFeedback/put", serde_json::json!({"request": request}))?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let code = value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("feedback-rejected");
            let message = value
                .get("error")
                .and_then(|error| error.get("diagnostic"))
                .and_then(Value::as_str)
                .unwrap_or("feedback rejected")
                .to_owned();
            return Err(Error::Remote {
                code: code.to_owned(),
                message,
                details: serde_json::Map::new(),
            });
        }
        serde_json::from_value(value.get("value").cloned().unwrap_or(Value::Null))
            .map_err(Error::from)
    }

    /// Deletes feedback for one assistant message.
    pub fn delete(&self, request: MessageFeedbackDeleteRequest) -> Result<(), Error> {
        let value = self.call("messageFeedback/delete", serde_json::json!({"request": request}))?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            let code = value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .unwrap_or("feedback-rejected");
            let message = value
                .get("error")
                .and_then(|error| error.get("diagnostic"))
                .and_then(Value::as_str)
                .unwrap_or("feedback rejected")
                .to_owned();
            return Err(Error::Remote {
                code: code.to_owned(),
                message,
                details: serde_json::Map::new(),
            });
        }
        Ok(())
    }

    fn call(&self, method: &str, payload: Value) -> Result<Value, Error> {
        Ok(self.transport.request(method, payload, &AtomicBool::new(false))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_put_request() {
        let request = MessageFeedbackPutRequest {
            session_id: "s".into(),
            message_id: "m".into(),
            rating: "positive".into(),
            note: Some("great".into()),
            category: None,
            if_version: None,
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({
                "sessionId": "s",
                "messageId": "m",
                "rating": "positive",
                "note": "great",
                "ifVersion": null
            })
        );
    }
}
