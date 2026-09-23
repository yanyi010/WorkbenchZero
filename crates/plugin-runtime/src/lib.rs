//! Plugin runtime: manifests, discovery, lifecycle and the plugin manager
//! (spec §10-14, §30-41, §87-93).
//!
//! v0.1 plugin package layout:
//!
//!   my-plugin/
//!   ├── plugin.json     manifest (public API)
//!   ├── entry.html      plugin UI document (loaded in an isolated iframe)
//!   └── dist/main.js    bundled plugin code (via @eigendesk/plugin-sdk)
//!
//! Trust classes: bundled first-party plugins are `trusted` (auto-granted at
//! install); everything else is `sandboxed` and runs behind explicit
//! permission grants. The manifest `trust` field is honored only for plugins
//! loaded from the read-only bundled resources directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eigendesk_permissions::{parse_declarations, Grants, PermissionDeclaration};
use eigendesk_settings::{Scope, SettingDescriptor, SettingType};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported plugin API generation (spec §41).
pub const API_VERSION: &str = "1";

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("manifest io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("manifest parse error in {path}: {message}")]
    Parse { path: String, message: String },
    #[error("invalid plugin id `{0}` (expected `publisher.name`, lowercase)")]
    InvalidId(String),
    #[error("unsupported apiVersion `{0}` (kernel supports `{1}`)")]
    UnsupportedApi(String, String),
    #[error("plugin `{0}` is not installed")]
    NotInstalled(String),
    #[error("plugin `{0}` already exists")]
    AlreadyExists(String),
    #[error("package error: {0}")]
    Package(String),
    #[error("state error: {0}")]
    State(String),
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

fn default_entry() -> String {
    "entry.html".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: String,
    /// Honored only for bundled first-party plugins.
    #[serde(default)]
    pub trust: Option<String>,
    #[serde(default)]
    pub permissions: Vec<PermissionDeclaration>,
    #[serde(default)]
    pub activation_events: Vec<String>,
    #[serde(default)]
    pub contributes: Contributions,
    #[serde(default = "default_entry")]
    pub entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<CommandContribution>,
    #[serde(default)]
    pub views: Vec<ViewContribution>,
    #[serde(default)]
    pub widgets: Vec<WidgetContribution>,
    #[serde(default)]
    pub settings: Vec<SettingContribution>,
    #[serde(default)]
    pub search_providers: Vec<SearchProviderContribution>,
    #[serde(default)]
    pub capture_providers: Vec<CaptureProviderContribution>,
    #[serde(default)]
    pub artifact_types: Vec<ArtifactTypeContribution>,
    #[serde(default)]
    pub status_items: Vec<StatusItemContribution>,
    #[serde(default)]
    pub file_handlers: Vec<FileHandlerContribution>,
    #[serde(default)]
    pub ai_tools: Vec<AiToolContribution>,
    #[serde(default)]
    pub services: Vec<ServiceContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybinding: Option<String>,
    #[serde(default)]
    pub takes_args: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Declarative behavior: opening the named view of the owning plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_view: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ViewLocation {
    Main,
    Sidebar,
    Bottom,
    Floating,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default = "view_main")]
    pub location: ViewLocation,
}

fn view_main() -> ViewLocation {
    ViewLocation::Main
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetContribution {
    pub id: String,
    pub title: String,
    pub min_width: u32,
    pub min_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingContribution {
    pub key: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub enum_values: Vec<String>,
    /// `global` or `workspace` (default global).
    #[serde(default = "default_global")]
    pub scope: String,
}

fn default_global() -> String {
    "global".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchProviderContribution {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProviderContribution {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub prefixes: Vec<String>,
    /// Lower = weaker claim. Default providers use high numbers.
    #[serde(default = "priority_default")]
    pub priority: i32,
    /// Command invoked with the captured text when this provider wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

fn priority_default() -> i32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTypeContribution {
    #[serde(rename = "type")]
    pub r#type: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatusItemContribution {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileHandlerContribution {
    pub id: String,
    /// MIME type or `*/*`.
    pub mime_type: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiToolContribution {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    /// High-risk tools require explicit confirmation (spec §54).
    #[serde(default)]
    pub high_risk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServiceContribution {
    pub id: String,
    pub api: String,
}

pub fn valid_plugin_id(id: &str) -> bool {
    let Some((publisher, name)) = id.split_once('.') else {
        return false;
    };
    let valid_part = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.chars().next().unwrap().is_ascii_lowercase()
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    valid_part(publisher) && valid_part(name)
}

/// Parse and validate a manifest from raw JSON text.
pub fn parse_manifest(raw: &str, path: &Path) -> Result<PluginManifest, PluginError> {
    let manifest: PluginManifest = serde_json::from_str(raw).map_err(|e| PluginError::Parse {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    validate_manifest(&manifest, path)?;
    Ok(manifest)
}

pub fn validate_manifest(m: &PluginManifest, path: &Path) -> Result<(), PluginError> {
    if !valid_plugin_id(&m.id) {
        return Err(PluginError::InvalidId(m.id.clone()));
    }
    if m.name.trim().is_empty() {
        return Err(PluginError::Parse { path: path.display().to_string(), message: "name is required".into() });
    }
    if m.api_version != API_VERSION {
        return Err(PluginError::UnsupportedApi(m.api_version.clone(), API_VERSION.to_string()));
    }
    // Basic semver check.
    let parts: Vec<&str> = m.version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u64>().is_err()) {
        return Err(PluginError::Parse {
            path: path.display().to_string(),
            message: format!("version `{}` is not semver", m.version),
        });
    }
    // Contribution id sanity: ids must be non-empty and free of whitespace.
    for cmd in &m.contributes.commands {
        if cmd.id.trim().is_empty() || cmd.id.contains(char::is_whitespace) {
            return Err(PluginError::Parse {
                path: path.display().to_string(),
                message: format!("invalid command id `{}`", cmd.id),
            });
        }
    }
    for view in &m.contributes.views {
        if view.id.trim().is_empty() {
            return Err(PluginError::Parse {
                path: path.display().to_string(),
                message: format!("invalid view id `{}`", view.id),
            });
        }
    }
    if let Some(trust) = &m.trust {
        if trust != "trusted" && trust != "sandboxed" {
            return Err(PluginError::Parse {
                path: path.display().to_string(),
                message: format!("trust must be `trusted` or `sandboxed`, got `{trust}`"),
            });
        }
    }
    Ok(())
}

impl PluginManifest {
    pub fn requested_grants(&self) -> Result<Grants, PluginError> {
        parse_declarations(&self.permissions).map_err(|e| PluginError::Parse {
            path: self.id.clone(),
            message: e.to_string(),
        })
    }

    pub fn setting_descriptors(&self) -> Vec<SettingDescriptor> {
        self.contributes
            .settings
            .iter()
            .map(|s| SettingDescriptor {
                key: format!("{}.{}", self.id, s.key),
                r#type: match s.r#type.as_str() {
                    "boolean" => SettingType::Boolean,
                    "number" => SettingType::Number,
                    "enum" => SettingType::Enum,
                    _ => SettingType::String,
                },
                title: s.title.clone(),
                description: s.description.clone(),
                default: s.default.clone(),
                enum_values: s.enum_values.clone(),
                scope: if s.scope == "workspace" { Scope::Workspace } else { Scope::Global },
                plugin_id: Some(self.id.clone()),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginState {
    /// Present on disk but never installed.
    Discovered,
    /// Installed but awaiting permission approval.
    Installed,
    Disabled,
    Enabled,
    Uninstalled,
    /// Runtime-only states (not persisted).
    Activating,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginSource {
    Bundled,
    User,
    Dev,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub manifest: PluginManifest,
    pub source: PluginSource,
    pub state: PluginState,
    #[serde(default)]
    pub pinned: bool,
    /// Granted permission set (raw, with `${workspace}` placeholders intact).
    #[serde(default)]
    pub granted: Grants,
    pub install_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
    #[serde(default)]
    pub failure_count: u32,
    /// Set when an update introduced new permissions needing re-approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_permissions: Option<Grants>,
    #[serde(default)]
    pub trusted: bool,
}

impl PluginRecord {
    pub fn is_installed(&self) -> bool {
        !matches!(self.state, PluginState::Discovered | PluginState::Uninstalled)
    }

    /// Activation events this plugin subscribes to. Supports exact matches
    /// and trailing `*` wildcards (`onCommand:memo.*`).
    pub fn matches_activation(&self, event: &str) -> bool {
        self.manifest.activation_events.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix('*') {
                event.starts_with(prefix)
            } else {
                pattern == event
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Catalog & packs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Local path (bundled resources or registry dir) to the package folder.
    pub package_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPack {
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugins: Vec<String>,
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

pub struct PluginManager {
    records: std::sync::RwLock<HashMap<String, PluginRecord>>,
    state_path: PathBuf,
    bundled_dir: Option<PathBuf>,
    user_dir: PathBuf,
    dev_dir: PathBuf,
    registry_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub manifest: PluginManifest,
    pub source: PluginSource,
    pub state: PluginState,
    pub pinned: bool,
    pub trusted: bool,
    pub install_path: String,
    pub failure_count: u32,
    pub pending_permissions: Option<Grants>,
    pub requested: Grants,
}

impl From<&PluginRecord> for PluginInfo {
    fn from(r: &PluginRecord) -> Self {
        PluginInfo {
            manifest: r.manifest.clone(),
            source: r.source,
            state: r.state,
            pinned: r.pinned,
            trusted: r.trusted,
            install_path: r.install_path.display().to_string(),
            failure_count: r.failure_count,
            pending_permissions: r.pending_permissions.clone(),
            requested: r.manifest.requested_grants().unwrap_or_default(),
        }
    }
}

impl PluginManager {
    pub fn new(
        state_path: PathBuf,
        bundled_dir: Option<PathBuf>,
        user_dir: PathBuf,
        dev_dir: PathBuf,
        registry_dir: PathBuf,
    ) -> Self {
        Self { records: std::sync::RwLock::new(HashMap::new()), state_path, bundled_dir, user_dir, dev_dir, registry_dir }
    }

    // -- persistence --------------------------------------------------------

    fn load_state(&self) -> HashMap<String, PluginRecord> {
        if !self.state_path.exists() {
            return HashMap::new();
        }
        match std::fs::read_to_string(&self.state_path) {
            Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "plugin state file corrupt, starting fresh");
                HashMap::new()
            }),
            _ => HashMap::new(),
        }
    }

    fn save_state(&self, records: &HashMap<String, PluginRecord>) -> Result<(), PluginError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.state_path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(records)?)?;
        std::fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    // -- discovery ----------------------------------------------------------

    /// Discover plugins from bundled / user / dev directories and merge with
    /// persisted install state. Called at boot and whenever directories change.
    pub fn discover(&self) -> Result<Vec<String>, PluginError> {
        let persisted = self.load_state();
        let mut records = persisted;
        let mut changed = Vec::new();

        let mut scan = |dir: &Path, source: PluginSource, trusted_source: bool| -> Result<(), PluginError> {
            if !dir.is_dir() {
                return Ok(());
            }
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                if !entry.path().is_dir() {
                    continue;
                }
                let manifest_path = entry.path().join("plugin.json");
                if !manifest_path.exists() {
                    continue;
                }
                let raw = std::fs::read_to_string(&manifest_path)?;
                let manifest = match parse_manifest(&raw, &manifest_path) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(dir = %entry.path().display(), error = %e, "skipping plugin with invalid manifest");
                        continue;
                    }
                };
                let trusted = trusted_source && manifest.trust.as_deref() == Some("trusted");
                match records.get_mut(&manifest.id) {
                    Some(existing) => {
                        existing.manifest = manifest;
                        existing.install_path = entry.path();
                        existing.source = source;
                        existing.trusted = trusted;
                    }
                    None => {
                        records.insert(
                            manifest.id.clone(),
                            PluginRecord {
                                manifest,
                                source,
                                state: PluginState::Discovered,
                                pinned: false,
                                granted: Grants::default(),
                                install_path: entry.path(),
                                installed_at: None,
                                failure_count: 0,
                                pending_permissions: None,
                                trusted,
                            },
                        );
                    }
                }
                changed.push(entry.path().display().to_string());
            }
            Ok(())
        };

        if let Some(bundled) = &self.bundled_dir {
            scan(bundled, PluginSource::Bundled, true)?;
        }
        scan(&self.dev_dir, PluginSource::Dev, false)?;
        scan(&self.user_dir, PluginSource::User, false)?;

        // Persist merged view (drop records whose directories vanished).
        let user_dir = self.user_dir.clone();
        let dev_dir = self.dev_dir.clone();
        let bundled_dir = self.bundled_dir.clone();
        records.retain(|_, rec| {
            if !rec.install_path.is_dir() {
                return false;
            }
            let under_user = rec.install_path.starts_with(&user_dir);
            let under_dev = rec.install_path.starts_with(&dev_dir);
            let under_bundled = bundled_dir.as_ref().map(|b| rec.install_path.starts_with(b)).unwrap_or(false);
            under_user || under_dev || under_bundled
        });

        // Dev plugins are auto-enabled; sandboxed user plugins follow state.
        for rec in records.values_mut() {
            if rec.source == PluginSource::Dev && rec.state == PluginState::Discovered {
                rec.state = PluginState::Enabled;
                rec.granted = rec.manifest.requested_grants().unwrap_or_default();
                rec.installed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }

        self.save_state(&records)?;
        *self.records.write().unwrap() = records;
        Ok(changed)
    }

    pub fn list(&self) -> Vec<PluginInfo> {
        let records = self.records.read().unwrap();
        let mut infos: Vec<PluginInfo> = records.values().map(PluginInfo::from).collect();
        infos.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        infos
    }

    pub fn get(&self, id: &str) -> Option<PluginRecord> {
        self.records.read().unwrap().get(id).cloned()
    }

    /// Plugins that should activate for a given activation event.
    pub fn activation_candidates(&self, event: &str) -> Vec<PluginRecord> {
        self.records
            .read()
            .unwrap()
            .values()
            .filter(|r| {
                matches!(
                    r.state,
                    PluginState::Enabled | PluginState::Activating | PluginState::Active
                ) && r.matches_activation(event)
            })
            .cloned()
            .collect()
    }

    pub fn set_state(&self, id: &str, state: PluginState) -> Result<(), PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        rec.state = state;
        if state == PluginState::Active || state == PluginState::Activating {
            rec.failure_count = 0;
        }
        self.save_state(&records)
    }

    pub fn record_failure(&self, id: &str) -> Result<u32, PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        rec.failure_count += 1;
        let count = rec.failure_count;
        self.save_state(&records)?;
        Ok(count)
    }

    pub fn set_pending_permissions(&self, id: &str, pending: Option<Grants>) -> Result<(), PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        rec.pending_permissions = pending;
        self.save_state(&records)
    }

    // -- install / uninstall ------------------------------------------------

    /// Install a plugin from a catalog entry (local package path) or from an
    /// arbitrary directory / `.edplugin.zip` on disk.
    pub fn install_from(&self, source: &str) -> Result<PluginInfo, PluginError> {
        let path = PathBuf::from(source);
        let staging: PathBuf;
        let src_dir: PathBuf = if path.is_dir() {
            path
        } else if path.extension().map(|e| e == "zip").unwrap_or(false) || path.to_string_lossy().ends_with(".edplugin.zip") {
            staging = self.extract_zip(&path)?;
            staging
        } else {
            return Err(PluginError::Package(format!("`{source}` is neither a plugin directory nor a .edplugin.zip")));
        };

        let manifest_path = src_dir.join("plugin.json");
        if !manifest_path.exists() {
            return Err(PluginError::Package(format!("`{source}` has no plugin.json")));
        }
        let raw = std::fs::read_to_string(&manifest_path)?;
        let manifest = parse_manifest(&raw, &manifest_path)?;

        let target = self.user_dir.join(format!(
            "{}-{}",
            manifest.id.replace('.', "-"),
            manifest.version
        ));
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| PluginError::Package(e.to_string()))?;
        }
        copy_dir(&src_dir, &target).map_err(|e| PluginError::Package(e.to_string()))?;

        let mut records = self.records.write().unwrap();
        // Installing over an existing plugin = update; keep grants, detect
        // permission drift (spec §33: new permissions need renewed approval).
        let existing = records.get(&manifest.id).cloned();
        let requested = manifest.requested_grants()?;
        let (state, granted, pending) = match existing {
            Some(prev) if prev.is_installed() => {
                let drifted = permission_drift(&prev.granted, &requested);
                if drifted.is_empty() {
                    (PluginState::Disabled, prev.granted.clone(), None)
                } else {
                    // Disable until re-approval, keep old grants for the diff UI.
                    (PluginState::Disabled, prev.granted.clone(), Some(requested))
                }
            }
            _ => (PluginState::Installed, Grants::default(), None),
        };
        let record = PluginRecord {
            trusted: false,
            manifest,
            source: PluginSource::User,
            state,
            pinned: false,
            granted,
            install_path: target,
            installed_at: Some(chrono::Utc::now().to_rfc3339()),
            failure_count: 0,
            pending_permissions: pending,
        };
        let info = PluginInfo::from(&record);
        records.insert(info.manifest.id.clone(), record);
        self.save_state(&records)?;
        Ok(info)
    }

    fn extract_zip(&self, zip_path: &Path) -> Result<PathBuf, PluginError> {
        let staging = self.user_dir.join(".staging").join(format!(
            "{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_subsec_nanos()
        ));
        let file = std::fs::File::open(zip_path).map_err(|e| PluginError::Package(e.to_string()))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| PluginError::Package(e.to_string()))?;
        archive
            .extract(&staging)
            .map_err(|e| PluginError::Package(format!("zip extract failed: {e}")))?;
        // Accept either layout: <staging>/plugin.json or <staging>/<dir>/plugin.json
        if staging.join("plugin.json").exists() {
            Ok(staging)
        } else {
            for entry in std::fs::read_dir(&staging).map_err(|e| PluginError::Package(e.to_string()))? {
                let entry = entry.map_err(|e| PluginError::Package(e.to_string()))?;
                if entry.path().join("plugin.json").exists() {
                    return Ok(entry.path());
                }
            }
            Err(PluginError::Package("zip does not contain a plugin.json".into()))
        }
    }

    /// Approve the requested permission set and enable.
    pub fn approve_permissions(&self, id: &str, approve: bool) -> Result<PluginInfo, PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        if approve {
            let requested = if let Some(pending) = &rec.pending_permissions {
                pending.clone()
            } else {
                rec.manifest.requested_grants()?
            };
            rec.granted = requested;
            rec.pending_permissions = None;
            rec.state = PluginState::Enabled;
        } else {
            rec.state = PluginState::Disabled;
        }
        let info = PluginInfo::from(&*rec);
        self.save_state(&records)?;
        Ok(info)
    }

    /// Trusted (bundled) plugins can be installed with auto-granted permissions.
    pub fn install_trusted(&self, id: &str) -> Result<PluginInfo, PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        if !rec.trusted {
            return Err(PluginError::State(format!("plugin {id} is not a trusted bundled plugin")));
        }
        rec.granted = rec.manifest.requested_grants()?;
        rec.state = PluginState::Enabled;
        rec.installed_at = Some(chrono::Utc::now().to_rfc3339());
        let info = PluginInfo::from(&*rec);
        self.save_state(&records)?;
        Ok(info)
    }

    pub fn enable(&self, id: &str) -> Result<PluginInfo, PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        if rec.state == PluginState::Installed || rec.pending_permissions.is_some() {
            return Err(PluginError::State(format!(
                "plugin {id} requires permission approval before enabling"
            )));
        }
        rec.state = PluginState::Enabled;
        let info = PluginInfo::from(&*rec);
        self.save_state(&records)?;
        Ok(info)
    }

    pub fn disable(&self, id: &str) -> Result<PluginInfo, PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        rec.state = PluginState::Disabled;
        let info = PluginInfo::from(&*rec);
        self.save_state(&records)?;
        Ok(info)
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        rec.pinned = pinned;
        self.save_state(&records)
    }

    pub fn uninstall(&self, id: &str) -> Result<(), PluginError> {
        let mut records = self.records.write().unwrap();
        let rec = records.get_mut(id).ok_or_else(|| PluginError::NotInstalled(id.to_string()))?;
        if rec.trusted && rec.source == PluginSource::Bundled {
            // Bundled plugins are mark-uninstalled, not deleted (spec §93).
            rec.state = PluginState::Uninstalled;
            rec.granted = Grants::default();
        } else {
            let path = rec.install_path.clone();
            records.remove(id);
            if path.starts_with(&self.user_dir) && path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|e| PluginError::Package(e.to_string()))?;
            }
        }
        self.save_state(&records)
    }

    // -- catalog ------------------------------------------------------------

    pub fn load_catalog(&self) -> Vec<CatalogEntry> {
        let index = self.registry_dir.join("index.json");
        if !index.exists() {
            return vec![];
        }
        std::fs::read_to_string(&index)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn load_packs(&self) -> Vec<ExtensionPack> {
        let packs = self.registry_dir.join("packs.json");
        if !packs.exists() {
            return vec![];
        }
        std::fs::read_to_string(&packs)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}

/// Compute which requested permissions are not covered by current grants.
pub fn permission_drift(current: &Grants, requested: &Grants) -> Vec<String> {
    let mut drift = Vec::new();
    for (name, restriction) in &requested.permissions {
        match current.permissions.get(name) {
            None => drift.push(name.clone()),
            Some(current_restriction) => {
                // A permission that gains scope entries also needs re-approval
                // (e.g. new filesystem roots / hosts).
                if let Some(roots) = &restriction.roots {
                    if let Some(current_roots) = &current_restriction.roots {
                        if roots.iter().any(|r| !current_roots.contains(r)) {
                            drift.push(name.clone());
                        }
                    }
                }
                if let Some(hosts) = &restriction.hosts {
                    if let Some(current_hosts) = &current_restriction.hosts {
                        if hosts.iter().any(|h| !current_hosts.contains(h)) {
                            drift.push(name.clone());
                        }
                    }
                }
            }
        }
    }
    drift
}

pub fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_symlink() {
            // Refuse symlinks inside plugin packages (supply-chain hygiene).
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("symlink in plugin package: {}", entry.path().display()),
            ));
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let data = std::fs::read(path)?;
    hasher.update(&data);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(dir: &Path, id: &str, version: &str, permissions: &str) -> PathBuf {
        let pdir = dir.join(id.replace('.', "-"));
        std::fs::create_dir_all(&pdir).unwrap();
        let manifest = format!(
            r#"{{
              "id": "{id}",
              "name": "Test {id}",
              "version": "{version}",
              "apiVersion": "1",
              "publisher": "test",
              "permissions": {permissions},
              "activationEvents": ["onCommand:{id}.*"],
              "contributes": {{
                "commands": [{{"id": "{id}.hello", "title": "Hello"}}]
              }}
            }}"#
        );
        std::fs::write(pdir.join("plugin.json"), manifest).unwrap();
        std::fs::write(pdir.join("entry.html"), "<html><body></body></html>").unwrap();
        pdir
    }

    fn manager(base: &Path) -> PluginManager {
        let user = base.join("user-plugins");
        let dev = base.join("dev-plugins");
        let reg = base.join("registry");
        for d in [&user, &dev, &reg] {
            std::fs::create_dir_all(d).unwrap();
        }
        PluginManager::new(base.join("plugins-state.json"), None, user, dev, reg)
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ed-pr-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().elapsed().unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plugin_id_format() {
        assert!(valid_plugin_id("workbench.memo"));
        assert!(valid_plugin_id("yan-yi.slurm-monitor"));
        assert!(!valid_plugin_id("no-dot"));
        assert!(!valid_plugin_id("UPPER.lower"));
        assert!(!valid_plugin_id("a.b.c"));
        assert!(!valid_plugin_id(".b"));
        assert!(!valid_plugin_id("a."));
    }

    #[test]
    fn manifest_parse_and_api_version() {
        let raw = r#"{"id":"a.b","name":"A","version":"1.0.0","apiVersion":"1"}"#;
        let m = parse_manifest(raw, Path::new("x")).unwrap();
        assert_eq!(m.entry, "entry.html");

        let raw2 = r#"{"id":"a.b","name":"A","version":"1.0.0","apiVersion":"2"}"#;
        assert!(matches!(
            parse_manifest(raw2, Path::new("x")),
            Err(PluginError::UnsupportedApi(_, _))
        ));
    }

    #[test]
    fn malformed_manifest_rejected() {
        for raw in [
            r#"{"id":"nodot","name":"A","version":"1.0.0","apiVersion":"1"}"#,
            r#"{"id":"a.b","name":"","version":"1.0.0","apiVersion":"1"}"#,
            r#"{"id":"a.b","name":"A","version":"1.0","apiVersion":"1"}"#,
            r#"{"id":"a.b","name":"A","version":"1.0.0","apiVersion":"1","contributes":{"commands":[{"id":"  ","title":"x"}]}}"#,
        ] {
            assert!(parse_manifest(raw, Path::new("x")).is_err(), "should reject: {raw}");
        }
    }

    #[test]
    fn discovery_and_lifecycle() {
        let base = temp();
        let user_dir = base.join("user-plugins");
        write_plugin(&user_dir, "test.alpha", "1.0.0", r#"["notification"]"#);
        let mgr = manager(&base);
        mgr.discover().unwrap();
        let info = mgr.get("test.alpha").unwrap();
        assert_eq!(info.state, PluginState::Discovered);

        // Install from the user dir path itself (same dir; simulate install of external path).
        let info = mgr.install_from(user_dir.join("test-alpha").to_str().unwrap()).unwrap();
        assert_eq!(info.state, PluginState::Installed);
        assert!(mgr.get("test.alpha").unwrap().install_path.starts_with(&base.join("user-plugins")));

        // Enabling before approval fails.
        assert!(mgr.enable("test.alpha").is_err());
        let info = mgr.approve_permissions("test.alpha", true).unwrap();
        assert_eq!(info.state, PluginState::Enabled);
        assert!(info.requested.has("notification"));

        mgr.set_state("test.alpha", PluginState::Active).unwrap();
        let candidates = mgr.activation_candidates("onCommand:test.alpha.hello");
        assert_eq!(candidates.len(), 1);

        mgr.disable("test.alpha").unwrap();
        assert_eq!(mgr.activation_candidates("onCommand:test.alpha.hello").len(), 0);
        mgr.uninstall("test.alpha").unwrap();
        assert!(mgr.get("test.alpha").is_none());
    }

    #[test]
    fn activation_wildcards() {
        let raw = r#"{
            "id":"w.c","name":"W","version":"0.1.0","apiVersion":"1",
            "activationEvents":["onCommand:w.*","onView:w.main","onSearch"]
        }"#;
        let m = parse_manifest(raw, Path::new("x")).unwrap();
        let rec = PluginRecord {
            manifest: m,
            source: PluginSource::User,
            state: PluginState::Enabled,
            pinned: false,
            granted: Grants::default(),
            install_path: PathBuf::new(),
            installed_at: None,
            failure_count: 0,
            pending_permissions: None,
            trusted: false,
        };
        assert!(rec.matches_activation("onCommand:w.new"));
        assert!(rec.matches_activation("onView:w.main"));
        assert!(rec.matches_activation("onSearch"));
        assert!(!rec.matches_activation("onCommand:x.y"));
    }

    #[test]
    fn update_permission_drift_requires_reapproval() {
        let base = temp();
        let user_dir = base.join("user-plugins");
        std::fs::create_dir_all(&user_dir).unwrap();
        write_plugin(&user_dir, "test.beta", "1.0.0", r#"["notification"]"#);
        let mgr = manager(&base);
        mgr.discover().unwrap();
        mgr.install_from(user_dir.join("test-beta").to_str().unwrap()).unwrap();
        mgr.approve_permissions("test.beta", true).unwrap();

        // Simulate an update that adds a new permission.
        write_plugin(&user_dir, "test.beta", "1.1.0", r#"["notification","network"]"#);
        let info = mgr.install_from(user_dir.join("test-beta").to_str().unwrap()).unwrap();
        assert!(info.pending_permissions.is_some(), "new permission must require re-approval");
        assert!(mgr.enable("test.beta").is_err());

        let info = mgr.approve_permissions("test.beta", true).unwrap();
        assert_eq!(info.state, PluginState::Enabled);
        assert!(info.requested.has("network"));
    }

    #[test]
    fn drift_detection() {
        let current = Grants { permissions: HashMap::from([("network".to_string(), Default::default())]) };
        let same = Grants { permissions: HashMap::from([("network".to_string(), Default::default())]) };
        assert!(permission_drift(&current, &same).is_empty());
        let extra = Grants {
            permissions: HashMap::from([
                ("network".to_string(), Default::default()),
                ("process:spawn".to_string(), Default::default()),
            ]),
        };
        assert_eq!(permission_drift(&current, &extra), vec!["process:spawn"]);
    }
}
