//! Typed Workspace navigation RPC for the Desktop session surfaces.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Complete archived Session set after an archive mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceArchiveValue {
    pub archived_session_ids: Vec<String>,
}

/// Complete pin set after a pin mutation, most recently pinned first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePinValue {
    pub pinned_session_ids: Vec<String>,
}

/// Native client for Workspace session grouping mutations.
pub struct WorkspaceClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> WorkspaceClient<'a> {
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Archives one Session; with `stop_activity`, running work is stopped
    /// first instead of refused as `workspace/session-active`.
    pub fn archive_session(
        &self,
        session_id: &str,
        stop_activity: bool,
    ) -> Result<WorkspaceArchiveValue, Error> {
        self.call(
            "workspace/archiveSession",
            serde_json::json!({"sessionId": session_id, "stopActivity": stop_activity}),
        )
    }

    /// Restores one archived Session to Workspace grouping surfaces.
    pub fn unarchive_session(&self, session_id: &str) -> Result<WorkspaceArchiveValue, Error> {
        self.call(
            "workspace/unarchiveSession",
            serde_json::json!({"sessionId": session_id}),
        )
    }

    /// Surfaces one known unarchived Session ahead of unpinned Sessions.
    pub fn pin_session(&self, session_id: &str) -> Result<WorkspacePinValue, Error> {
        self.call(
            "workspace/pinSession",
            serde_json::json!({"sessionId": session_id}),
        )
    }

    /// Removes one Session's pin.
    pub fn unpin_session(&self, session_id: &str) -> Result<WorkspacePinValue, Error> {
        self.call(
            "workspace/unpinSession",
            serde_json::json!({"sessionId": session_id}),
        )
    }

    fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        payload: serde_json::Value,
    ) -> Result<T, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            payload,
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_grouping_calls_use_the_gateway_methods_and_arguments() {
        use httptest::{matchers::*, responders::json_encoded, Expectation, Server};
        let server = Server::run();
        let archive = serde_json::json!({"archivedSessionIds": ["s1"]});
        let pin = serde_json::json!({"pinnedSessionIds": ["s1"]});
        let cases = [
            (
                "workspace/archiveSession",
                serde_json::json!({"sessionId": "s1", "stopActivity": false}),
                archive.clone(),
            ),
            (
                "workspace/unarchiveSession",
                serde_json::json!({"sessionId": "s1"}),
                archive.clone(),
            ),
            (
                "workspace/pinSession",
                serde_json::json!({"sessionId": "s1"}),
                pin.clone(),
            ),
            (
                "workspace/unpinSession",
                serde_json::json!({"sessionId": "s1"}),
                pin.clone(),
            ),
        ];
        for (index, (method, args, value)) in cases.into_iter().enumerate() {
            let rpc = format!("native-{}", index + 1);
            server.expect(
                Expectation::matching(all_of![
                    request::method_path("POST", format!("/api/{method}")),
                    request::body(json_decoded(eq(serde_json::json!({
                        "type": "client-request",
                        "rpcId": rpc,
                        "method": method,
                        "payload": {"args": args}
                    }))))
                ])
                .respond_with(json_encoded(serde_json::json!({
                    "type": "server-response",
                    "rpcId": rpc,
                    "result": {"ok": true, "value": value}
                }))),
            );
        }
        let transport = HttpTransport::new(&server.url_str("/")).unwrap();
        let client = WorkspaceClient::new(&transport);
        assert_eq!(
            client.archive_session("s1", false).unwrap().archived_session_ids,
            ["s1"]
        );
        assert_eq!(
            client.unarchive_session("s1").unwrap().archived_session_ids,
            ["s1"]
        );
        assert_eq!(
            client.pin_session("s1").unwrap().pinned_session_ids,
            ["s1"]
        );
        assert_eq!(
            client.unpin_session("s1").unwrap().pinned_session_ids,
            ["s1"]
        );
    }
}
