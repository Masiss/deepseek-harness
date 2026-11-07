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

/// One changed path from `git/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFile {
    pub path: String,
    pub status: String,
    #[serde(default)]
    pub staged: bool,
    #[serde(default)]
    pub worktree: bool,
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
        self.call(
            "git/commit",
            serde_json::json!({
                "gitWorkspaceScopeId": session_id.into(),
                "request": { "message": message.into() },
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
        self.call("git/push", serde_json::json!({ "gitWorkspaceScopeId": session_id.into() }))
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
    fn serializes_checkout_request() {
        assert_eq!(
            serde_json::json!({
                "gitWorkspaceScopeId": "s",
                "request": { "branch": "feature" },
            }),
            serde_json::json!({
                "gitWorkspaceScopeId": "s",
                "request": { "branch": "feature" },
            })
        );
    }
}
