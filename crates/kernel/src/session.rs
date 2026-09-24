//! Session-scoped in-memory key-value store (spec §9: `session` scope).
//! Cleared when the workspace closes or the app exits; shared by all
//! surfaces (logic iframe + view iframes) of one plugin.

use std::collections::HashMap;
use std::sync::RwLock;

use wz_common::RwLockRecover;

#[derive(Default)]
pub struct SessionKv {
    data: RwLock<HashMap<(String, String), serde_json::Value>>,
}

impl SessionKv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, plugin_id: &str, key: &str) -> Option<serde_json::Value> {
        self.data
            .read_or_recover()
            .get(&(plugin_id.to_string(), key.to_string()))
            .cloned()
    }

    pub fn set(&self, plugin_id: &str, key: &str, value: serde_json::Value) {
        self.data
            .write_or_recover()
            .insert((plugin_id.to_string(), key.to_string()), value);
    }

    pub fn delete(&self, plugin_id: &str, key: &str) -> bool {
        self.data
            .write_or_recover()
            .remove(&(plugin_id.to_string(), key.to_string()))
            .is_some()
    }

    pub fn keys(&self, plugin_id: &str) -> Vec<String> {
        let mut keys: Vec<String> = self
            .data
            .read_or_recover()
            .keys()
            .filter(|(p, _)| p == plugin_id)
            .map(|(_, k)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    pub fn clear(&self) {
        self.data.write_or_recover().clear();
    }
}
