//! Secrets service (spec §44-45).
//!
//! Primary backend: the OS keychain (Linux Secret Service via D-Bus, macOS
//! Keychain, Windows Credential Manager) using the `keyring` crate.
//!
//! Documented fallback (see docs/adr/ADR-0007-secrets-fallback.md): on
//! headless Linux boxes without a Secret Service daemon, secrets fall back to
//! a restricted-permission file store (`0600`, XDG data dir) and the UI
//! surfaces a warning. Normal config files are never used for secrets.
//!
//! Access is owner-scoped: a plugin can only read/write its own namespace
//! (`pluginId/key`). Secrets are never exposed to UI JavaScript except
//! through the owning plugin's explicit request.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::Serialize;
use thiserror::Error;

const SERVICE: &str = "eigendesk";

#[derive(Debug, Error)]
pub enum SecretsError {
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid key `{0}`")]
    InvalidKey(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    SecretService,
    LocalFile,
}

pub struct SecretsService {
    backend: Backend,
    fallback_path: Option<PathBuf>,
    fallback: RwLock<BTreeMap<String, String>>,
}

fn namespaced(plugin_id: &str, key: &str) -> String {
    format!("{plugin_id}/{key}")
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

impl SecretsService {
    /// Create the service, probing the OS keychain. `fallback_dir` is the
    /// application data dir used when the keychain is unavailable.
    pub fn new(fallback_dir: PathBuf) -> Self {
        let backend = Self::probe_backend();
        let (backend, fallback_path, fallback) = match backend {
            Backend::SecretService => (Backend::SecretService, None, BTreeMap::new()),
            Backend::LocalFile => {
                let path = fallback_dir.join("secrets.json");
                let map = load_fallback(&path).unwrap_or_default();
                (Backend::LocalFile, Some(path), map)
            }
        };
        Self { backend, fallback_path, fallback: RwLock::new(fallback) }
    }

    fn probe_backend() -> Backend {
        // Probe with a throwaway entry; D-Bus errors mean no Secret Service.
        match keyring::Entry::new(SERVICE, "eigendesk/probe") {
            Ok(entry) => {
                let _ = entry.get_password();
                // get_password errors with NoEntry on success-path for missing
                // items; only D-Bus connection failures are fatal.
                match entry.get_password() {
                    Err(keyring::Error::NoEntry) => Backend::SecretService,
                    Ok(_) => Backend::SecretService,
                    Err(keyring::Error::Ambiguous(_)) => Backend::SecretService,
                    Err(e) => {
                        tracing::warn!(error = %e, "Secret Service unavailable, using local file fallback");
                        Backend::LocalFile
                    }
                }
            }
            Err(_) => Backend::LocalFile,
        }
    }

    pub fn backend(&self) -> Backend {
        self.backend
    }

    pub fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": self.backend,
            "warning": match self.backend {
                Backend::SecretService => None,
                Backend::LocalFile => Some(
                    "OS keychain (Secret Service) is unavailable; secrets are stored in a \
                     restricted-permission local file. Start gnome-keyring for full protection.",
                ),
            },
        })
    }

    pub fn set(&self, plugin_id: &str, key: &str, value: &str) -> Result<(), SecretsError> {
        if !valid_key(key) {
            return Err(SecretsError::InvalidKey(key.to_string()));
        }
        let full = namespaced(plugin_id, key);
        match self.backend {
            Backend::SecretService => {
                let entry = keyring::Entry::new(SERVICE, &full)
                    .map_err(|e| SecretsError::Keychain(e.to_string()))?;
                entry
                    .set_password(value)
                    .map_err(|e| SecretsError::Keychain(e.to_string()))?;
                self.index_add(plugin_id, key);
                Ok(())
            }
            Backend::LocalFile => {
                let mut map = self.fallback.write().unwrap();
                map.insert(full, value.to_string());
                persist(&self.fallback_path.clone().unwrap(), &map)?;
                Ok(())
            }
        }
    }

    pub fn get(&self, plugin_id: &str, key: &str) -> Result<Option<String>, SecretsError> {
        if !valid_key(key) {
            return Err(SecretsError::InvalidKey(key.to_string()));
        }
        let full = namespaced(plugin_id, key);
        match self.backend {
            Backend::SecretService => {
                let entry = keyring::Entry::new(SERVICE, &full)
                    .map_err(|e| SecretsError::Keychain(e.to_string()))?;
                match entry.get_password() {
                    Ok(v) => Ok(Some(v)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(SecretsError::Keychain(e.to_string())),
                }
            }
            Backend::LocalFile => Ok(self.fallback.read().unwrap().get(&full).cloned()),
        }
    }

    pub fn delete(&self, plugin_id: &str, key: &str) -> Result<bool, SecretsError> {
        if !valid_key(key) {
            return Err(SecretsError::InvalidKey(key.to_string()));
        }
        let full = namespaced(plugin_id, key);
        match self.backend {
            Backend::SecretService => {
                let entry = keyring::Entry::new(SERVICE, &full)
                    .map_err(|e| SecretsError::Keychain(e.to_string()))?;
                match entry.delete_credential() {
                    Ok(()) => {
                        self.index_remove(plugin_id, key);
                        Ok(true)
                    }
                    Err(keyring::Error::NoEntry) => Ok(false),
                    Err(e) => Err(SecretsError::Keychain(e.to_string())),
                }
            }
            Backend::LocalFile => {
                let mut map = self.fallback.write().unwrap();
                let existed = map.remove(&full).is_some();
                if existed {
                    persist(&self.fallback_path.clone().unwrap(), &map)?;
                }
                Ok(existed)
            }
        }
    }

    /// List keys for one plugin (values are never returned in bulk).
    pub fn list(&self, plugin_id: &str) -> Vec<String> {
        let prefix = format!("{plugin_id}/");
        match self.backend {
            Backend::SecretService => {
                // The Secret Service exposes no enumeration through `keyring`;
                // a per-plugin index entry tracks known keys.
                match keyring::Entry::new(SERVICE, &format!("{plugin_id}/.index")) {
                    Ok(entry) => match entry.get_password() {
                        Ok(raw) => serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default(),
                        Err(_) => vec![],
                    },
                    Err(_) => vec![],
                }
            }
            Backend::LocalFile => self
                .fallback
                .read()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(&prefix) && !k.ends_with("/.index"))
                .map(|k| k[prefix.len()..].to_string())
                .collect(),
        }
    }

    fn read_index(&self, plugin_id: &str) -> Vec<String> {
        match keyring::Entry::new(SERVICE, &format!("{plugin_id}/.index")) {
            Ok(entry) => match entry.get_password() {
                Ok(raw) => serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default(),
                Err(_) => vec![],
            },
            Err(_) => vec![],
        }
    }

    fn index_add(&self, plugin_id: &str, key: &str) {
        let mut keys = self.read_index(plugin_id);
        if !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
            if let Ok(entry) = keyring::Entry::new(SERVICE, &format!("{plugin_id}/.index")) {
                let _ = entry.set_password(&serde_json::to_string(&keys).unwrap_or_default());
            }
        }
    }

    fn index_remove(&self, plugin_id: &str, key: &str) {
        let mut keys = self.read_index(plugin_id);
        keys.retain(|k| k != key);
        if let Ok(entry) = keyring::Entry::new(SERVICE, &format!("{plugin_id}/.index")) {
            let _ = entry.set_password(&serde_json::to_string(&keys).unwrap_or_default());
        }
    }
}

fn load_fallback(path: &PathBuf) -> Result<BTreeMap<String, String>, SecretsError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)?;
    let mut perms = meta.permissions();
    if perms.mode() & 0o077 != 0 {
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    let raw = std::fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(serde_json::from_str(&raw)?)
}

fn persist(path: &PathBuf, map: &BTreeMap<String, String>) -> Result<(), SecretsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(map)?)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ed-secrets-{}-{}", std::process::id(), std::time::SystemTime::now().elapsed().unwrap().subsec_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn owner_scoped_roundtrip() {
        let svc = SecretsService::new(tmpdir());
        svc.set("a.plugin", "api_key", "s3cret").unwrap();
        assert_eq!(svc.get("a.plugin", "api_key").unwrap(), Some("s3cret".to_string()));
        // Owner scoping: another plugin cannot read it through the service.
        assert_eq!(svc.get("b.plugin", "api_key").unwrap(), None);
        assert!(svc.delete("a.plugin", "api_key").unwrap());
        assert_eq!(svc.get("a.plugin", "api_key").unwrap(), None);
    }

    #[test]
    fn invalid_keys_rejected() {
        let svc = SecretsService::new(tmpdir());
        assert!(svc.set("a.plugin", "../evil", "x").is_err());
        assert!(svc.set("a.plugin", "", "x").is_err());
    }

    #[test]
    fn backend_status_shape() {
        let svc = SecretsService::new(tmpdir());
        let status = svc.status();
        assert!(status["backend"].is_string());
    }
}
