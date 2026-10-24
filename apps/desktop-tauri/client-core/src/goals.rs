//! Typed RPCs for reading and managing the current Session goal.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Compare-and-set identity for one exact goal revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalRef {
    pub id: String,
    pub revision: u64,
}

/// Durable phase of the current goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalPhase {
    Active,
    Paused,
    Blocked,
    Complete,
}

/// Process-local continuation eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalActivation {
    Armed,
    Disarmed,
}

/// Machine-routable and human-readable explanation for a blocked goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalBlockReason {
    pub code: String,
    pub message: String,
}

/// Current durable goal and its runtime activation state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalView {
    pub id: String,
    pub revision: u64,
    pub objective: String,
    pub phase: GoalPhase,
    #[serde(default)]
    pub blocked_reason: Option<GoalBlockReason>,
    pub max_goal_rounds: u64,
    pub rounds_started: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub activation: GoalActivation,
}

/// Values accepted when a goal is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGoalRequest {
    pub objective: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_goal_rounds: Option<u64>,
}

/// Wire acknowledgement of a created goal; read the live view separately with `get`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreateGoalResult {
    pub r#ref: GoalRef,
}

/// Fields accepted when a goal is edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditGoalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_goal_rounds: Option<u64>,
}

/// Native RPC client for the current goal attached to a live Agent's Session.
/// Calls use the live `agentId`; stale revisions are reported without retries.
pub struct GoalsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> GoalsClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Reads the current goal, if the Session has one.
    pub fn get(&self, agent_id: &str) -> Result<Option<GoalView>, Error> {
        self.call("goals/get", serde_json::json!({"agentId":agent_id}))
    }

    /// Creates and arms a goal, returning its identity rather than a live view.
    pub fn create(
        &self,
        agent_id: &str,
        request: CreateGoalRequest,
    ) -> Result<CreateGoalResult, Error> {
        self.call(
            "goals/create",
            serde_json::json!({"agentId":agent_id,"request":request}),
        )
    }

    /// Edits the current goal using its expected revision.
    pub fn edit(
        &self,
        agent_id: &str,
        reference: &GoalRef,
        request: EditGoalRequest,
    ) -> Result<GoalView, Error> {
        self.call(
            "goals/edit",
            serde_json::json!({"agentId":agent_id,"ref":reference,"request":request}),
        )
    }

    /// Pauses the current goal and disarms automatic continuation.
    pub fn pause(&self, agent_id: &str, reference: &GoalRef) -> Result<GoalView, Error> {
        self.transition("goals/pause", agent_id, reference)
    }

    /// Resumes a paused or blocked goal, or rearms a disarmed active goal.
    pub fn resume(&self, agent_id: &str, reference: &GoalRef) -> Result<GoalView, Error> {
        self.transition("goals/resume", agent_id, reference)
    }

    /// Marks the current goal complete.
    pub fn complete(&self, agent_id: &str, reference: &GoalRef) -> Result<GoalView, Error> {
        self.transition("goals/complete", agent_id, reference)
    }

    /// Clears the current goal and returns the resulting tombstone revision.
    pub fn clear(&self, agent_id: &str, reference: &GoalRef) -> Result<GoalRef, Error> {
        self.call(
            "goals/clear",
            serde_json::json!({"agentId":agent_id,"ref":reference}),
        )
    }

    fn transition(
        &self,
        method: &str,
        agent_id: &str,
        reference: &GoalRef,
    ) -> Result<GoalView, Error> {
        self.call(
            method,
            serde_json::json!({"agentId":agent_id,"ref":reference}),
        )
    }

    fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        args: serde_json::Value,
    ) -> Result<T, Error> {
        let cancel = AtomicBool::new(false);
        Ok(serde_json::from_value(
            self.transport.request(method, args, &cancel)?,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_optionals_are_omitted_instead_of_sent_as_null() {
        assert_eq!(
            serde_json::to_value(CreateGoalRequest {
                objective: "Ship goal RPCs".into(),
                max_goal_rounds: None,
            })
            .unwrap(),
            json!({"objective":"Ship goal RPCs"}),
        );
        assert_eq!(
            serde_json::to_value(CreateGoalRequest {
                objective: "Ship goal RPCs".into(),
                max_goal_rounds: Some(9_007_199_254_740_991),
            })
            .unwrap(),
            json!({"objective":"Ship goal RPCs","maxGoalRounds":9_007_199_254_740_991_u64}),
        );
        for (request, expected) in [
            (
                EditGoalRequest {
                    objective: Some("Revised".into()),
                    max_goal_rounds: None,
                },
                json!({"objective":"Revised"}),
            ),
            (
                EditGoalRequest {
                    objective: None,
                    max_goal_rounds: Some(500),
                },
                json!({"maxGoalRounds":500}),
            ),
            (
                EditGoalRequest {
                    objective: Some("Revised".into()),
                    max_goal_rounds: Some(500),
                },
                json!({"objective":"Revised","maxGoalRounds":500}),
            ),
        ] {
            assert_eq!(serde_json::to_value(request).unwrap(), expected);
        }
    }

    #[test]
    fn create_acknowledgement_requires_the_nested_reference() {
        let result: CreateGoalResult = serde_json::from_value(json!({
            "ref":{"id":"goal-1","revision":1}
        }))
        .unwrap();
        assert_eq!(
            result.r#ref,
            GoalRef {
                id: "goal-1".into(),
                revision: 1
            }
        );
        for invalid in [
            json!({"id":"goal-1","revision":1}),
            json!({"ref":{"id":"goal-1"}}),
            json!({"ref":{"id":"goal-1","revision":-1}}),
        ] {
            assert!(serde_json::from_value::<CreateGoalResult>(invalid).is_err());
        }
    }

    #[test]
    fn goal_view_requires_known_phase_activation_and_round_counts() {
        let value = json!({
            "id":"goal-1","revision":2,"objective":"Ship goal RPCs",
            "phase":"active","activation":"disarmed",
            "maxGoalRounds":9_007_199_254_740_991_u64,"roundsStarted":4_294_967_296_u64,
            "createdAt":1792710000000_u64,"updatedAt":1792710000001_u64
        });
        let view: GoalView = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(view.phase, GoalPhase::Active);
        assert_eq!(view.activation, GoalActivation::Disarmed);
        assert_eq!(view.blocked_reason, None);
        assert_eq!(view.max_goal_rounds, 9_007_199_254_740_991);
        assert_eq!(view.rounds_started, 4_294_967_296);
        for (field, invalid) in [
            ("phase", json!("pending")),
            ("activation", json!("enabled")),
            ("roundsStarted", json!(-1)),
            ("revision", json!(1.5)),
        ] {
            let mut malformed = value.clone();
            malformed[field] = invalid;
            assert!(serde_json::from_value::<GoalView>(malformed).is_err());
        }
        let mut missing = value;
        missing.as_object_mut().unwrap().remove("activation");
        assert!(serde_json::from_value::<GoalView>(missing).is_err());
    }
}
