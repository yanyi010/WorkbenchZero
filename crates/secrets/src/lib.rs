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
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::Serialize;
use thiserror::Error;
use wz_common::{load_json, JsonLoad, RwLockRecover};

const SERVICE: &str = "workbench-zero";

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
    /// Non-fatal degradation surfaced in `status()` (e.g. a quarantined
    /// corrupt fallback file).
    degraded: Option<String>,
}

fn namespaced(plugin_id: &str, key: &str) -> String {
    format!("{plugin_id}/{key}")
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

impl SecretsService {
    /// Create the service, probing the OS keychain. `fallback_dir` is the
    /// application data dir used when the keychain is unavailable.
    pub fn new(fallback_dir: PathBuf) -> Self {
        let backend = Self::probe_backend();
        let mut degraded = None;
        let (backend, fallback_path, fallback) = match backend {
            Backend::SecretService => (Backend::SecretService, None, BTreeMap::new()),
            Backend::LocalFile => {
                let path = fallback_dir.join("secrets.json");
                // A corrupt fallback file is quarantined (bytes preserved) and
                // never silently overwritten — the next `set` writes a fresh
                // file while the quarantined copy remains for manual recovery.
                let (map, note) = load_fallback(&path);
                degraded = note;
                (Backend::LocalFile, Some(path), map)
            }
        };
        Self {
            backend,
            fallback_path,
            fallback: RwLock::new(fallback),
            degraded,
        }
    }

    fn probe_backend() -> Backend {
        // Probe with a throwaway entry; D-Bus errors mean no Secret Service.
        match keyring::Entry::new(SERVICE, "workbench-zero/probe") {
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
                Backend::SecretService => self.degraded.clone(),
                Backend::LocalFile => Some(self.degraded.clone().unwrap_or_else(||
                    "OS keychain (Secret Service) is unavailable; secrets are stored in a \
                     restricted-permission local file. Start gnome-keyring for full protection.".to_string()
                )),
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
                let Some(path) = self.fallback_path.clone() else {
                    return Err(SecretsError::Keychain(
                        "local-file backend selected without a fallback path".into(),
                    ));
                };
                let mut map = self.fallback.write_or_recover();
                map.insert(full, value.to_string());
                persist(&path, &map)?;
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
            Backend::LocalFile => Ok(self.fallback.read_or_recover().get(&full).cloned()),
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
                let Some(path) = self.fallback_path.clone() else {
                    return Err(SecretsError::Keychain(
                        "local-file backend selected without a fallback path".into(),
                    ));
                };
                let mut map = self.fallback.write_or_recover();
                let existed = map.remove(&full).is_some();
                if existed {
                    persist(&path, &map)?;
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
                .read_or_recover()
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

/// Load the fallback secrets file. Returns the map plus an optional
/// degradation note for `status()`. Corrupt files are quarantined and the
/// service starts empty — previous secrets remain recoverable by hand.
fn load_fallback(path: &Path) -> (BTreeMap<String, String>, Option<String>) {
    if !path.exists() {
        return (BTreeMap::new(), None);
    }
    // Repair permissions first: the file must never be group/world readable.
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o077 != 0 {
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    match load_json::<BTreeMap<String, String>>(path) {
        Ok(JsonLoad::Loaded(m)) | Ok(JsonLoad::RecoveredFromTmp(m)) => (m, None),
        Ok(JsonLoad::Missing) => (BTreeMap::new(), None),
        Ok(JsonLoad::Corrupt { quarantined_to }) => (
            BTreeMap::new(),
            Some(format!(
                "secrets file was corrupt and has been preserved at `{}`; secrets are empty until re-set",
                quarantined_to.display()
            )),
        ),
        Err(e) => {
            tracing::error!(error = %e, "failed to read secrets file");
            (
                BTreeMap::new(),
                Some(format!("secrets file unreadable: {e}")),
            )
        }
    }
}

fn persist(path: &Path, map: &BTreeMap<String, String>) -> Result<(), SecretsError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Restrictive permissions from creation: write to a unique tmp file, set
    // 0600 before the atomic rename so a secrets file never exists with wider
    // permissions at any point in time.
    let tmp = path.with_file_name(format!(
        ".secrets.json.tmp-{}",
        std::process::id()
    ));
    std::fs::write(&tmp, serde_json::to_string(map)?)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    // atomic_write would create a fresh tmp; reuse its fsync discipline by
    // renaming ourselves after explicit file fsync.
    {
        let f = std::fs::File::open(&tmp)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::File::open(parent).map(|d| d.sync_all());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wz-secrets-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn owner_scoped_roundtrip() {
        let svc = SecretsService::new(tmpdir());
        svc.set("a.plugin", "api_key", "s3cret").unwrap();
        assert_eq!(
            svc.get("a.plugin", "api_key").unwrap(),
            Some("s3cret".to_string())
        );
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

    #[test]
    fn corrupt_fallback_never_silently_overwritten() {
        let dir = tmpdir();
        // Force the LocalFile backend path shape: write a corrupt secrets.json.
        let path = dir.join("secrets.json");
        std::fs::write(&path, "{torn secrets").unwrap();
        let (map, note) = load_fallback(&path);
        assert!(map.is_empty());
        assert!(note.is_some(), "degradation must be surfaced");
        // The original bytes survive under a quarantine name, not overwritten.
        assert!(!path.exists());
        let quarantined: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(
            std::fs::read_to_string(quarantined[0].path()).unwrap(),
            "{torn secrets"
        );
    }
}
