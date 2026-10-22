//! Typed approval Remote Event payloads for the Desktop permission screen.
//!
//! Approval decisions are not unary RPCs: the Host resolves `approval/request`
//! through its Remote Event waterfall. This module therefore exposes payload
//! types only; sending a decision awaits the not-yet-implemented event channel.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_reason: Option<DisplayReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayReason {
    pub en: String,
    #[serde(flatten)]
    pub localized: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecisionPayload {
    pub decision: ApprovalDecision,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deserializes_request_fixture() {
        let request: ApprovalRequest = serde_json::from_str(r#"{"toolName":"bash","callId":"call-1","reason":"run command","displayReason":{"en":"Run command"}}"#).unwrap();
        assert_eq!(request.tool_name, "bash");
        assert_eq!(request.call_id.as_deref(), Some("call-1"));
        assert_eq!(
            request.display_reason.as_ref().map(|v| v.en.as_str()),
            Some("Run command")
        );
    }
}
