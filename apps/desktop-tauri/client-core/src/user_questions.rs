//! Typed answer submission for live `ask_user_question` calls.
//!
//! The method is an agent-scoped Remote method (`userQuestions/answer`); the
//! Host resolves the owning Agent from the Session identity. The Desktop
//! inventory lists only the wait stream, so this client documents the unary
//! method the web client also calls.

use crate::{
    remote_events::{RemoteEventError, RemoteEventInvocation, RemoteEvents},
    transport::HttpTransport,
    Error,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// One answered question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserQuestionAnswerItem {
    /// Caller-provided question id echoed from the request.
    pub id: String,
    /// Selected option labels; empty means the question was skipped.
    #[serde(default)]
    pub selected: Vec<String>,
    /// Optional free-text answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<String>,
}

/// The human's answer batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskUserQuestionAnswer {
    /// One item per question.
    pub answers: Vec<AskUserQuestionAnswerItem>,
}

/// One choice offered for a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserQuestionOption {
    /// User-facing label.
    pub label: String,
    /// Optional extra context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One question the model asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserQuestion {
    /// Caller-provided id echoed in the answer.
    pub id: String,
    /// Question text.
    pub question: String,
    /// Optional supporting detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Optional short heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Offered choices.
    #[serde(default)]
    pub options: Vec<UserQuestionOption>,
    /// Whether more than one option may be selected.
    #[serde(rename = "multiSelect", default)]
    pub multi_select: bool,
}

/// Foreground wait metadata for a timed question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserQuestionWait {
    /// Tool call the card is keyed by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Whether the answer may also arrive after the foreground wait ends.
    #[serde(default)]
    pub timed: bool,
}

/// The `user-questions/request` payload presented by the waterfall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserQuestionRequest {
    /// Questions to answer.
    pub questions: Vec<UserQuestion>,
    /// Wait metadata; absent for blocking questions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<UserQuestionWait>,
}

/// A live question request correlated with its owning Agent and event generation.
#[derive(Debug, Clone)]
pub struct PendingQuestion {
    /// Waterfall invocation the answer settles.
    pub invocation: RemoteEventInvocation,
    /// Presentation fields.
    pub request: UserQuestionRequest,
}

impl PendingQuestion {
    /// Decodes a forwarded question request.
    pub fn from_invocation(invocation: RemoteEventInvocation) -> Result<Self, RemoteEventError> {
        if invocation.event != "user-questions/request" {
            return Err(RemoteEventError::Protocol("not a user-questions request"));
        }
        let request = serde_json::from_value::<UserQuestionRequest>(Value::Object(
            invocation.request.clone(),
        ))
        .map_err(|_| RemoteEventError::Protocol("invalid user-questions request"))?;
        Ok(Self { invocation, request })
    }

    /// Settles the request with the human's answer batch.
    pub fn answer(
        &self,
        client: &RemoteEvents,
        answer: AskUserQuestionAnswer,
    ) -> Result<(), RemoteEventError> {
        client.submit(
            &self.invocation,
            crate::remote_events::RemoteEventOutcome::Result { value: answer },
        )
    }

    /// Delegates to the next Host waterfall listener without answering.
    pub fn delegate(&self, client: &RemoteEvents) -> Result<(), RemoteEventError> {
        client.submit::<AskUserQuestionAnswer>(
            &self.invocation,
            crate::remote_events::RemoteEventOutcome::Next,
        )
    }
}

/// Submits answers to a live question call.
pub struct UserQuestionsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> UserQuestionsClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Answers one live question call.
    ///
    /// `false` means the call is no longer answerable, not a transport failure.
    pub fn answer(
        &self,
        session_id: &str,
        call_id: &str,
        answer: AskUserQuestionAnswer,
    ) -> Result<bool, Error> {
        let cancel = AtomicBool::new(false);
        self.transport
            .request(
                "userQuestions/answer",
                serde_json::json!({
                    "agentId": session_id,
                    "callId": call_id,
                    "answer": answer,
                }),
                &cancel,
            )
            .and_then(|value| serde_json::from_value(value).map_err(Error::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httptest::{matchers::*, responders::status_code, Expectation, Server};

    fn invocation(request: serde_json::Value) -> RemoteEventInvocation {
        RemoteEventInvocation::for_test(
            "user-questions/request",
            request.as_object().unwrap().clone(),
        )
    }

    #[test]
    fn decodes_questions_and_wait_metadata() {
        let pending = PendingQuestion::from_invocation(invocation(serde_json::json!({
            "questions": [{
                "id": "probe",
                "question": "Continue?",
                "header": "Probe",
                "multiSelect": true,
                "options": [
                    {"label": "Yes"},
                    {"label": "No", "description": "stop here"}
                ]
            }],
            "wait": {"callId": "call-1", "timed": true}
        })))
        .unwrap();
        let question = &pending.request.questions[0];
        assert_eq!(question.id, "probe");
        assert!(question.multi_select);
        assert_eq!(question.options[1].label, "No");
        assert_eq!(question.options[1].description.as_deref(), Some("stop here"));
        assert_eq!(
            pending.request.wait.as_ref().and_then(|wait| wait.call_id.as_deref()),
            Some("call-1")
        );
    }

    #[test]
    fn rejects_other_events_and_malformed_questions() {
        let wrong = RemoteEventInvocation::for_test(
            "approval/request",
            serde_json::json!({"questions": []})
                .as_object()
                .unwrap()
                .clone(),
        );
        assert!(matches!(
            PendingQuestion::from_invocation(wrong),
            Err(RemoteEventError::Protocol("not a user-questions request"))
        ));
        assert!(matches!(
            PendingQuestion::from_invocation(invocation(serde_json::json!({"questions": "x"}))),
            Err(RemoteEventError::Protocol("invalid user-questions request"))
        ));
    }

    #[test]
    fn submits_the_answer_batch_with_the_call_identity() {
        let server = Server::run();
        server.expect(
            Expectation::matching(all_of![
                request::method_path("POST", "/api/userQuestions/answer"),
                request::body(json_decoded(eq(serde_json::json!({
                    "type": "client-request",
                    "rpcId": "native-1",
                    "method": "userQuestions/answer",
                    "payload": {
                        "args": {
                            "agentId": "s1",
                            "callId": "call-1",
                            "answer": { "answers": [{ "id": "probe", "selected": ["Yes"] }] }
                        }
                    }
                })))),
            ])
            .respond_with(status_code(200).body(
                r#"{"type":"server-response","rpcId":"native-1","result":{"ok":true,"value":true}}"#,
            )),
        );
        let transport = HttpTransport::new(&server.url_str("/")).unwrap();
        let client = UserQuestionsClient::new(&transport);
        assert!(client
            .answer(
                "s1",
                "call-1",
                AskUserQuestionAnswer {
                    answers: vec![AskUserQuestionAnswerItem {
                        id: "probe".into(),
                        selected: vec!["Yes".into()],
                        custom: None,
                    }],
                },
            )
            .unwrap());
    }
}
