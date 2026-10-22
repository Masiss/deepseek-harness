//! Wire envelopes stay identical to the browser Connection RPC protocol so native callers can interoperate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    pub rpc_id: String,
    pub method: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    pub rpc_id: String,
    pub result: ResponseResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ResponseResult {
    Success { ok: bool, value: Value },
    Failure { ok: bool, error: RpcError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub details: Value,
}

// The wire format carries `ok` as a JSON boolean, so the boolean is read
// first and the variant is chosen from it. serde's tagged/untagged enums
// cannot express "variant depends on a boolean tag" directly.
impl<'de> serde::Deserialize<'de> for ResponseResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            ok: bool,
            #[serde(default)]
            value: Option<Value>,
            #[serde(default)]
            error: Option<RpcError>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.ok {
            Ok(ResponseResult::Success {
                ok: true,
                value: raw.value.unwrap_or(Value::Null),
            })
        } else {
            Ok(ResponseResult::Failure {
                ok: false,
                error: raw.error.unwrap_or_else(|| RpcError {
                    code: "unknown".to_owned(),
                    message: String::new(),
                    details: Value::Null,
                }),
            })
        }
    }
}

impl RequestEnvelope {
    pub fn new(rpc_id: impl Into<String>, method: impl Into<String>, payload: Value) -> Self {
        Self { message_type: "client-request".into(), rpc_id: rpc_id.into(), method: method.into(), payload }
    }
}

impl ResponseEnvelope {
    pub fn validate(&self, expected_rpc_id: &str) -> Result<&ResponseResult, crate::Error> {
        if self.message_type != "server-response" { return Err(crate::Error::InvalidEnvelope("server-response type")); }
        if self.rpc_id != expected_rpc_id { return Err(crate::Error::CorrelationMismatch { expected: expected_rpc_id.into(), actual: self.rpc_id.clone() }); }
        Ok(&self.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_browser_request() {
        let value = serde_json::to_value(RequestEnvelope::new("r1", "goals/get", serde_json::json!({"args":{}}))).unwrap();
        assert_eq!(value, serde_json::json!({"type":"client-request","rpc_id":"r1","method":"goals/get","payload":{"args":{}}}));
    }
    #[test]
    fn validates_success_and_correlation() {
        let response: ResponseEnvelope = serde_json::from_value(serde_json::json!({"type":"server-response","rpc_id":"r1","result":{"ok":true,"value":7}})).unwrap();
        assert!(response.validate("r1").is_ok());
        assert!(matches!(response.validate("r2"), Err(crate::Error::CorrelationMismatch { .. })));
    }
    #[test]
    fn preserves_error_shape() {
        let response: ResponseEnvelope = serde_json::from_value(serde_json::json!({"type":"server-response","rpc_id":"r1","result":{"ok":false,"error":{"code":"gateway/internal","message":"bad","details":{}}}})).unwrap();
        assert!(matches!(response.result, ResponseResult::Failure { .. }));
    }
}
