//! Typed schedule management RPCs for Desktop task screens.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;

/// A schedule record. The endpoint preserves the discriminated web-client record fields.
pub type ScheduleRecord = Value;
/// A catalog entry containing a schedule and its Session binding.
pub type ScheduleCatalogEntry = Value;
/// A schedule timing edit accepted by the Host.
pub type ScheduleTimingChange = Value;
/// Delivery history result returned by the Host.
pub type ScheduleDeliveryHistoryResult = Value;
/// Delete result returned by the Host.
pub type ScheduleDeleteResult = Value;
/// Update result returned by the Host.
pub type ScheduleUpdateResult = Value;

/// Session-scoped schedule list request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleListRequest {
    pub session_id: String,
}

/// Session and schedule identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleDeleteRequest {
    pub session_id: String,
    pub id: String,
}

/// Delivery history request with an explicit bounded page size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleHistoryRequest {
    pub session_id: String,
    pub id: String,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

/// Compare-and-update schedule request. `expected` and `change` retain the protocol's variant fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleUpdateRequest {
    pub session_id: String,
    pub id: String,
    pub expected: ScheduleRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ScheduleTimingChange>,
}

/// Native client for schedule listing and management.
pub struct SchedulesClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> SchedulesClient<'a> {
    /// Creates a schedule client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists schedules bound to one Session.
    pub fn list(&self, request: ScheduleListRequest) -> Result<Vec<ScheduleRecord>, Error> {
        self.call("schedule/list", request)
    }
    /// Lists the global schedule catalog.
    pub fn catalog(&self) -> Result<Vec<ScheduleCatalogEntry>, Error> {
        self.call_value("schedule/catalog", serde_json::json!({}))
    }
    /// Reads saved delivery history.
    pub fn history(
        &self,
        request: ScheduleHistoryRequest,
    ) -> Result<ScheduleDeliveryHistoryResult, Error> {
        self.call_value("schedule/history", serde_json::to_value(request)?)
    }
    /// Deletes one schedule.
    pub fn delete(&self, request: ScheduleDeleteRequest) -> Result<ScheduleDeleteResult, Error> {
        self.call_value("schedule/delete", serde_json::to_value(request)?)
    }
    /// Updates one schedule using compare-and-update semantics.
    pub fn update(&self, request: ScheduleUpdateRequest) -> Result<ScheduleUpdateResult, Error> {
        self.call_value("schedule/update", serde_json::to_value(request)?)
    }

    fn call<R: Serialize>(&self, method: &str, request: R) -> Result<Vec<Value>, Error> {
        self.call_value(method, serde_json::to_value(request)?)
    }
    fn call_value<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        payload: Value,
    ) -> Result<T, Error> {
        Ok(serde_json::from_value(self.transport.request(
            method,
            payload,
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_history_request() {
        let request = ScheduleHistoryRequest {
            session_id: "s".into(),
            id: "task".into(),
            limit: 25,
            before: Some("m".into()),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"sessionId":"s","id":"task","limit":25,"before":"m"})
        );
    }
    #[test]
    fn omits_update_optionals() {
        let request = ScheduleUpdateRequest {
            session_id: "s".into(),
            id: "task".into(),
            expected: serde_json::json!({"kind":"at"}),
            title: None,
            prompt: None,
            change: None,
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({"sessionId":"s","id":"task","expected":{"kind":"at"}})
        );
    }
    #[test]
    fn deserializes_delete_result() {
        let result: ScheduleDeleteResult =
            serde_json::from_value(serde_json::json!({"id":"task","deleted":true})).unwrap();
        assert_eq!(result["deleted"], true);
    }
}
