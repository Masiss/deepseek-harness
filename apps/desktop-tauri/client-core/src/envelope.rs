//! Wire envelopes stay identical to the browser Connection RPC protocol so native callers can interoperate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(rename = "rpcId")]
    pub rpc_id: String,
    pub method: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(rename = "rpcId")]
    pub rpc_id: String,
    pub result: ResponseResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ResponseResult {
    Success { ok: bool, value: Value },
    Failure { ok: bool, error: RpcError },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    pub details: serde_json::Map<String, Value>,
}

// The wire result is a boolean-tagged object. Void success may omit `value`;
// failures must carry their structured error for callers to handle them.
impl<'de> serde::Deserialize<'de> for ResponseResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            ok: bool,
            value: Option<Value>,
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
                error: raw
                    .error
                    .ok_or_else(|| serde::de::Error::missing_field("error"))?,
            })
        }
    }
}

impl RequestEnvelope {
    pub fn new(rpc_id: impl Into<String>, method: impl Into<String>, payload: Value) -> Self {
        Self {
            message_type: "client-request".into(),
            rpc_id: rpc_id.into(),
            method: method.into(),
            payload: serde_json::json!({ "args": payload }),
        }
    }
}

impl ResponseEnvelope {
    pub fn validate(&self, expected_rpc_id: &str) -> Result<&ResponseResult, crate::Error> {
        if self.message_type != "server-response" {
            return Err(crate::Error::InvalidEnvelope("server-response type"));
        }
        if self.rpc_id != expected_rpc_id {
            return Err(crate::Error::CorrelationMismatch {
                expected: expected_rpc_id.into(),
                actual: self.rpc_id.clone(),
            });
        }
        Ok(&self.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_browser_request() {
        let value = serde_json::to_value(RequestEnvelope::new(
            "r1",
            "goals/get",
            serde_json::json!({}),
        ))
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"type":"client-request","rpcId":"r1","method":"goals/get","payload":{"args":{}}})
        );
    }
    #[test]
    fn validates_success_and_correlation() {
        let response: ResponseEnvelope = serde_json::from_value(serde_json::json!({"type":"server-response","rpcId":"r1","result":{"ok":true,"value":7}})).unwrap();
        assert!(response.validate("r1").is_ok());
        assert!(matches!(
            response.validate("r2"),
            Err(crate::Error::CorrelationMismatch { .. })
        ));
    }
    #[test]
    fn preserves_error_shape() {
        let response: ResponseEnvelope = serde_json::from_value(serde_json::json!({"type":"server-response","rpcId":"r1","result":{"ok":false,"error":{"code":"gateway/internal","message":"bad","details":{}}}})).unwrap();
        assert!(matches!(response.result, ResponseResult::Failure { .. }));
    }

    #[test]
    fn accepts_void_success_and_rejects_failure_without_error() {
        let void: ResponseEnvelope = serde_json::from_value(
            serde_json::json!({"type":"server-response","rpcId":"r1","result":{"ok":true}}),
        )
        .unwrap();
        assert!(matches!(void.result, ResponseResult::Success { value, .. } if value.is_null()));
        assert!(serde_json::from_value::<ResponseEnvelope>(
            serde_json::json!({"type":"server-response","rpcId":"r1","result":{"ok":false}})
        )
        .is_err());
    }

    #[test]
    fn serializes_response_result_as_connection_wire_object() {
        let result = ResponseResult::Failure {
            ok: false,
            error: RpcError {
                code: "gateway/internal".to_owned(),
                message: "failed".to_owned(),
                details: serde_json::Map::from_iter([("retryable".to_owned(), Value::Bool(false))]),
            },
        };
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({"ok":false,"error":{"code":"gateway/internal","message":"failed","details":{"retryable":false}}})
        );
    }
}
