//! Typed skill inventory RPCs for the Desktop composer.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// One skill available to a session composer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    pub model_invocable: bool,
}

/// Skill list response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillEntry>,
}

/// Request for skills visible to one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListRequest {
    pub session_id: String,
}

/// Native client for both skill list endpoints.
pub struct SkillsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> SkillsClient<'a> {
    /// Creates a skills client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists skills for the general session composer endpoint.
    pub fn list(&self, session_id: impl Into<String>) -> Result<SkillListResponse, Error> {
        self.call("skills/list", session_id.into())
    }

    /// Lists skills through the session namespace endpoint.
    pub fn session_list(&self, session_id: impl Into<String>) -> Result<SkillListResponse, Error> {
        self.call("session/skills/list", session_id.into())
    }

    fn call(&self, method: &str, session_id: String) -> Result<SkillListResponse, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            serde_json::to_value(SkillListRequest { session_id })?,
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_request_and_optional_fields() {
        let entry = SkillEntry {
            path: None,
            name: "review".into(),
            description: "Review code".into(),
            when_to_use: Some("when asked".into()),
            model_invocable: true,
        };
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::json!({"name":"review","description":"Review code","whenToUse":"when asked","modelInvocable":true})
        );
    }
    #[test]
    fn deserializes_response() {
        let response: SkillListResponse = serde_json::from_value(
            serde_json::json!({"skills":[{"name":"x","description":"X","modelInvocable":false}]}),
        )
        .unwrap();
        assert_eq!(response.skills[0].name, "x");
    }
}
