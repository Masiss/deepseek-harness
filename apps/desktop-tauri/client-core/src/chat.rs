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
#[serde(rename_all = "camelCase")]
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
                .request("session/prompt", json!({"request": request}), &cancel)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Cancels the active turn for a session.
    pub fn cancel(&self, session_id: impl Into<String>) -> Result<AcceptedResponse, Error> {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let value = self.transport.request(
            "session/cancel",
            json!({"request": CancelRequest {
                session_id: session_id.into()
            }}),
            &cancel,
        )?;
        Ok(serde_json::from_value(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rpc_args<T>(
        method: &str,
        args: serde_json::Value,
        call: impl FnOnce(&HttpTransport) -> Result<T, Error>,
    ) {
        use httptest::{matchers::*, responders::json_encoded, Expectation, Server};

        let server = Server::run();
        server.expect(
            Expectation::matching(all_of![
                request::method_path("POST", format!("/api/{method}")),
                request::body(json_decoded(eq(json!({
                    "type": "client-request", "rpcId": "native-1", "method": method,
                    "payload": {"args": args}
                }))))
            ])
            .respond_with(json_encoded(json!({
                "type": "server-response", "rpcId": "native-1",
                "result": {"ok": false, "error": {
                    "code": "test/received", "message": "received", "details": {}
                }}
            }))),
        );
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let transport = HttpTransport::with_client(&server.url_str("/"), client).unwrap();
        assert!(
            matches!(call(&transport), Err(Error::Remote {code, ..}) if code == "test/received")
        );
    }

    #[test]
    fn unary_rpcs_match_server_named_parameters() {
        use crate::{
            models::*, plugins::*, schedules::*, session::*, sessions::*, settings::*, skills::*,
            workspace_files::*,
        };

        assert_rpc_args(
            "session/prompt",
            json!({"request": {
                "requestId": "r", "sessionId": "s", "mode": "queue",
                "content": [{"type": "text", "text": "hello"}]
            }}),
            |t| {
                ChatClient::new(t).prompt(PromptRequest {
                    request_id: "r".into(),
                    session_id: "s".into(),
                    mode: PromptMode::Queue,
                    content: vec![PromptContentPart::Text {
                        text: "hello".into(),
                    }],
                    client_time_zone: None,
                })
            },
        );
        assert_rpc_args(
            "session/cancel",
            json!({"request": {"sessionId": "s"}}),
            |t| ChatClient::new(t).cancel("s"),
        );
        assert_rpc_args("session/list", json!({"_request": {}}), |t| {
            SessionViewer::new(t).list(None)
        });
        assert_rpc_args("session/list", json!({"_request": {"cursor": "c"}}), |t| {
            SessionViewer::new(t).list(Some("c"))
        });
        assert_rpc_args(
            "session/page",
            json!({"request": {"address": {"kind": "session", "sessionId": "s"}, "throughSeq": 3}}),
            |t| SessionViewer::new(t).page("s", 3),
        );
        assert_rpc_args("session/search", json!({"request": {"query": "q"}}), |t| {
            SessionsClient::new(t).search("q")
        });
        assert_rpc_args("session/create", json!({"request": {}}), |t| {
            SessionsClient::new(t).create(CreateSessionRequest {
                workspace_id: None,
                cwd: None,
                session_id: None,
                agent_preset: None,
            })
        });
        assert_rpc_args(
            "session/rename",
            json!({"request": {"sessionId": "s", "title": "T"}}),
            |t| {
                SessionsClient::new(t).rename(RenameSessionRequest {
                    session_id: "s".into(),
                    title: "T".into(),
                })
            },
        );
        assert_rpc_args(
            "session/fork",
            json!({"request": {"sessionId": "s", "atSeq": 2, "allowMigration": false}}),
            |t| {
                SessionsClient::new(t).fork(ForkSessionRequest {
                    session_id: "s".into(),
                    at_seq: Some(2),
                    allow_migration: Some(false),
                })
            },
        );
        assert_rpc_args("session/modelCatalog", json!({}), |t| {
            ModelClient::new(t).catalog()
        });
        assert_rpc_args("session/initializeDefaultModel", json!({}), |t| {
            ModelClient::new(t).initialize_default_model()
        });
        assert_rpc_args(
            "session/selectModel",
            json!({"request": {"sessionId": "s", "provider": "p", "model": "m", "reasoningEffort": "high"}}),
            |t| {
                ModelClient::new(t).select(SelectModelRequest {
                    session_id: "s".into(),
                    selection: ModelSelection {
                        provider: "p".into(),
                        model: "m".into(),
                        reasoning_effort: Some("high".into()),
                    },
                })
            },
        );
        assert_rpc_args("skills/list", json!({"request": {"sessionId": "s"}}), |t| {
            SkillsClient::new(t).list("s")
        });
        assert_rpc_args("skills/list", json!({"request": {"sessionId": "s"}}), |t| {
            SkillsClient::new(t).session_list("s")
        });
        assert_rpc_args("settings/describe", json!({}), |t| {
            SettingsClient::new(t).describe()
        });
        assert_rpc_args("settings/openSettingsDocument", json!({}), |t| {
            SettingsClient::new(t).open_settings_document()
        });
        assert_rpc_args(
            "settings/update",
            json!({"ns": "n", "patch": {"x": true}, "expectedRevision": 4}),
            |t| {
                SettingsClient::new(t).update(SettingsUpdateRequest {
                    ns: "n".into(),
                    patch: serde_json::from_value(json!({"x": true})).unwrap(),
                    expected_revision: Some(4),
                })
            },
        );
        assert_rpc_args("settings/replace", json!({"ns": "n", "section": {}}), |t| {
            SettingsClient::new(t).replace(SettingsReplaceRequest {
                ns: "n".into(),
                section: Default::default(),
                expected_revision: None,
            })
        });
        assert_rpc_args(
            "settings/mutate",
            json!({"ns": "n", "ops": [{"op": "unset", "path": ["x"]}]}),
            |t| {
                SettingsClient::new(t).mutate(SettingsMutateRequest {
                    ns: "n".into(),
                    ops: vec![SettingsPathOp::Unset {
                        path: vec!["x".into()],
                    }],
                    expected_revision: None,
                })
            },
        );
        assert_rpc_args(
            "schedule/list",
            json!({"request": {"sessionId": "s"}}),
            |t| {
                SchedulesClient::new(t).list(ScheduleListRequest {
                    session_id: "s".into(),
                })
            },
        );
        assert_rpc_args("schedule/catalog", json!({}), |t| {
            SchedulesClient::new(t).catalog()
        });
        assert_rpc_args(
            "schedule/history",
            json!({"request": {"sessionId": "s", "id": "task", "limit": 25, "before": "b"}}),
            |t| {
                SchedulesClient::new(t).history(ScheduleHistoryRequest {
                    session_id: "s".into(),
                    id: "task".into(),
                    limit: 25,
                    before: Some("b".into()),
                })
            },
        );
        assert_rpc_args(
            "schedule/delete",
            json!({"request": {"sessionId": "s", "id": "task"}}),
            |t| {
                SchedulesClient::new(t).delete(ScheduleDeleteRequest {
                    session_id: "s".into(),
                    id: "task".into(),
                })
            },
        );
        assert_rpc_args(
            "schedule/update",
            json!({"request": {"sessionId": "s", "id": "task", "expected": {"kind": "at"}}}),
            |t| {
                SchedulesClient::new(t).update(ScheduleUpdateRequest {
                    session_id: "s".into(),
                    id: "task".into(),
                    expected: json!({"kind": "at"}),
                    title: None,
                    prompt: None,
                    change: None,
                })
            },
        );
        assert_rpc_args("pluginInventory/list", json!({}), |t| {
            PluginsClient::new(t).list()
        });
        assert_rpc_args(
            "workspaceFiles/read",
            json!({"workspaceFileScopeId": "s", "path": "a", "range": {"offset": 1, "limit": 5}}),
            |t| {
                WorkspaceFilesClient::new(t).read(
                    WorkspaceFileScope {
                        session_id: "s".into(),
                    },
                    "a",
                    WorkspaceFileRange {
                        offset: Some(1),
                        limit: Some(5),
                    },
                )
            },
        );
        assert_rpc_args(
            "workspaceFiles/readBytes",
            json!({"workspaceFileScopeId": "s", "path": "a", "options": {"range": {"offset": 2, "length": 4}, "baseFile": "b"}}),
            |t| {
                WorkspaceFilesClient::new(t).read_bytes(
                    WorkspaceFileScope {
                        session_id: "s".into(),
                    },
                    "a",
                    WorkspaceByteReadOptions {
                        range: Some(WorkspaceByteRange {
                            offset: Some(2),
                            length: Some(4),
                        }),
                        base_file: Some("b".into()),
                    },
                )
            },
        );
        assert_rpc_args(
            "workspaceFiles/stat",
            json!({"workspaceFileScopeId": "s", "path": "a"}),
            |t| {
                WorkspaceFilesClient::new(t).stat(
                    WorkspaceFileScope {
                        session_id: "s".into(),
                    },
                    "a",
                )
            },
        );
        assert_rpc_args(
            "workspaceFiles/list",
            json!({"workspaceFileScopeId": "s", "path": "a"}),
            |t| {
                WorkspaceFilesClient::new(t).list(
                    WorkspaceFileScope {
                        session_id: "s".into(),
                    },
                    "a",
                )
            },
        );
    }

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
            json!({"sessionId":"s"})
        );
    }

    #[test]
    fn request_ids_are_unique_per_client() {
        let transport = HttpTransport::new("http://127.0.0.1/").unwrap();
        let client = ChatClient::new(&transport);
        assert_ne!(client.next_request_id(), client.next_request_id());
    }
}
