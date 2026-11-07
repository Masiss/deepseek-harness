//! Diagnostics snapshot RPC for configured language servers.
//!
//! The Host merges diagnostics from the language servers this deployment
//! configured or installed; the native shell only renders the snapshot.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Request diagnostics for one Session's workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsRequest {
    pub session_id: String,
}

/// Zero-based UTF-16 diagnostic coordinate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticPosition {
    pub line: u32,
    pub character: u32,
}

/// Zero-based UTF-16 diagnostic range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticRange {
    pub start: DiagnosticPosition,
    pub end: DiagnosticPosition,
}

/// One diagnostics item from a configured language server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticItem {
    pub uri: String,
    pub range: DiagnosticRange,
    pub severity: String,
    pub message: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

/// Diagnostics snapshot plus the workspace root the Host checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsResponse {
    pub workspace_root: String,
    pub items: Vec<DiagnosticItem>,
}

/// On-demand diagnose request for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseRequest {
    pub session_id: String,
    pub path: String,
}

/// Diagnostics returned for one diagnosed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseResponse {
    pub workspace_root: String,
    pub path: String,
    pub uri: String,
    pub items: Vec<DiagnosticItem>,
}

/// Typed `session/diagnostics` client.
pub struct DiagnosticsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> DiagnosticsClient<'a> {
    /// Binds the client to one authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Reads the merged diagnostics snapshot for one Session.
    pub fn list(&self, session_id: impl Into<String>) -> Result<DiagnosticsResponse, Error> {
        let payload = serde_json::to_value(DiagnosticsRequest {
            session_id: session_id.into(),
        })?;
        Ok(serde_json::from_value(self.transport.request(
            "session/diagnostics",
            serde_json::json!({"request": payload}),
            &AtomicBool::new(false),
        )?)?)
    }

    /// Opens one file in its configured language server and returns its diagnostics.
    pub fn diagnose(
        &self,
        session_id: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<DiagnoseResponse, Error> {
        let payload = serde_json::to_value(DiagnoseRequest {
            session_id: session_id.into(),
            path: path.into(),
        })?;
        Ok(serde_json::from_value(self.transport.request(
            "session/diagnose",
            serde_json::json!({"request": payload}),
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_the_session_request() {
        assert_eq!(
            serde_json::to_value(DiagnosticsRequest {
                session_id: "s".into()
            })
            .unwrap(),
            serde_json::json!({"sessionId": "s"})
        );
        assert_eq!(
            serde_json::to_value(DiagnoseRequest {
                session_id: "s".into(),
                path: "src/a.ts".into()
            })
            .unwrap(),
            serde_json::json!({"sessionId": "s", "path": "src/a.ts"})
        );
    }

    #[test]
    fn deserializes_items_and_optional_fields() {
        let response: DiagnosticsResponse = serde_json::from_value(serde_json::json!({
            "workspaceRoot": "/work",
            "items": [
                {
                    "uri": "file:///work/a.ts",
                    "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}},
                    "severity": "error",
                    "message": "bad",
                    "source": "ts",
                    "code": "1001"
                },
                {
                    "uri": "file:///work/b.ts",
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "severity": "warning",
                    "message": "ok"
                }
            ]
        }))
        .unwrap();
        assert_eq!(response.workspace_root, "/work");
        assert_eq!(response.items.len(), 2);
        assert_eq!(response.items[0].range.start.line, 1);
        assert_eq!(response.items[0].code.as_deref(), Some("1001"));
        assert!(response.items[1].source.is_none());
    }
}
