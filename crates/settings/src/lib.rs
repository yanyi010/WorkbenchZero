//! Settings service with global / workspace scopes (spec §9, §42-43).
//!
//! Every configurable entity declares a scope. Workspace settings override
//! compatible global defaults, which override descriptor defaults. Settings
//! schemas are plugin-defined; the UI is auto-generated from descriptors.
//! Values live in JSON files (human-readable ownership, spec §2.7), written
//! atomically.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("unknown setting `{0}`")]
    UnknownKey(String),
    #[error("invalid value for `{key}`: {reason}")]
    InvalidValue { key: String, reason: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("setting `{0}` requires a workspace to be open")]
    NeedsWorkspace(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    Boolean,
    Number,
    String,
    Enum,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingDescriptor {
    /// Dotted key: `pluginId.settingName` or `core.section.key`.
    pub key: String,
    #[serde(rename = "type")]
    pub r#type: SettingType,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
}

struct Stores {
    global_path: PathBuf,
    global: HashMap<String, serde_json::Value>,
    workspace_path: Option<PathBuf>,
    workspace: HashMap<String, serde_json::Value>,
}

pub struct SettingsService {
    descriptors: RwLock<Vec<SettingDescriptor>>,
    stores: RwLock<Stores>,
}

fn load_json(path: &Path) -> Result<HashMap<String, serde_json::Value>, SettingsError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    Ok(serde_json::from_str(&raw)?)
}

fn save_json(path: &Path, map: &HashMap<String, serde_json::Value>) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(map)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl SettingsService {
    pub fn new(global_path: PathBuf) -> Result<Self, SettingsError> {
        Ok(Self {
            descriptors: RwLock::new(Vec::new()),
            stores: RwLock::new(Stores {
                global: load_json(&global_path)?,
                global_path,
                workspace_path: None,
                workspace: HashMap::new(),
            }),
        })
    }

    /// Attach (or detach) the workspace-scoped store.
    pub fn set_workspace(&self, path: Option<PathBuf>) -> Result<(), SettingsError> {
        let mut stores = self.stores.write().unwrap();
        match path {
            Some(p) => {
                let map = load_json(&p)?;
                stores.workspace_path = Some(p);
                stores.workspace = map;
            }
            None => {
                stores.workspace_path = None;
                stores.workspace.clear();
            }
        }
        Ok(())
    }

    pub fn register_descriptors(&self, descriptors: Vec<SettingDescriptor>) {
        let mut all = self.descriptors.write().unwrap();
        for d in descriptors {
            all.retain(|x| x.key != d.key);
            all.push(d);
        }
    }

    pub fn unregister_by_plugin(&self, plugin_id: &str) {
        self.descriptors
            .write()
            .unwrap()
            .retain(|d| d.plugin_id.as_deref() != Some(plugin_id));
    }

    pub fn descriptors(&self) -> Vec<SettingDescriptor> {
        self.descriptors.read().unwrap().clone()
    }

    pub fn descriptor(&self, key: &str) -> Option<SettingDescriptor> {
        self.descriptors
            .read()
            .unwrap()
            .iter()
            .find(|d| d.key == key)
            .cloned()
    }

    fn validate(&self, key: &str, value: &serde_json::Value) -> Result<(), SettingsError> {
        let Some(d) = self.descriptor(key) else {
            return Err(SettingsError::UnknownKey(key.to_string()));
        };
        let ok = match d.r#type {
            SettingType::Boolean => value.is_boolean(),
            SettingType::Number => value.is_number(),
            SettingType::String => value.is_string(),
            SettingType::Enum => value
                .as_str()
                .map(|s| d.enum_values.iter().any(|v| v == s))
                .unwrap_or(false),
        };
        if !ok {
            return Err(SettingsError::InvalidValue {
                key: key.to_string(),
                reason: format!("expected {}, got {value}", type_name(&d.r#type)),
            });
        }
        Ok(())
    }

    /// Effective value: workspace override -> global -> descriptor default -> null.
    pub fn get(&self, key: &str) -> serde_json::Value {
        {
            let stores = self.stores.read().unwrap();
            if let Some(v) = stores.workspace.get(key) {
                return v.clone();
            }
            if let Some(v) = stores.global.get(key) {
                return v.clone();
            }
        }
        self.descriptor(key)
            .and_then(|d| d.default)
            .unwrap_or(serde_json::Value::Null)
    }

    /// All effective values for descriptors (UI display).
    pub fn get_all(&self) -> HashMap<String, serde_json::Value> {
        self.descriptors
            .read()
            .unwrap()
            .iter()
            .map(|d| (d.key.clone(), self.get(&d.key)))
            .collect()
    }

    pub fn set(
        &self,
        scope: Scope,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), SettingsError> {
        self.validate(key, &value)?;
        let mut stores = self.stores.write().unwrap();
        match scope {
            Scope::Global => {
                stores.global.insert(key.to_string(), value);
                save_json(&stores.global_path, &stores.global)
            }
            Scope::Workspace => {
                let Some(path) = stores.workspace_path.clone() else {
                    return Err(SettingsError::NeedsWorkspace(key.to_string()));
                };
                stores.workspace.insert(key.to_string(), value);
                save_json(&path, &stores.workspace)
            }
        }
    }

    pub fn reset(&self, scope: Scope, key: &str) -> Result<(), SettingsError> {
        if self.descriptor(key).is_none() {
            return Err(SettingsError::UnknownKey(key.to_string()));
        }
        let mut stores = self.stores.write().unwrap();
        match scope {
            Scope::Global => {
                stores.global.remove(key);
                save_json(&stores.global_path, &stores.global)
            }
            Scope::Workspace => {
                let Some(path) = stores.workspace_path.clone() else {
                    return Err(SettingsError::NeedsWorkspace(key.to_string()));
                };
                stores.workspace.remove(key);
                save_json(&path, &stores.workspace)
            }
        }
    }
}

fn type_name(t: &SettingType) -> &'static str {
    match t {
        SettingType::Boolean => "boolean",
        SettingType::Number => "number",
        SettingType::String => "string",
        SettingType::Enum => "one of the enum values",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(dir: &std::path::Path) -> SettingsService {
        SettingsService::new(dir.join("settings.json")).unwrap()
    }

    fn bool_desc(key: &str, scope: Scope) -> SettingDescriptor {
        SettingDescriptor {
            key: key.into(),
            r#type: SettingType::Boolean,
            title: key.into(),
            description: None,
            default: Some(serde_json::json!(false)),
            enum_values: vec![],
            scope,
            plugin_id: None,
        }
    }

    #[test]
    fn precedence_workspace_overrides_global() {
        let dir = std::env::temp_dir().join(format!("ed-settings-{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = svc(&dir);
        s.register_descriptors(vec![bool_desc("core.test.flag", Scope::Workspace)]);
        assert_eq!(s.get("core.test.flag"), serde_json::json!(false)); // default
        s.set(Scope::Global, "core.test.flag", serde_json::json!(true))
            .unwrap();
        assert_eq!(s.get("core.test.flag"), serde_json::json!(true)); // global
        s.set_workspace(Some(dir.join("ws-settings.json"))).unwrap();
        s.set(Scope::Workspace, "core.test.flag", serde_json::json!(false))
            .unwrap();
        assert_eq!(s.get("core.test.flag"), serde_json::json!(false)); // workspace wins
    }

    #[test]
    fn unknown_key_rejected() {
        let dir = std::env::temp_dir().join(format!("ed-settings-{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = svc(&dir);
        assert!(matches!(
            s.set(Scope::Global, "nope.key", serde_json::json!(1)),
            Err(SettingsError::UnknownKey(_))
        ));
    }

    #[test]
    fn type_validation() {
        let dir = std::env::temp_dir().join(format!("ed-settings-{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = svc(&dir);
        s.register_descriptors(vec![bool_desc("core.test.b", Scope::Global)]);
        assert!(s
            .set(Scope::Global, "core.test.b", serde_json::json!("yes"))
            .is_err());
        assert!(s
            .set(Scope::Global, "core.test.b", serde_json::json!(true))
            .is_ok());
    }

    #[test]
    fn workspace_scope_needs_workspace() {
        let dir = std::env::temp_dir().join(format!("ed-settings-{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = svc(&dir);
        s.register_descriptors(vec![bool_desc("core.test.w", Scope::Workspace)]);
        assert!(matches!(
            s.set(Scope::Workspace, "core.test.w", serde_json::json!(true)),
            Err(SettingsError::NeedsWorkspace(_))
        ));
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("ed-settings-{}", uuid_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        {
            let s = svc(&dir);
            s.register_descriptors(vec![bool_desc("core.test.p", Scope::Global)]);
            s.set(Scope::Global, "core.test.p", serde_json::json!(true))
                .unwrap();
        }
        let s2 = svc(&dir);
        s2.register_descriptors(vec![bool_desc("core.test.p", Scope::Global)]);
        assert_eq!(s2.get("core.test.p"), serde_json::json!(true));
    }

    fn uuid_v4() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{nanos:x}")
    }
}
