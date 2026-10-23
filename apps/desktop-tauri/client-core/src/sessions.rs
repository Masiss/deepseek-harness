//! Typed session-list management RPCs for the Desktop session screen.
//!
//! The request and response structs follow `session-controller` wire types;
//! optional fields are omitted exactly as in the web client.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchRequest {
    pub query: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchItem {
    pub session_id: String,
    pub snippet: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchResponse {
    pub items: Vec<SessionSearchItem>,
    pub has_more: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_preset: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub session_id: String,
    #[serde(default)]
    pub agent_preset: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameSessionRequest {
    pub session_id: String,
    pub title: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameSessionResponse {
    pub title: String,
    pub seq: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkSessionRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_migration: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkSessionResponse {
    pub session_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedResponse {
    pub accepted: bool,
}

pub struct SessionsClient<'a> {
    transport: &'a HttpTransport,
}
impl<'a> SessionsClient<'a> {
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }
    pub fn search(&self, query: impl Into<String>) -> Result<SessionSearchResponse, Error> {
        self.call(
            "session/search",
            serde_json::to_value(SessionSearchRequest {
                query: query.into(),
            })?,
        )
    }
    pub fn create(&self, request: CreateSessionRequest) -> Result<Session, Error> {
        self.call("session/create", serde_json::to_value(request)?)
    }
    pub fn rename(&self, request: RenameSessionRequest) -> Result<RenameSessionResponse, Error> {
        self.call("session/rename", serde_json::to_value(request)?)
    }
    pub fn fork(&self, request: ForkSessionRequest) -> Result<ForkSessionResponse, Error> {
        self.call("session/fork", serde_json::to_value(request)?)
    }
    fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<T, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            serde_json::json!({"request": payload}),
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn omits_optional_create_fields() {
        assert_eq!(
            serde_json::to_value(CreateSessionRequest {
                workspace_id: None,
                cwd: None,
                session_id: None,
                agent_preset: None
            })
            .unwrap(),
            serde_json::json!({})
        );
    }
    #[test]
    fn serializes_rename() {
        assert_eq!(
            serde_json::to_value(RenameSessionRequest {
                session_id: "s".into(),
                title: "T".into()
            })
            .unwrap(),
            serde_json::json!({"sessionId":"s","title":"T"})
        );
    }
}
