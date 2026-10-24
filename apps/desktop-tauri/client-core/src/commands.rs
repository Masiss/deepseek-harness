//! Native RPC client for the Host's human-command registry.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Optional free-form input metadata shown by command-aware clients.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInputDescriptor {
    pub hint: String,
    #[serde(default)]
    pub attachments: Option<bool>,
}

/// Handler-free command metadata returned by the Host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDescriptor {
    #[serde(default)]
    pub definition_id: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input: Option<CommandInputDescriptor>,
}

/// A file receipt submitted with a command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum CommandSubmitAttachment {
    File { receipt_id: String },
}

/// One command handler's settled outcome.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CommandResult {
    Success {
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        source_event_seq: Option<u64>,
    },
    Error {
        text: String,
    },
}

/// Result and durable command lifecycle identity returned by the Host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecution {
    pub command_id: String,
    pub result: CommandResult,
}

/// Native RPC client for session-scoped human commands.
pub struct CommandsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> CommandsClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists commands registered for the live Agent identified by `agent_id`.
    pub fn list(&self, agent_id: &str) -> Result<Vec<CommandDescriptor>, Error> {
        self.call("commands/list", serde_json::json!({"agentId":agent_id}))
    }

    /// Executes one slash-command line without sending it to the model.
    ///
    /// The Host records the command lifecycle. An absent result means the
    /// syntax or command name did not resolve and the caller may submit the
    /// original line as an ordinary prompt.
    pub fn execute(
        &self,
        agent_id: &str,
        line: &str,
        attachments: Vec<CommandSubmitAttachment>,
    ) -> Result<Option<CommandExecution>, Error> {
        self.call(
            "commands/execute",
            serde_json::json!({
                "agentId": agent_id,
                "line": line,
                "submittedAttachments": attachments,
            }),
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
    fn command_response_decodes_optional_metadata_and_outcomes() {
        let command: CommandDescriptor = serde_json::from_value(json!({
            "definitionId":"@deepseek-ai/dsh-plan-mode",
            "name":"plan",
            "description":"Enter or leave plan mode",
            "input":{"hint":"[off|message]","attachments":true}
        }))
        .unwrap();
        assert_eq!(command.name, "plan");
        assert_eq!(command.input.as_ref().unwrap().attachments, Some(true));
        assert!(matches!(
            serde_json::from_value::<CommandResult>(
                json!({"kind":"success","text":"Plan mode on"})
            ),
            Ok(CommandResult::Success {
                source_event_seq: None,
                ..
            })
        ));
        assert_eq!(
            serde_json::from_value::<CommandResult>(json!({"kind":"error","text":"Cannot run"}))
                .unwrap(),
            CommandResult::Error {
                text: "Cannot run".into()
            }
        );
    }

    #[test]
    fn file_command_attachment_keeps_its_receipt_identity() {
        assert_eq!(
            serde_json::to_value(CommandSubmitAttachment::File {
                receipt_id: "receipt-1".into(),
            })
            .unwrap(),
            json!({"type":"file","receiptId":"receipt-1"})
        );
    }
}
