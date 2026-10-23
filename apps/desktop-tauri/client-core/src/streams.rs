//! The remaining Desktop stream endpoints, over the shared mux.
//!
//! Each function opens one logical stream and decodes its frames. Payload types
//! mirror the Host Remote declarations; fields the native shell does not consume
//! stay in the typed struct so the wire contract is checked at the boundary.

use crate::{
    remote_stream::RemoteStreamMux,
    stream::{open, FrameStream},
    workspace_files::{WorkspaceFileScope, WorkspaceFileWatchFrame},
    Error,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A Workspace delivered by `workspace/follow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub workspace_id: String,
    pub path: String,
    pub title: String,
    pub session_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Complete Workspace state carried by `baseline` and `replace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBaseline {
    pub items: Vec<WorkspaceView>,
    pub archived_session_ids: Vec<String>,
    pub pinned_session_ids: Vec<String>,
}

/// One `workspace/follow` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum WorkspaceFollowFrame {
    Baseline { value: WorkspaceBaseline },
    Replace { value: WorkspaceBaseline },
    Upsert { workspace: WorkspaceView },
    Remove { workspace_id: String },
    Order { workspace_ids: Vec<String> },
    Archived { archived_session_ids: Vec<String> },
    Pinned { pinned_session_ids: Vec<String> },
}

/// Opens the Workspace generation.
pub fn workspace_follow(
    mux: &RemoteStreamMux,
) -> Result<FrameStream<WorkspaceFollowFrame>, Error> {
    open(mux, "workspace/follow", serde_json::json!({}))
}

/// A job row as delivered by `job/list` and `job/follow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_limit_bytes: Option<u64>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub output: JobOutput,
}

/// Output ring summary attached to a job row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobOutput {
    pub total: u64,
    pub earliest: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spill_paths: Option<Vec<String>>,
}

/// One chunk of job output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobChunk {
    pub at: i64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_before: Option<bool>,
}

/// Complete row set for one Session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRowsFrame {
    #[serde(rename = "type")]
    pub frame_type: String,
    pub jobs: Vec<JobView>,
}

/// One `job/follow` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum JobFollowFrame {
    Opened { job: JobView, from: u64 },
    Output {
        chunks: Vec<JobChunk>,
        next: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lossy: Option<bool>,
    },
    Status { job: JobView },
}

/// Request for `job/follow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFollowRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<u64>,
}

/// Lists every job row for a Session.
pub fn job_list(mux: &RemoteStreamMux, session_id: &str) -> Result<FrameStream<JobRowsFrame>, Error> {
    open(
        mux,
        "job/list",
        serde_json::json!({"request": {"sessionId": session_id}}),
    )
}

/// Follows one job's output and status.
pub fn job_follow(
    mux: &RemoteStreamMux,
    request: JobFollowRequest,
) -> Result<FrameStream<JobFollowFrame>, Error> {
    open(mux, "job/follow", serde_json::json!({"request": request}))
}

/// Projection values for one Session at a sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProjectionBaseline {
    pub as_of_seq: i64,
    pub values: BTreeMap<String, Value>,
}

/// Complete projection baseline for every Session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControlBaseline {
    pub projections: BTreeMap<String, SessionProjectionBaseline>,
}

/// One `session/control` frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum SessionControlFrame {
    Baseline {
        value: SessionControlBaseline,
    },
    Projection {
        session_id: String,
        key: String,
        value: Value,
        seq: i64,
    },
}

/// Opens the projection generation.
pub fn session_control(
    mux: &RemoteStreamMux,
) -> Result<FrameStream<SessionControlFrame>, Error> {
    open(mux, "session/control", serde_json::json!({}))
}

/// Account link destinations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLinks {
    pub usage_url: String,
    pub top_up_url: String,
}

/// Sign-in attempt attached to an account view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInAttemptView {
    pub id: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorize_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// Account state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub status: String,
    pub links: AccountLinks,
    pub attempt: Option<SignInAttemptView>,
}

/// Receives account snapshots.
pub fn account_watch(mux: &RemoteStreamMux) -> Result<FrameStream<AccountView>, Error> {
    open(mux, "account/watch", serde_json::json!({}))
}

/// Receives the single `session-expired` notice.
pub fn account_watch_expiry(mux: &RemoteStreamMux) -> Result<FrameStream<String>, Error> {
    open(mux, "account/watchExpiry", serde_json::json!({}))
}

/// Resource named by one access grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessResource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// Actions allowed on one resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAccessGrant {
    pub resource: RemoteAccessResource,
    pub actions: Vec<String>,
}

/// One `remoteAccess/follow` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RemoteAccessFrame {
    Baseline {
        owner: bool,
        #[serde(default)]
        grants: Vec<RemoteAccessGrant>,
    },
    Upsert {
        grant: RemoteAccessGrant,
    },
    Remove {
        resource: RemoteAccessResource,
    },
}

/// Receives this Peer's access snapshot.
pub fn remote_access_follow(
    mux: &RemoteStreamMux,
) -> Result<FrameStream<RemoteAccessFrame>, Error> {
    open(mux, "remoteAccess/follow", serde_json::json!({}))
}

/// Remaining wait advertised for one user question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserQuestionWait {
    pub remaining_ms: u64,
}

/// Follows the wait window of one user question.
///
/// The Host resolves the owning Agent from `agentId`, so both identities are required.
pub fn user_question_wait(
    mux: &RemoteStreamMux,
    agent_id: &str,
    call_id: &str,
) -> Result<FrameStream<UserQuestionWait>, Error> {
    open(
        mux,
        "userQuestions/attachWait",
        serde_json::json!({"agentId": agent_id, "callId": call_id}),
    )
}

/// Watches one workspace file for changes.
pub fn workspace_file_changes(
    mux: &RemoteStreamMux,
    scope: &WorkspaceFileScope,
    path: &str,
) -> Result<FrameStream<WorkspaceFileWatchFrame>, Error> {
    open(
        mux,
        "workspaceFiles/changes",
        serde_json::json!({"workspaceFileScopeId": scope.session_id, "path": path}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_stream::tests::{read, send, server};
    use crate::remote_stream::RemoteStreamMux;

    fn expect_open(
        socket: &mut tungstenite::WebSocket<std::net::TcpStream>,
        endpoint: &str,
        payload: &Value,
    ) -> Value {
        let open = read(socket);
        assert_eq!(open["type"], "open");
        assert_eq!(open["endpoint"], endpoint);
        assert_eq!(&open["payload"], payload);
        open
    }

    #[test]
    fn workspace_follow_decodes_baseline_and_increments() {
        let (url, credential, worker) = server(|mut socket| {
            let open = expect_open(&mut socket, "workspace/follow", &serde_json::json!({"args":{}}));
            let id = &open["streamId"];
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":id,"value":{
                    "type":"baseline",
                    "value":{
                        "items":[{"workspaceId":"w1","path":"/p","title":"T","sessionIds":["s1"],"createdAt":"c","updatedAt":"u"}],
                        "archivedSessionIds":[],"pinnedSessionIds":["s1"]
                    }
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":id,"value":{"type":"pinned","pinnedSessionIds":["s1","s2"]}}),
            );
            send(&mut socket, serde_json::json!({"type":"end","streamId":id}));
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream = workspace_follow(&mux).unwrap();
        let WorkspaceFollowFrame::Baseline { value } = stream.next().unwrap().unwrap() else {
            panic!("expected baseline")
        };
        assert_eq!(value.items[0].title, "T");
        assert_eq!(value.pinned_session_ids, vec!["s1"]);
        assert_eq!(
            stream.next().unwrap().unwrap(),
            WorkspaceFollowFrame::Pinned {
                pinned_session_ids: vec!["s1".into(), "s2".into()]
            }
        );
        assert!(stream.next().is_none());
        worker.join().unwrap();
    }

    #[test]
    fn job_list_and_follow_decode_typed_rows_and_output() {
        let (url, credential, worker) = server(|mut socket| {
            let open = expect_open(
                &mut socket,
                "job/list",
                &serde_json::json!({"args":{"request":{"sessionId":"s1"}}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{
                    "type":"rows","jobs":[{
                        "id":"j1","kind":"bash","label":"Build","status":"running",
                        "startedAt":1,"output":{"total":2,"earliest":0}
                    }]
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
            let follow = expect_open(
                &mut socket,
                "job/follow",
                &serde_json::json!({"args":{"request":{"jobId":"j1","from":4}}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":follow["streamId"],"value":{
                    "type":"output","next":6,"chunks":[{"at":5,"text":"ok\n"}]
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":follow["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut list = job_list(&mux, "s1").unwrap();
        let rows = list.next().unwrap().unwrap();
        assert_eq!(rows.frame_type, "rows");
        assert_eq!(rows.jobs[0].label, "Build");
        let _ = &list;
        let mut follow = job_follow(
            &mux,
            JobFollowRequest {
                session_id: None,
                job_id: "j1".into(),
                from: Some(4),
            },
        )
        .unwrap();
        let JobFollowFrame::Output { chunks, next, .. } = follow.next().unwrap().unwrap() else {
            panic!("expected output")
        };
        assert_eq!(next, 6);
        assert_eq!(chunks[0].text, "ok\n");
        worker.join().unwrap();
    }

    #[test]
    fn session_control_decodes_projection_updates() {
        let (url, credential, worker) = server(|mut socket| {
            let open = expect_open(
                &mut socket,
                "session/control",
                &serde_json::json!({"args":{}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{
                    "type":"baseline","value":{"projections":{}}
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{
                    "type":"projection","sessionId":"s1","key":"title","value":"Hello","seq":9
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut stream = session_control(&mux).unwrap();
        assert!(matches!(
            stream.next().unwrap().unwrap(),
            SessionControlFrame::Baseline { .. }
        ));
        assert_eq!(
            stream.next().unwrap().unwrap(),
            SessionControlFrame::Projection {
                session_id: "s1".into(),
                key: "title".into(),
                value: serde_json::json!("Hello"),
                seq: 9
            }
        );
        worker.join().unwrap();
    }

    #[test]
    fn account_remote_access_and_wait_frames_decode() {
        let (url, credential, worker) = server(|mut socket| {
            let watch = expect_open(
                &mut socket,
                "account/watch",
                &serde_json::json!({"args":{}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":watch["streamId"],"value":{
                    "status":"signed-out","attempt":null,
                    "links":{"usageUrl":"u","topUpUrl":"t"}
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":watch["streamId"]}),
            );
            let expiry = expect_open(
                &mut socket,
                "account/watchExpiry",
                &serde_json::json!({"args":{}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":expiry["streamId"],"value":"session-expired"}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":expiry["streamId"]}),
            );
            let access = expect_open(
                &mut socket,
                "remoteAccess/follow",
                &serde_json::json!({"args":{}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":access["streamId"],"value":{"type":"baseline","owner":true}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":access["streamId"]}),
            );
            let wait = expect_open(
                &mut socket,
                "userQuestions/attachWait",
                &serde_json::json!({"args":{"agentId":"a1","callId":"call-1"}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":wait["streamId"],"value":{"remainingMs":1500}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":wait["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let mut account = account_watch(&mux).unwrap();
        assert_eq!(account.next().unwrap().unwrap().status, "signed-out");
        let _ = &account;
        let mut expiry = account_watch_expiry(&mux).unwrap();
        assert_eq!(expiry.next().unwrap().unwrap(), "session-expired");
        let _ = &expiry;
        let mut access = remote_access_follow(&mux).unwrap();
        assert_eq!(
            access.next().unwrap().unwrap(),
            RemoteAccessFrame::Baseline {
                owner: true,
                grants: Vec::new()
            }
        );
        let _ = &access;
        let mut wait = user_question_wait(&mux, "a1", "call-1").unwrap();
        assert_eq!(wait.next().unwrap().unwrap().remaining_ms, 1500);
        worker.join().unwrap();
    }

    #[test]
    fn workspace_file_changes_decodes_ready_and_change_frames() {
        let (url, credential, worker) = server(|mut socket| {
            let open = expect_open(
                &mut socket,
                "workspaceFiles/changes",
                &serde_json::json!({"args":{"workspaceFileScopeId":"s1","path":"a.txt"}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{"kind":"ready"}}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"item","streamId":open["streamId"],"value":{
                    "kind":"change","change":{"absolutePath":"/w/a.txt","version":"v2"}
                }}),
            );
            send(
                &mut socket,
                serde_json::json!({"type":"end","streamId":open["streamId"]}),
            );
        });
        let mux = RemoteStreamMux::connect(&url, &credential).unwrap();
        let scope = WorkspaceFileScope {
            session_id: "s1".into(),
        };
        let mut stream = workspace_file_changes(&mux, &scope, "a.txt").unwrap();
        assert_eq!(
            stream.next().unwrap().unwrap(),
            WorkspaceFileWatchFrame::Ready
        );
        assert!(matches!(
            stream.next().unwrap().unwrap(),
            WorkspaceFileWatchFrame::Change { .. }
        ));
        worker.join().unwrap();
    }

    #[test]
    fn unknown_frame_tags_are_rejected() {
        assert!(serde_json::from_value::<WorkspaceFollowFrame>(
            serde_json::json!({"type":"other"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<SessionControlFrame>(serde_json::json!({"type":"other"}))
                .is_err()
        );
    }
}
