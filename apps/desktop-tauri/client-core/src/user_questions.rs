//! Typed answer submission for live `ask_user_question` calls.
//!
//! The method is an agent-scoped Remote method (`userQuestions/answer`); the
//! Host resolves the owning Agent from the Session identity. The Desktop
//! inventory lists only the wait stream, so this client documents the unary
//! method the web client also calls.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
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
