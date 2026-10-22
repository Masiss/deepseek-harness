//! Typed settings RPCs for the Desktop settings surface.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// JSON value accepted by settings writes.
pub type JsonValue = Value;

/// A redacted configured secret slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSecretView {
    pub path: Vec<String>,
    pub set: bool,
}

/// One settings namespace returned by the Host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsNamespaceView {
    pub auto_generate: bool,
    pub ns: String,
    pub schema: JsonValue,
    pub value: JsonValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<JsonValue>,
    pub applies: String,
    pub secrets: Vec<SettingsSecretView>,
    pub revision: u64,
}

/// Full settings description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDescribeValue {
    pub writable: bool,
    pub has_document: bool,
    pub namespaces: Vec<SettingsNamespaceView>,
}

/// One path-addressed settings mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum SettingsPathOp {
    #[serde(rename = "set")]
    Set { path: Vec<String>, value: JsonValue },
    #[serde(rename = "unset")]
    Unset { path: Vec<String> },
}

/// Namespace patch request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdateRequest {
    pub ns: String,
    pub patch: serde_json::Map<String, JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// Namespace replacement request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsReplaceRequest {
    pub ns: String,
    pub section: serde_json::Map<String, JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// Path mutation request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsMutateRequest {
    pub ns: String,
    pub ops: Vec<SettingsPathOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

/// Native client for settings reads and writes.
pub struct SettingsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> SettingsClient<'a> {
    /// Creates a settings client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Reads all settings namespaces and provider capabilities.
    pub fn describe(&self) -> Result<SettingsDescribeValue, Error> {
        self.call("settings/describe", serde_json::json!({}))
    }

    /// Applies a shallow namespace patch.
    pub fn update(&self, request: SettingsUpdateRequest) -> Result<SettingsNamespaceView, Error> {
        self.call("settings/update", serde_json::to_value(request)?)
    }

    /// Replaces a namespace section.
    pub fn replace(&self, request: SettingsReplaceRequest) -> Result<SettingsNamespaceView, Error> {
        self.call("settings/replace", serde_json::to_value(request)?)
    }

    /// Applies path-addressed set and unset operations.
    pub fn mutate(&self, request: SettingsMutateRequest) -> Result<SettingsNamespaceView, Error> {
        self.call("settings/mutate", serde_json::to_value(request)?)
    }

    /// Asks the Host to open its settings document.
    pub fn open_settings_document(&self) -> Result<OpenedResponse, Error> {
        self.call("settings/openSettingsDocument", serde_json::json!({}))
    }

    fn call<T: for<'de> Deserialize<'de>>(&self, method: &str, payload: Value) -> Result<T, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            payload,
            &AtomicBool::new(false),
        )?)?)
    }
}

/// Successful document-open response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedResponse {
    pub opened: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_mutation_and_optional_revision() {
        let request = SettingsMutateRequest {
            ns: "demo".into(),
            ops: vec![
                SettingsPathOp::Set {
                    path: vec!["x".into()],
                    value: serde_json::json!(true),
                },
                SettingsPathOp::Unset {
                    path: vec!["y".into()],
                },
            ],
            expected_revision: Some(3),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"ns":"demo","ops":[{"op":"set","path":["x"],"value":true},{"op":"unset","path":["y"]}],"expectedRevision":3})
        );
    }

    #[test]
    fn deserializes_namespace_view() {
        let view: SettingsNamespaceView = serde_json::from_value(serde_json::json!({"autoGenerate":true,"ns":"demo","schema":{},"value":{"x":1},"applies":"live","secrets":[],"revision":4})).unwrap();
        assert_eq!(view.revision, 4);
        assert_eq!(view.applies, "live");
    }
}
