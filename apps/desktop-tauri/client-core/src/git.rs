//! Typed Git RPCs for the desktop client.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// One branch reported by `git/branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranch {
    pub name: String,
    pub commit: String,
    #[serde(default)]
    pub current: bool,
}

/// Line totals one side of a changed path reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileSideStats {
    pub additions: u64,
    pub deletions: u64,
}

/// Line totals one changed path reports for the sides that carry counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileStats {
    /// Index against HEAD; absent when that side reports no counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<GitFileSideStats>,
    /// Work tree against the index; absent when that side reports no counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<GitFileSideStats>,
}

/// One changed path from `git/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    pub path: String,
    pub status: String,
    #[serde(default)]
    pub staged: bool,
    #[serde(default)]
    pub worktree: bool,
    /// Source path of a renamed or copied change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    /// Per-side line totals; absent for untracked, ignored, and binary changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<GitFileStats>,
}

/// Result of `git/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub detached: bool,
    /// Upstream ref such as `origin/main`.
    #[serde(default)]
    pub upstream: Option<String>,
    /// Commits the branch has that its upstream lacks.
    #[serde(default)]
    pub ahead: u64,
    /// Commits the upstream has that the branch lacks.
    #[serde(default)]
    pub behind: u64,
    #[serde(default)]
    pub files: Vec<GitFile>,
}

/// Result of `git/branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranches {
    #[serde(default)]
    pub branches: Vec<GitBranch>,
    #[serde(default)]
    pub current: Option<String>,
}

/// Result of `git/checkout`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCheckoutResult {
    pub branch: String,
}

/// One changed path's unified diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDiff {
    pub path: String,
    pub staged: bool,
    pub patch: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub untracked: bool,
}

/// Result of one index or work-tree mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPathResult {
    pub path: String,
    pub operation: String,
}

/// Side of the repository a patch is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitApplyTarget {
    /// The index.
    Index,
    /// The work tree.
    Worktree,
}

/// Request for `git/apply`: one patch that changes exactly one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitApplyRequest {
    pub path: String,
    pub patch: String,
    pub target: GitApplyTarget,
    /// Apply the patch backwards.
    #[serde(default)]
    pub reverse: bool,
    /// Status the caller displayed; a different current status is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_status: Option<String>,
}

/// Result of `git/apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitApplyResult {
    pub path: String,
    /// Verb the applied patch settled: `stage`, `unstage`, `discard`, or `apply`.
    pub operation: String,
}

/// Options for one `git/commit`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitCommitOptions {
    /// Replace the current commit instead of adding one.
    pub amend: bool,
    /// Append the committer's `Signed-off-by` trailer.
    pub sign_off: bool,
}

/// One commit from `git/log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitEntry {
    pub oid: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub refs: Vec<String>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    /// Author instant as ISO 8601 with the author's UTC offset.
    pub author_time: String,
}

/// One newest-first page of `git/log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitLog {
    #[serde(default)]
    pub commits: Vec<GitCommitEntry>,
    /// Whether the Host byte cap dropped older commits from the page.
    #[serde(default)]
    pub truncated: bool,
}

/// Result of `git/show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitShow {
    /// Full object id the patch belongs to.
    pub commit: String,
    pub patch: String,
    #[serde(default)]
    pub truncated: bool,
}

/// Result of one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitResult {
    pub commit: String,
}

/// Result of one fetch, pull, or push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRemoteResult {
    pub operation: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub truncated: bool,
}

/// Multi-step Git operation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitOperationState {
    pub operation: String,
    #[serde(default)]
    pub conflicts: Vec<String>,
}

/// Result of one rebase, cherry-pick, continue, or abort step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitOperationResult {
    pub state: GitOperationState,
    #[serde(default)]
    pub output: String,
}

/// Native client for Git operations over one Session's workspace root.
pub struct GitClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> GitClient<'a> {
    /// Creates a Git client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Reads the Session workspace's branch and changed paths.
    pub fn status(&self, session_id: impl Into<String>) -> Result<GitStatus, Error> {
        self.call(
            "git/status",
            serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }),
        )
    }

    /// Lists the Session workspace's local branches.
    pub fn branches(&self, session_id: impl Into<String>) -> Result<GitBranches, Error> {
        self.call(
            "git/branches",
            serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }),
        )
    }

    /// Checks out one existing local branch.
    pub fn checkout(
        &self,
        session_id: impl Into<String>,
        branch: impl Into<String>,
    ) -> Result<GitCheckoutResult, Error> {
        self.call(
            "git/checkout",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "branch": branch.into() },
            }),
        )
    }

    /// Reads one changed path's unified diff.
    pub fn diff(&self, session_id: impl Into<String>, path: impl Into<String>, staged: bool) -> Result<GitDiff, Error> {
        self.call(
            "git/diff",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "path": path.into(), "staged": staged },
            }),
        )
    }

    /// Stages one changed path.
    pub fn stage(&self, session_id: impl Into<String>, path: impl Into<String>) -> Result<GitPathResult, Error> {
        self.path_verb("git/stage", session_id, path)
    }

    /// Removes one path's staged change.
    pub fn unstage(&self, session_id: impl Into<String>, path: impl Into<String>) -> Result<GitPathResult, Error> {
        self.path_verb("git/unstage", session_id, path)
    }

    /// Discards one tracked path's unstaged change.
    ///
    /// `expected_status` carries the status the caller displayed; the Host
    /// refuses when the path's current status differs.
    pub fn discard(
        &self,
        session_id: impl Into<String>,
        path: impl Into<String>,
        expected_status: Option<String>,
    ) -> Result<GitPathResult, Error> {
        let mut request = serde_json::json!({ "path": path.into() });
        if let Some(expected_status) = expected_status {
            request["expectedStatus"] = serde_json::Value::String(expected_status);
        }
        self.call(
            "git/discard",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": request,
            }),
        )
    }

    /// Commits the staged index.
    pub fn commit(&self, session_id: impl Into<String>, message: impl Into<String>) -> Result<GitCommitResult, Error> {
        self.commit_with_options(session_id, message, &GitCommitOptions::default())
    }

    /// Commits the staged index, optionally amending the current commit or signing it off.
    pub fn commit_with_options(
        &self,
        session_id: impl Into<String>,
        message: impl Into<String>,
        options: &GitCommitOptions,
    ) -> Result<GitCommitResult, Error> {
        let mut request = serde_json::json!({ "message": message.into() });
        if options.amend {
            request["amend"] = serde_json::Value::Bool(true);
        }
        if options.sign_off {
            request["signOff"] = serde_json::Value::Bool(true);
        }
        self.call(
            "git/commit",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": request,
            }),
        )
    }

    /// Applies one unified patch to the index or the work tree.
    pub fn apply(
        &self,
        session_id: impl Into<String>,
        request: &GitApplyRequest,
    ) -> Result<GitApplyResult, Error> {
        self.call(
            "git/apply",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": serde_json::to_value(request)?,
            }),
        )
    }

    /// Lists commit history newest first.
    pub fn log(
        &self,
        session_id: impl Into<String>,
        limit: Option<u32>,
        skip: Option<u32>,
    ) -> Result<GitLog, Error> {
        let mut request = serde_json::Map::new();
        if let Some(limit) = limit {
            request.insert("limit".to_owned(), serde_json::Value::from(limit));
        }
        if let Some(skip) = skip {
            request.insert("skip".to_owned(), serde_json::Value::from(skip));
        }
        self.call(
            "git/log",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": serde_json::Value::Object(request),
            }),
        )
    }

    /// Reads one commit's patch, optionally restricted to one path.
    pub fn show(
        &self,
        session_id: impl Into<String>,
        commit: impl Into<String>,
        path: Option<String>,
    ) -> Result<GitShow, Error> {
        let mut request = serde_json::json!({ "commit": commit.into() });
        if let Some(path) = path {
            request["path"] = serde_json::Value::String(path);
        }
        self.call(
            "git/show",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": request,
            }),
        )
    }

    /// Reads the repository's multi-step operation and conflicts.
    pub fn state(&self, session_id: impl Into<String>) -> Result<GitOperationState, Error> {
        self.call("git/state", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
    }

    /// Fetches every remote.
    pub fn fetch(&self, session_id: impl Into<String>) -> Result<GitRemoteResult, Error> {
        self.call("git/fetch", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
    }

    /// Fast-forwards the current branch from its upstream.
    pub fn pull(&self, session_id: impl Into<String>) -> Result<GitRemoteResult, Error> {
        self.call("git/pull", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
    }

    /// Pushes the current branch to its upstream.
    pub fn push(&self, session_id: impl Into<String>) -> Result<GitRemoteResult, Error> {
        self.push_with_options(session_id, false)
    }

    /// Pushes the current branch, optionally publishing it upstream.
    ///
    /// Publishing requires a checked-out branch and an `origin` remote; the
    /// Host refuses otherwise.
    pub fn push_with_options(
        &self,
        session_id: impl Into<String>,
        set_upstream: bool,
    ) -> Result<GitRemoteResult, Error> {
        let mut request = serde_json::Map::new();
        if set_upstream {
            request.insert("setUpstream".to_owned(), serde_json::Value::Bool(true));
        }
        self.call(
            "git/push",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": request,
            }),
        )
    }

    /// Rebases the current branch onto one local branch.
    pub fn rebase(&self, session_id: impl Into<String>, branch: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call(
            "git/rebase",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "branch": branch.into() },
            }),
        )
    }

    /// Merges one local branch into the current branch.
    pub fn merge(&self, session_id: impl Into<String>, branch: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call(
            "git/merge",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "branch": branch.into() },
            }),
        )
    }

    /// Cherry-picks one commit onto the current branch.
    pub fn cherry_pick(&self, session_id: impl Into<String>, commit: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call(
            "git/cherryPick",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "commit": commit.into() },
            }),
        )
    }

    /// Reverts one commit on the current branch.
    pub fn revert(&self, session_id: impl Into<String>, commit: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call(
            "git/revert",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "commit": commit.into() },
            }),
        )
    }

    /// Continues the in-progress merge, rebase, cherry-pick, or revert.
    pub fn continue_operation(&self, session_id: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call("git/continueOperation", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
    }

    /// Aborts the in-progress merge, rebase, cherry-pick, or revert.
    pub fn abort_operation(&self, session_id: impl Into<String>) -> Result<GitOperationResult, Error> {
        self.call("git/abortOperation", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
    }

    fn path_verb(&self, method: &str, session_id: impl Into<String>, path: impl Into<String>) -> Result<GitPathResult, Error> {
        self.call(
            method,
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "path": path.into() },
            }),
        )
    }

    fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, payload: Value) -> Result<T, Error> {
        Ok(serde_json::from_value(
            self.transport
                .request(method, payload, &AtomicBool::new(false))?,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_status_with_files() {
        let status: GitStatus = serde_json::from_value(serde_json::json!({
            "branch": "main",
            "commit": "abc",
            "detached": false,
            "files": [{"path": "src/a.ts", "status": "modified", "staged": false, "worktree": true}],
        }))
        .unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.files[0].path, "src/a.ts");
        assert!(status.files[0].worktree);
    }

    #[test]
    fn deserializes_detached_head_without_a_branch() {
        let status: GitStatus = serde_json::from_value(serde_json::json!({
            "branch": null,
            "commit": "abc",
            "detached": true,
            "files": [],
        }))
        .unwrap();
        assert!(status.branch.is_none());
        assert!(status.detached);
    }

    #[test]
    fn deserializes_operation_state_and_remote_summary() {
        let state: GitOperationState = serde_json::from_value(serde_json::json!({
            "operation": "rebase",
            "conflicts": ["a.txt"],
        }))
        .unwrap();
        assert_eq!(state.operation, "rebase");
        assert_eq!(state.conflicts, ["a.txt"]);
        let remote: GitRemoteResult = serde_json::from_value(serde_json::json!({
            "operation": "fetch",
            "summary": "done",
            "truncated": false,
        }))
        .unwrap();
        assert_eq!(remote.summary, "done");
    }

    #[test]
    fn deserializes_status_details_and_per_side_stats() {
        let status: GitStatus = serde_json::from_value(serde_json::json!({
            "branch": "main",
            "commit": "abc",
            "detached": false,
            "upstream": "origin/main",
            "ahead": 2,
            "behind": 3,
            "files": [
                {
                    "path": "src/new.ts",
                    "status": "renamed",
                    "staged": true,
                    "worktree": false,
                    "origPath": "src/old.ts",
                    "stats": {"index": {"additions": 2, "deletions": 0}},
                },
                {
                    "path": "src/both.ts",
                    "status": "modified",
                    "staged": true,
                    "worktree": true,
                    "stats": {
                        "index": {"additions": 4, "deletions": 1},
                        "worktree": {"additions": 1, "deletions": 1},
                    },
                },
                {"path": "src/loose.ts", "status": "untracked", "staged": false, "worktree": true},
            ],
        }))
        .unwrap();
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 3));
        assert_eq!(status.files[0].orig_path.as_deref(), Some("src/old.ts"));
        assert_eq!(
            status.files[0].stats,
            Some(GitFileStats {
                index: Some(GitFileSideStats { additions: 2, deletions: 0 }),
                worktree: None,
            })
        );
        let both = status.files[1].stats.as_ref().unwrap();
        assert_eq!(both.worktree, Some(GitFileSideStats { additions: 1, deletions: 1 }));
        assert!(status.files[2].orig_path.is_none());
        assert!(status.files[2].stats.is_none());
    }

    #[test]
    fn defaults_status_details_absent_from_an_older_host() {
        let status: GitStatus = serde_json::from_value(serde_json::json!({
            "branch": "main",
            "files": [{"path": "a.ts", "status": "modified"}],
        }))
        .unwrap();
        assert!(status.upstream.is_none());
        assert_eq!((status.ahead, status.behind), (0, 0));
        assert!(status.files[0].orig_path.is_none());
        assert!(status.files[0].stats.is_none());
    }

    #[test]
    fn serializes_the_apply_request_onto_the_wire_fields() {
        let request = GitApplyRequest {
            path: "src/a.ts".to_owned(),
            patch: "diff --git a/src/a.ts b/src/a.ts\n".to_owned(),
            target: GitApplyTarget::Index,
            reverse: true,
            expected_status: Some("modified".to_owned()),
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["target"], serde_json::json!("index"));
        assert_eq!(wire["reverse"], serde_json::json!(true));
        assert_eq!(wire["expectedStatus"], serde_json::json!("modified"));
        let round_trip: GitApplyRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(round_trip, request);
    }

    #[test]
    fn serializes_an_apply_request_without_optional_fields() {
        let request = GitApplyRequest {
            path: "src/a.ts".to_owned(),
            patch: "patch".to_owned(),
            target: GitApplyTarget::Worktree,
            reverse: false,
            expected_status: None,
        };
        let wire = serde_json::to_value(&request).unwrap();
        assert_eq!(wire["target"], serde_json::json!("worktree"));
        assert!(wire.get("expectedStatus").is_none());
    }

    #[test]
    fn deserializes_history_and_show_results() {
        let log: GitLog = serde_json::from_value(serde_json::json!({
            "commits": [{
                "oid": "a".repeat(40),
                "parents": ["b".repeat(40)],
                "refs": ["HEAD", "main", "tag:v1.0.0"],
                "subject": "second",
                "authorName": "Ada",
                "authorEmail": "ada@example.com",
                "authorTime": "2026-11-08T10:00:00+07:00",
            }],
            "truncated": true,
        }))
        .unwrap();
        assert!(log.truncated);
        assert_eq!(log.commits[0].refs, ["HEAD", "main", "tag:v1.0.0"]);
        assert_eq!(log.commits[0].author_time, "2026-11-08T10:00:00+07:00");

        let show: GitShow = serde_json::from_value(serde_json::json!({
            "commit": "a".repeat(40),
            "patch": "diff --git a/a b/a\n",
            "truncated": false,
        }))
        .unwrap();
        assert!(!show.truncated);
        let apply: GitApplyResult = serde_json::from_value(serde_json::json!({
            "path": "a.ts",
            "operation": "unstage",
        }))
        .unwrap();
        assert_eq!(apply.operation, "unstage");
    }
}
