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

/// One installable or selected profile bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleInfo {
    pub name: String,
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub availability: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub installed: bool,
}

/// One running-profile entry with its persistent control availability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedPlugin {
    pub entry_id: String,
    pub module_name: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub fiber_phase: Option<PluginFiberPhase>,
    /// Present when the entry has a profile patch row the client may toggle.
    #[serde(default)]
    pub patch_id: Option<String>,
    /// Present when the entry cannot be changed from a client.
    #[serde(default)]
    pub read_only_reason: Option<String>,
}

/// Failure detail reported by a management change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagementError {
    pub code: String,
    #[serde(default)]
    pub diagnostic: Option<String>,
}

/// Outcome of one enablement or removal request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeResult {
    #[serde(default)]
    pub changed: bool,
    pub application: String,
    pub stage: String,
    pub target: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub restart_reason: Option<String>,
    #[serde(default)]
    pub error: Option<ManagementError>,
    #[serde(default)]
    pub warnings: Vec<String>,
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
        self.call("pluginInventory/list", serde_json::json!({}))
    }

    /// Lists profile entries that expose persistent enablement control.
    pub fn list_plugins(&self) -> Result<Vec<ManagedPlugin>, Error> {
        self.call("pluginManager/listPlugins", serde_json::json!({}))
    }

    /// Lists the profile's bundles and on-demand catalog entries.
    pub fn list_bundles(&self) -> Result<Vec<BundleInfo>, Error> {
        self.call("pluginManager/listBundles", serde_json::json!({}))
    }

    /// Enables or disables one loader entry.
    pub fn set_plugin_enabled(&self, id: &str, enabled: bool) -> Result<ChangeResult, Error> {
        self.call(
            "pluginManager/setPluginEnabled",
            serde_json::json!({"id": id, "enabled": enabled}),
        )
    }

    /// Enables or disables one profile bundle.
    pub fn set_bundle_enabled(&self, name: &str, enabled: bool) -> Result<ChangeResult, Error> {
        self.call(
            "pluginManager/setBundleEnabled",
            serde_json::json!({"name": name, "enabled": enabled}),
        )
    }

    /// Removes one installed profile bundle.
    pub fn remove_bundle(&self, name: &str) -> Result<ChangeResult, Error> {
        self.call(
            "pluginManager/removeBundle",
            serde_json::json!({"name": name}),
        )
    }

    fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        payload: serde_json::Value,
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
    fn deserializes_bundles_and_change_results() {
        let bundles: Vec<BundleInfo> = serde_json::from_value(serde_json::json!([
            {"name":"@deepseek-ai/dsh-web-app","official":true,"availability":"profile","enabled":true,"installed":true,"version":"1.0.0"}
        ]))
        .unwrap();
        assert!(bundles[0].enabled);
        let plugins: Vec<ManagedPlugin> = serde_json::from_value(serde_json::json!([
            {"entryId":"e","moduleName":"m","enabled":true,"fiberPhase":"active","patchId":"p"},
            {"entryId":"f","moduleName":"n","enabled":false,"readOnlyReason":"unmanaged-plugin"}
        ]))
        .unwrap();
        assert_eq!(plugins[0].patch_id.as_deref(), Some("p"));
        assert_eq!(
            plugins[1].read_only_reason.as_deref(),
            Some("unmanaged-plugin")
        );
        let result: ChangeResult = serde_json::from_value(serde_json::json!({
            "changed":true,"application":"restart-required","stage":"enable",
            "target":"demo","enabled":false,"restartReason":"no-hmr",
            "error":{"code":"operation-error","diagnostic":"boom"},"warnings":["w"]
        }))
        .unwrap();
        assert_eq!(result.application, "restart-required");
        assert_eq!(result.error.unwrap().diagnostic.as_deref(), Some("boom"));
        assert_eq!(result.warnings, ["w"]);
    }

    #[test]
    fn management_calls_use_the_gateway_methods_and_arguments() {
        use httptest::{matchers::*, responders::json_encoded, Expectation, Server};
        let server = Server::run();
        let applied = serde_json::json!({
            "changed": true, "application": "applied", "stage": "enable", "target": "p"
        });
        for (index, (method, args)) in [
            ("pluginManager/setPluginEnabled", serde_json::json!({"id": "p", "enabled": false})),
            ("pluginManager/setBundleEnabled", serde_json::json!({"name": "b", "enabled": true})),
            ("pluginManager/removeBundle", serde_json::json!({"name": "b"})),
        ]
        .into_iter()
        .enumerate()
        {
            let rpc = format!("native-{}", index + 1);
            server.expect(
                Expectation::matching(all_of![
                    request::method_path("POST", format!("/api/{method}")),
                    request::body(json_decoded(eq(serde_json::json!({
                        "type": "client-request",
                        "rpcId": rpc,
                        "method": method,
                        "payload": {"args": args}
                    }))))
                ])
                .respond_with(json_encoded(serde_json::json!({
                    "type": "server-response",
                    "rpcId": rpc,
                    "result": {"ok": true, "value": applied}
                }))),
            );
        }
        let transport = HttpTransport::new(&server.url_str("/")).unwrap();
        let client = PluginsClient::new(&transport);
        assert!(client.set_plugin_enabled("p", false).unwrap().changed);
        assert!(client.set_bundle_enabled("b", true).unwrap().changed);
        assert!(client.remove_bundle("b").unwrap().changed);
    }

    #[test]
    fn serializes_and_deserializes_inventory() {
        let value = serde_json::json!({"managementAvailable":true,"entries":[{"entryId":"e","moduleName":"m","enabled":true,"fiberPhase":"active"}]});
        let snapshot: PluginInventorySnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(snapshot.entries[0].entry_id, "e");
        assert_eq!(serde_json::to_value(snapshot).unwrap(), value);
    }
}
