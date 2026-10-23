//! Typed marketplace and install RPCs for the Desktop plugin surface.

use crate::{transport::HttpTransport, Error};
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;

/// Native client for marketplace search and bundle installation.
pub struct MarketplaceClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> MarketplaceClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Lists configured market indexes.
    pub fn list_indexes(&self) -> Result<Value, Error> {
        self.call("pluginMarket/listIndexes", json!({}))
    }

    /// Searches one index.
    pub fn search(&self, url: &str, text: &str) -> Result<Value, Error> {
        self.call(
            "pluginMarket/search",
            json!({
                "url": url,
                "query": {
                    "text": text,
                    "sort": {"kind": "relevance"},
                    "offset": 0,
                    "limit": 20
                }
            }),
        )
    }

    /// Inspects one install spec.
    pub fn inspect(&self, spec: &str) -> Result<Value, Error> {
        self.call("pluginManager/inspect", json!({"spec": spec}))
    }

    /// Starts installing one spec.
    pub fn install_bundle(&self, spec: &str) -> Result<Value, Error> {
        self.call(
            "pluginManager/installBundle",
            json!({"spec": spec, "options": {}}),
        )
    }

    /// Waits for an installation to finish.
    pub fn wait_for_install(&self, request_id: &str) -> Result<Value, Error> {
        self.call("pluginManager/waitForInstall", json!({"requestId": request_id}))
    }

    /// Cancels an installation.
    pub fn cancel_install(&self, request_id: &str) -> Result<Value, Error> {
        self.call("pluginManager/cancelInstall", json!({"requestId": request_id}))
    }

    fn call(&self, method: &str, payload: Value) -> Result<Value, Error> {
        Ok(self.transport.request(method, payload, &AtomicBool::new(false))?)
    }
}
