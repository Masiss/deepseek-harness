//! Typed workspace file RPCs, including binary attachment values.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// Session identity used by the Host to resolve its active workspace scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFileScope {
    pub session_id: String,
}

/// Line range for text reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFileRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Byte range for binary reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceByteRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
}

/// Binary read options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceByteReadOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<WorkspaceByteRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_file: Option<String>,
}

/// File metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileStat {
    pub absolute_path: String,
    pub version: String,
    #[serde(default)]
    pub bytes: Option<u64>,
}

/// Text file page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceFileText {
    #[serde(flatten)]
    pub stat: WorkspaceFileStat,
    pub offset: u64,
    pub text: String,
    pub lines: u64,
    pub eof: bool,
}

/// Binary file page. `data` is the decoded attachment bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileBytes {
    pub stat: WorkspaceFileStat,
    pub offset: u64,
    pub data: Vec<u8>,
    pub eof: bool,
}

impl<'de> Deserialize<'de> for WorkspaceFileBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(flatten)]
            stat: WorkspaceFileStat,
            offset: u64,
            data: Vec<u8>,
            eof: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            stat: wire.stat,
            offset: wire.offset,
            data: wire.data,
            eof: wire.eof,
        })
    }
}

/// One workspace file observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub absolute_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absent: Option<bool>,
}

/// One `workspaceFiles/changes` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkspaceFileWatchFrame {
    Ready,
    Change { change: WorkspaceFileChange },
}

/// Directory child entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDirectoryEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// Directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDirectoryListing {
    pub path: String,
    pub entries: Vec<WorkspaceDirectoryEntry>,
    pub truncated: bool,
}

/// Native client for workspace file operations.
pub struct WorkspaceFilesClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> WorkspaceFilesClient<'a> {
    /// Creates a workspace files client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Reads a UTF-8 text page.
    pub fn read(
        &self,
        scope: WorkspaceFileScope,
        path: impl Into<String>,
        range: WorkspaceFileRange,
    ) -> Result<WorkspaceFileText, Error> {
        self.call(
            "workspaceFiles/read",
            serde_json::json!({"workspaceFileScopeId":scope.session_id,"path":path.into(),"range":range}),
        )
    }
    /// Reads a binary attachment page. The current HTTP transport expects the attachment bytes as `data` in the decoded result.
    pub fn read_bytes(
        &self,
        scope: WorkspaceFileScope,
        path: impl Into<String>,
        options: WorkspaceByteReadOptions,
    ) -> Result<WorkspaceFileBytes, Error> {
        self.call(
            "workspaceFiles/readBytes",
            serde_json::json!({"workspaceFileScopeId":scope.session_id,"path":path.into(),"options":options}),
        )
    }
    /// Reads file metadata.
    pub fn stat(
        &self,
        scope: WorkspaceFileScope,
        path: impl Into<String>,
    ) -> Result<WorkspaceFileStat, Error> {
        self.call(
            "workspaceFiles/stat",
            serde_json::json!({"workspaceFileScopeId":scope.session_id,"path":path.into()}),
        )
    }
    /// Lists direct workspace directory children.
    pub fn list(
        &self,
        scope: WorkspaceFileScope,
        path: impl Into<String>,
    ) -> Result<WorkspaceDirectoryListing, Error> {
        self.call(
            "workspaceFiles/list",
            serde_json::json!({"workspaceFileScopeId":scope.session_id,"path":path.into()}),
        )
    }

    fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, payload: Value) -> Result<T, Error> {
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
    fn serializes_scoped_binary_request() {
        let scope = WorkspaceFileScope {
            session_id: "s".into(),
        };
        let value = serde_json::json!({"workspaceFileScopeId":scope.session_id,"path":"a.bin","options":{"range":{"offset":2,"length":4}}});
        assert_eq!(
            value,
            serde_json::json!({"workspaceFileScopeId":"s","path":"a.bin","options":{"range":{"offset":2,"length":4}}})
        );
    }
    #[test]
    fn deserializes_binary_attachment_bytes() {
        let bytes: WorkspaceFileBytes = serde_json::from_value(serde_json::json!({"absolutePath":"/x","version":"v","offset":0,"data":[1,2],"eof":true})).unwrap();
        assert_eq!(bytes.data, vec![1, 2]);
    }
    #[test]
    fn deserializes_listing() {
        let listing: WorkspaceDirectoryListing = serde_json::from_value(serde_json::json!({"path":"src","entries":[{"name":"a","type":"file","size":3}],"truncated":false})).unwrap();
        assert_eq!(listing.entries[0].entry_type, "file");
    }
}
