//! Typed model-selection RPCs for the Desktop model selector.
//!
//! These types mirror the web client's `ModelCatalog` and model-selection
//! request so native callers send the same JSON as browser callers.

use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

use crate::{transport::HttpTransport, Error};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelReasoningEffort {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReasoning {
    pub efforts: Vec<ModelReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogModel {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ModelReasoning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProviderGroup {
    pub id: String,
    pub name: String,
    pub models: Vec<ModelCatalogModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogFailure {
    pub id: String,
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub default: ModelSelection,
    pub routable_providers: Vec<String>,
    pub groups: Vec<ModelProviderGroup>,
    pub failures: Vec<ModelCatalogFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectModelRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub selection: ModelSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectModelResponse {
    pub selected: ModelSelection,
}

pub struct ModelClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> ModelClient<'a> {
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    pub fn catalog(&self) -> Result<ModelCatalog, Error> {
        let value = self.transport.request(
            "session/modelCatalog",
            serde_json::json!({}),
            &AtomicBool::new(false),
        )?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn select(&self, request: SelectModelRequest) -> Result<SelectModelResponse, Error> {
        let value = self.transport.request(
            "session/selectModel",
            serde_json::json!({"request": request}),
            &AtomicBool::new(false),
        )?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn initialize_default_model(&self) -> Result<(), Error> {
        let value = self.transport.request(
            "session/initializeDefaultModel",
            serde_json::json!({}),
            &AtomicBool::new(false),
        )?;
        Ok(serde_json::from_value(value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_initialization_accepts_void_without_inventing_a_receipt() {
        use httptest::{matchers::*, responders::json_encoded, Expectation, Server};

        for (value, succeeds) in [
            (serde_json::Value::Null, true),
            (serde_json::json!({"accepted": true}), false),
        ] {
            let server = Server::run();
            server.expect(
                Expectation::matching(request::body(json_decoded(eq(serde_json::json!({
                    "type": "client-request", "rpcId": "native-1",
                    "method": "session/initializeDefaultModel", "payload": {"args": {}}
                })))))
                .respond_with(json_encoded(serde_json::json!({
                    "type": "server-response", "rpcId": "native-1",
                    "result": {"ok": true, "value": value}
                }))),
            );
            let client = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap();
            let transport = HttpTransport::with_client(&server.url_str("/"), client).unwrap();
            assert_eq!(
                ModelClient::new(&transport)
                    .initialize_default_model()
                    .is_ok(),
                succeeds
            );
        }
    }

    #[test]
    fn reads_server_catalog_field_names() {
        let catalog: ModelCatalog = serde_json::from_value(serde_json::json!({
            "default": {"provider": "p", "model": "m"}, "routableProviders": ["p"],
            "groups": [{"id": "p", "name": "Provider", "models": [{
                "id": "m", "name": "Model", "reasoning": {"efforts": [], "defaultEffort": "high"}
            }]}], "failures": []
        }))
        .unwrap();
        assert_eq!(catalog.routable_providers, vec!["p"]);
        assert_eq!(
            catalog.groups[0].models[0]
                .reasoning
                .as_ref()
                .unwrap()
                .default_effort
                .as_deref(),
            Some("high")
        );
    }
    #[test]
    fn serializes_selection_request() {
        let request = SelectModelRequest {
            session_id: "s".into(),
            selection: ModelSelection {
                provider: "p".into(),
                model: "m".into(),
                reasoning_effort: Some("high".into()),
            },
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"sessionId":"s","provider":"p","model":"m","reasoningEffort":"high"})
        );
    }
}
