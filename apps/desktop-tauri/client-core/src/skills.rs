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
    /// Discovery source that produced the winning skill (project/user/bundled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Provider package that owns the skill body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// Skill list response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResponse {
    pub skills: Vec<SkillEntry>,
    /// Names the Host filters out of every catalog read.
    #[serde(default)]
    pub disabled_skills: Vec<String>,
}

/// Applied disabled set and whether the Host persisted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSetDisabledResponse {
    pub disabled_skills: Vec<String>,
    pub persisted: bool,
}

/// Request for skills visible to one session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillListRequest {
    pub session_id: String,
}

/// Native client for the session-addressed skill catalog.
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

    /// Lists session-visible skills through the Host skill catalog.
    pub fn session_list(&self, session_id: impl Into<String>) -> Result<SkillListResponse, Error> {
        self.list(session_id)
    }

    /// Replaces the disabled skill set for this composition.
    pub fn set_disabled(
        &self,
        names: Vec<String>,
    ) -> Result<SkillSetDisabledResponse, Error> {
        Ok(serde_json::from_value(self.transport.request(
            "skills/setDisabled",
            serde_json::json!({"request": {"names": names}}),
            &AtomicBool::new(false),
        )?)?)
    }

    fn call(&self, method: &str, session_id: String) -> Result<SkillListResponse, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            serde_json::json!({"request": SkillListRequest { session_id }}),
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
            source: Some("project-agents".into()),
            provider: Some("@deepseek-ai/dsh-skill".into()),
        };
        assert_eq!(
            serde_json::to_value(entry).unwrap(),
            serde_json::json!({"name":"review","description":"Review code","whenToUse":"when asked","modelInvocable":true,"source":"project-agents","provider":"@deepseek-ai/dsh-skill"})
        );
    }
    #[test]
    fn deserializes_response() {
        let response: SkillListResponse = serde_json::from_value(
            serde_json::json!({"skills":[{"name":"x","description":"X","modelInvocable":false}],"disabledSkills":["y"]}),
        )
        .unwrap();
        assert_eq!(response.skills[0].name, "x");
        assert_eq!(response.skills[0].source, None);
        assert_eq!(response.disabled_skills, ["y"]);
    }
}
