//! Typed approval Remote Event payloads for the Desktop permission screen.
//!
//! Approval decisions resolve `approval/request` through the Remote Event waterfall.
//! Host cancellation is carried separately from the JSON presentation fields.

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
    #[serde(rename = "allowed-once")]
    AllowedOnce,
    #[serde(rename = "rejected")]
    Rejected,
}

/// An approval presentation correlated with its owning Agent and event generation.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub invocation: crate::remote_events::RemoteEventInvocation,
    pub request: ApprovalRequest,
}

impl PendingApproval {
    /// Decodes approval presentation fields without projecting a cancellation signal into JSON.
    pub fn from_invocation(
        invocation: crate::remote_events::RemoteEventInvocation,
    ) -> Result<Self, crate::remote_events::RemoteEventError> {
        if invocation.event != "approval/request" {
            return Err(crate::remote_events::RemoteEventError::Protocol(
                "not an approval request",
            ));
        }
        let request = serde_json::from_value(serde_json::Value::Object(invocation.request.clone()))
            .map_err(crate::Error::from)?;
        Ok(Self {
            invocation,
            request,
        })
    }

    /// Submits the browser's one-time allow or deny decision. A denial is a result, not a rejection.
    pub fn answer(
        &self,
        client: &crate::remote_events::RemoteEvents,
        decision: ApprovalDecision,
    ) -> Result<(), crate::remote_events::RemoteEventError> {
        client.submit(
            &self.invocation,
            crate::remote_events::RemoteEventOutcome::Result { value: decision },
        )
    }

    /// Delegates to the next Host waterfall listener without answering the request.
    pub fn delegate(
        &self,
        client: &crate::remote_events::RemoteEvents,
    ) -> Result<(), crate::remote_events::RemoteEventError> {
        client.submit::<ApprovalDecision>(
            &self.invocation,
            crate::remote_events::RemoteEventOutcome::Next,
        )
    }

    /// Rejects the listener with structured error fields, distinct from the user's deny decision.
    pub fn reject(
        &self,
        client: &crate::remote_events::RemoteEvents,
        error: crate::remote_events::RemoteEventRejection,
    ) -> Result<(), crate::remote_events::RemoteEventError> {
        client.submit::<ApprovalDecision>(
            &self.invocation,
            crate::remote_events::RemoteEventOutcome::Rejected { error },
        )
    }
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

    #[test]
    fn decision_values_match_client_waterfall_results() {
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::AllowedOnce).unwrap(),
            "\"allowed-once\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Rejected).unwrap(),
            "\"rejected\""
        );
        assert!(serde_json::from_str::<ApprovalDecision>("\"allow-once\"").is_err());
    }
}
