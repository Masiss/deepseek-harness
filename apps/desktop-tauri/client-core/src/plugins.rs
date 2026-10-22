//! Typed plugin inventory RPC for the Desktop settings surface.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

/// Loader lifecycle phase for one plugin entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginFiberPhase {
    Pending,
    Loading,
    Active,
    Failed,
    Unloading,
}

/// One configured plugin entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInventoryEntry {
    pub entry_id: String,
    pub module_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub enabled: bool,
    pub fiber_phase: Option<PluginFiberPhase>,
}

/// Enablement of one preset composition row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PresetPluginEnablement {
    Boolean(bool),
    Conditional(String),
}

/// One row in a preset composition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPresetPluginRow {
    pub entry_id: Option<String>,
    pub module_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub enabled: PresetPluginEnablement,
    #[serde(default)]
    pub condition: Option<String>,
    pub fiber_phase: Option<PluginFiberPhase>,
}

/// One preset's flattened plugin composition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPresetPluginGroup {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub is_default: bool,
    #[serde(default)]
    pub broken: Option<String>,
    pub rows: Vec<AgentPresetPluginRow>,
}

/// Point-in-time plugin inventory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInventorySnapshot {
    #[serde(default)]
    pub management_available: Option<bool>,
    pub entries: Vec<PluginInventoryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_presets: Option<Vec<AgentPresetPluginGroup>>,
}

/// Native client for plugin inventory.
pub struct PluginsClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> PluginsClient<'a> {
    /// Creates a plugin inventory client.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists configured plugins and preset compositions.
    pub fn list(&self) -> Result<PluginInventorySnapshot, Error> {
        Ok(serde_json::from_value(self.transport.request(
            "pluginInventory/list",
            serde_json::json!({}),
            &AtomicBool::new(false),
        )?)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_and_deserializes_inventory() {
        let value = serde_json::json!({"managementAvailable":true,"entries":[{"entryId":"e","moduleName":"m","enabled":true,"fiberPhase":"active"}]});
        let snapshot: PluginInventorySnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(snapshot.entries[0].entry_id, "e");
        assert_eq!(serde_json::to_value(snapshot).unwrap(), value);
    }
}
