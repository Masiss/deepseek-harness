//! Typed read-only access to the Host session inventory and durable event pages.
//!
//! This module deliberately models the browser's `session/list` and
//! `session/page` responses without interpreting or rewriting event payloads.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

use crate::{transport::HttpTransport, Error};

/// A session identity accepted by the Host history endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionAddress {
    pub kind: SessionAddressKind,
    pub session_id: String,
}

/// Address kind used by the Host session controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionAddressKind {
    Session,
}

/// One entry returned by `session/list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub agent_available: bool,
    #[serde(default)]
    pub format_status: Option<FormatStatus>,
    pub session_id: String,
    pub updated_at: i64,
    pub running: bool,
    pub blank: bool,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub projections: Option<Value>,
}

/// Persistence status reported for a session summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormatStatus {
    Current,
    MigrationRequired,
}

/// Exact persisted event envelope read by the web client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub seq: i64,
    pub time: i64,
    pub data: Value,
    #[serde(default)]
    pub ignorable: Option<bool>,
    #[serde(default)]
    pub source_event_seqs: Option<Value>,
    #[serde(default)]
    pub surface_op: Option<Value>,
}

/// A page record containing one durable event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEventEntry {
    #[serde(rename = "type")]
    pub record_type: String,
    pub event: SessionEvent,
}

/// A page returned by `session/page`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPage {
    pub records: Vec<SessionEventEntry>,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
}

/// Status derived from a tool call and its matching result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallStatus {
    Pending,
    Succeeded,
    Failed,
    Unknown,
}

/// One tool invocation and its observed terminal status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub status: ToolCallStatus,
}

/// Read-only client for session inventory, history, and tool status.
pub struct SessionViewer<'a> {
    transport: &'a HttpTransport,
}

impl<'a> SessionViewer<'a> {
    /// Create a viewer over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// List sessions using the inventory's cursor request.
    pub fn list(&self, cursor: Option<&str>) -> Result<Vec<SessionSummary>, Error> {
        let cancel = AtomicBool::new(false);
        let request = cursor.map_or_else(|| json!({}), |value| json!({"cursor": value}));
        let payload = json!({"_request": request});
        let value = self.transport.request("session/list", payload, &cancel)?;
        let response: SessionListResponse = serde_json::from_value(value)?;
        Ok(response.items)
    }

    /// Read one page of a session's durable event log.
    pub fn page(&self, session_id: &str, through_seq: i64) -> Result<SessionPage, Error> {
        let cancel = AtomicBool::new(false);
        let payload = json!({"request": {
            "address": {"kind": "session", "sessionId": session_id},
            "throughSeq": through_seq
        }});
        let value = self.transport.request("session/page", payload, &cancel)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Derive each tool call's status from the supplied event page.
    pub fn tool_calls(page: &SessionPage) -> Vec<ToolCall> {
        let mut calls = Vec::new();
        for entry in &page.records {
            match entry.event.event_type.as_str() {
                "tool/call" => {
                    let Some(call_id) = entry.event.data.get("callId").and_then(Value::as_str)
                    else {
                        continue;
                    };
                    let name = entry
                        .event
                        .data
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    calls.push(ToolCall {
                        call_id: call_id.to_owned(),
                        name,
                        status: ToolCallStatus::Pending,
                    });
                }
                "tool/result" => {
                    let call_id = entry
                        .event
                        .data
                        .get("message")
                        .and_then(|m| m.get("toolCallId"))
                        .and_then(Value::as_str);
                    if let Some(call_id) = call_id {
                        if let Some(call) =
                            calls.iter_mut().rev().find(|call| call.call_id == call_id)
                        {
                            call.status = if entry
                                .event
                                .data
                                .get("message")
                                .and_then(|m| m.get("isError"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                ToolCallStatus::Failed
                            } else {
                                ToolCallStatus::Succeeded
                            };
                        }
                    }
                }
                _ => {}
            }
        }
        calls
    }
}

#[derive(Debug, Deserialize)]
struct SessionListResponse {
    items: Vec<SessionSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fixture_and_reports_tool_statuses() {
        let page: SessionPage =
            serde_json::from_str(include_str!("../tests/fixtures/session-page.json")).unwrap();
        let calls = SessionViewer::tool_calls(&page);
        assert_eq!(
            calls
                .iter()
                .map(|call| (&call.call_id, &call.status))
                .collect::<Vec<_>>(),
            vec![
                (&"call-ok".to_owned(), &ToolCallStatus::Succeeded),
                (&"call-failed".to_owned(), &ToolCallStatus::Failed),
                (&"call-pending".to_owned(), &ToolCallStatus::Pending),
            ]
        );
    }
}
