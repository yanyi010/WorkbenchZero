//! Storage service: application directories and per-plugin state stores.
//!
//! Layout (XDG on Linux):
//!   ~/.config/eigendesk/            global settings, workspaces registry
//!   ~/.local/share/eigendesk/       data: plugins, registry, logs, secrets
//!   ~/.cache/eigendesk/             disposable cache
//!
//! Inside a workspace, `.workbench/` holds (spec §8):
//!   workspace.json   workspace metadata + schema version
//!   index.sqlite     FTS index / artifacts / recents (derived data)
//!   settings.json    workspace-scoped settings
//!   layout.json      serialized layout
//!   plugin-state/    per-plugin canonical state (JSON key-value + data files)
//!   cache/           disposable

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("quota exceeded for plugin `{0}` ({1} bytes limit)")]
    QuotaExceeded(String, u64),
    #[error("key `{0}` is invalid")]
    InvalidKey(String),
}

/// Maximum bytes a plugin may store (canonical state), default 64 MiB.
pub const PLUGIN_STATE_QUOTA: u64 = 64 * 1024 * 1024;
/// Maximum serialized size of a single state value.
pub const MAX_STATE_VALUE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    /// Installed (user) plugins.
    pub plugins: PathBuf,
    /// Dev-mode plugins (hot reload target for `wb plugin dev`).
    pub dev_plugins: PathBuf,
    /// Local plugin registry index (store catalog).
    pub registry: PathBuf,
    /// Bundled first-party plugin packages shipped with the app.
    pub bundled: Option<PathBuf>,
}

impl Dirs {
    /// Resolve all application directories. `data_override` is used by tests
    /// and the standalone MCP server binary.
    pub fn init(
        bundled: Option<PathBuf>,
        data_override: Option<PathBuf>,
    ) -> Result<Self, StorageError> {
        let (config, data, cache) = match &data_override {
            Some(base) => (base.join("config"), base.clone(), base.join("cache")),
            None => (
                dirs::config_dir()
                    .ok_or_else(|| io_err("no config dir"))?
                    .join("eigendesk"),
                dirs::data_dir()
                    .ok_or_else(|| io_err("no data dir"))?
                    .join("eigendesk"),
                dirs::cache_dir()
                    .ok_or_else(|| io_err("no cache dir"))?
                    .join("eigendesk"),
            ),
        };
        let logs = data.join("logs");
        let plugins = data.join("plugins");
        let dev_plugins = data.join("dev-plugins");
        let registry = data.join("registry");
        for dir in [
            &config,
            &data,
            &cache,
            &logs,
            &plugins,
            &dev_plugins,
            &registry,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(Self {
            config,
            data,
            cache,
            logs,
            plugins,
            dev_plugins,
            registry,
            bundled,
        })
    }
}

fn io_err(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, msg)
}

/// Sanitize a plugin id (`publisher.name`) into a safe directory name.
pub fn sanitize_plugin_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Validate a state key.
pub fn valid_state_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' || c == '/')
        && !key.contains("..")
}

/// Persistent, per-plugin JSON key-value state under
/// `<workspace>/.workbench/plugin-state/<plugin>/state.json`, plus a `data/`
/// directory for plugin-owned files (both canonical, spec §98).
pub struct PluginStateStore {
    base: PathBuf,
    map: RwLock<HashMap<String, serde_json::Value>>,
}

impl PluginStateStore {
    pub fn open(workspace_state_root: &Path, plugin_id: &str) -> Result<Self, StorageError> {
        let base = workspace_state_root.join(sanitize_plugin_id(plugin_id));
        std::fs::create_dir_all(base.join("data"))?;
        let state_path = base.join("state.json");
        let map = if state_path.exists() {
            let raw = std::fs::read_to_string(&state_path)?;
            if raw.trim().is_empty() {
                HashMap::new()
            } else {
                serde_json::from_str(&raw)?
            }
        } else {
            HashMap::new()
        };
        Ok(Self {
            base,
            map: RwLock::new(map),
        })
    }

    fn state_path(&self) -> PathBuf {
        self.base.join("state.json")
    }

    fn dir_size(path: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    total += Self::dir_size(&p);
                } else if let Ok(md) = entry.metadata() {
                    total += md.len();
                }
            }
        }
        total
    }

    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.map.read().unwrap().get(key).cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.map.read().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn set(&self, key: &str, value: serde_json::Value) -> Result<(), StorageError> {
        if !valid_state_key(key) {
            return Err(StorageError::InvalidKey(key.to_string()));
        }
        let serialized = serde_json::to_string(&value)?;
        if serialized.len() as u64 > MAX_STATE_VALUE {
            return Err(StorageError::QuotaExceeded(
                key.to_string(),
                MAX_STATE_VALUE,
            ));
        }
        let mut map = self.map.write().unwrap();
        map.insert(key.to_string(), value);
        let payload = serde_json::to_string_pretty(&*map)?;
        if (payload.len() as u64) + Self::dir_size(&self.base.join("data")) > PLUGIN_STATE_QUOTA {
            map.remove(key);
            return Err(StorageError::QuotaExceeded(
                key.to_string(),
                PLUGIN_STATE_QUOTA,
            ));
        }
        let tmp = self.state_path().with_extension("tmp");
        std::fs::write(&tmp, payload)?;
        std::fs::rename(&tmp, self.state_path())?;
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<bool, StorageError> {
        let mut map = self.map.write().unwrap();
        let existed = map.remove(key).is_some();
        if existed {
            let payload = serde_json::to_string_pretty(&*map)?;
            let tmp = self.state_path().with_extension("tmp");
            std::fs::write(&tmp, payload)?;
            std::fs::rename(&tmp, self.state_path())?;
        }
        Ok(existed)
    }

    /// Plugin-owned data directory (canonical files).
    pub fn data_dir(&self) -> PathBuf {
        self.base.join("data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_roundtrip_and_persistence() {
        let root = std::env::temp_dir().join(format!("ed-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        {
            let store = PluginStateStore::open(&root, "test.plugin").unwrap();
            store.set("counter", serde_json::json!(42)).unwrap();
            store
                .set("nested/key", serde_json::json!({"a": 1}))
                .unwrap();
            assert_eq!(store.get("counter"), Some(serde_json::json!(42)));
        }
        let store = PluginStateStore::open(&root, "test.plugin").unwrap();
        assert_eq!(store.get("counter"), Some(serde_json::json!(42)));
        assert_eq!(store.get("nested/key"), Some(serde_json::json!({"a": 1})));
        assert!(store.delete("counter").unwrap());
        assert_eq!(store.get("counter"), None);
    }

    #[test]
    fn invalid_keys_rejected() {
        let root = std::env::temp_dir().join(format!("ed-store-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = PluginStateStore::open(&root, "test.plugin").unwrap();
        assert!(store.set("", serde_json::json!(1)).is_err());
        assert!(store.set("a/../b", serde_json::json!(1)).is_err());
        assert!(store.set(&"x".repeat(200), serde_json::json!(1)).is_err());
    }

    #[test]
    fn plugin_ids_sanitized() {
        assert_eq!(
            sanitize_plugin_id("yan-yi.slurm-monitor"),
            "yan-yi.slurm-monitor"
        );
        assert_eq!(sanitize_plugin_id("evil/../id"), "evil_.._id");
    }
}
