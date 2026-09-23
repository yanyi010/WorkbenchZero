//! Workbench Zero permission model.
//!
//! Third-party plugins start with no privileged access. Every capability the
//! kernel exposes (filesystem, network, process, notifications, ...) is gated
//! by an explicit permission grant that the user approved at install time.
//!
//! Permissions support restrictions ("scoped permissions"):
//!   - `filesystem:read` / `filesystem:write` may be scoped to directory roots
//!     (with a `${workspace}` placeholder).
//!   - `network` may be scoped to a host allow-list.
//!
//! First-party (bundled, trusted) plugins are granted their requested
//! permissions automatically at install time; user-installed plugins require
//! explicit approval, and new permissions added in an update require renewed
//! approval.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use wz_common::RwLockRecover;

#[derive(Debug, Error)]
pub enum PermissionError {
    #[error("plugin `{plugin}` is missing permission `{permission}`")]
    Missing { plugin: String, permission: String },
    #[error("path `{path}` is outside the directories granted to plugin `{plugin}`")]
    PathOutsideScope { plugin: String, path: String },
    #[error("host `{host}` is not in the network allow-list of plugin `{plugin}`")]
    HostNotAllowed { plugin: String, host: String },
    #[error("invalid permission declaration: {0}")]
    InvalidDeclaration(String),
    #[error("malformed path: {0}")]
    MalformedPath(String),
    #[error("io error while resolving path: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, PermissionError>;

/// Canonical permission names known to the kernel (v1).
pub const KNOWN_PERMISSIONS: &[&str] = &[
    "workspace:read",
    "workspace:write",
    "filesystem:read",
    "filesystem:write",
    "network",
    "process:spawn",
    "notification",
    "ai:invoke",
    "mcp:connect",
    "secrets:read",
    // Mutating secrets (set/delete) requires this stronger grant.
    "secrets:write",
    "clipboard:read",
    "clipboard:write",
    "system:open",
];

fn is_known(name: &str) -> bool {
    KNOWN_PERMISSIONS.contains(&name)
}

/// Validate a permission name appearing in a manifest.
pub fn validate_permission_name(name: &str) -> std::result::Result<(), String> {
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ':' || c == '-')
    {
        return Err(format!("permission `{name}` must be lowercase ascii"));
    }
    if !is_known(name) {
        // Unknown permissions are tolerated (future compatibility) but must
        // still be approved by the user; the kernel simply never grants a
        // capability behind them.
        tracing::warn!(permission = name, "manifest declares an unknown permission");
    }
    Ok(())
}

/// A single permission as declared by a manifest: either a bare flag
/// (`"network"`) or a scoped object (`{"filesystem:read": ["${workspace}/Papers"]}`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PermissionDeclaration {
    Flag(String),
    Scoped(HashMap<String, ScopeRestriction>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScopeRestriction {
    /// Directory roots for filesystem permissions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<Vec<String>>,
    /// Host allow-list for the network permission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
}

/// Normalized, resolved grant set for one plugin (stored raw, resolved lazily).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Grants {
    /// permission name -> restriction (empty = unrestricted within the capability)
    pub permissions: HashMap<String, ScopeRestriction>,
}

impl Grants {
    pub fn has(&self, name: &str) -> bool {
        self.permissions.contains_key(name)
    }
}

/// Parse a manifest's permission declarations into a normalized grant request.
pub fn parse_declarations(declarations: &[PermissionDeclaration]) -> Result<Grants> {
    let mut grants = Grants::default();
    for decl in declarations {
        match decl {
            PermissionDeclaration::Flag(name) => {
                validate_permission_name(name).map_err(PermissionError::InvalidDeclaration)?;
                grants
                    .permissions
                    .insert(name.clone(), ScopeRestriction::default());
            }
            PermissionDeclaration::Scoped(map) => {
                for (name, restriction) in map {
                    validate_permission_name(name).map_err(PermissionError::InvalidDeclaration)?;
                    grants.permissions.insert(name.clone(), restriction.clone());
                }
            }
        }
    }
    Ok(grants)
}

/// The permission evaluator: thread-safe map of plugin id -> granted set.
pub struct Evaluator {
    grants: std::sync::RwLock<HashMap<String, Grants>>,
}

impl Default for Evaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            grants: std::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn set_grants(&self, plugin_id: &str, grants: Grants) {
        self.grants
            .write()
            .unwrap()
            .insert(plugin_id.to_string(), grants);
    }

    pub fn remove_grants(&self, plugin_id: &str) {
        self.grants.write_or_recover().remove(plugin_id);
    }

    pub fn get_grants(&self, plugin_id: &str) -> Option<Grants> {
        self.grants.read_or_recover().get(plugin_id).cloned()
    }

    /// Check a boolean capability (`notification`, `process:spawn`, ...).
    pub fn check_flag(&self, plugin_id: &str, permission: &str) -> Result<()> {
        let has = self
            .grants
            .read()
            .unwrap()
            .get(plugin_id)
            .map(|g| g.has(permission))
            .unwrap_or(false);
        if has {
            Ok(())
        } else {
            Err(PermissionError::Missing {
                plugin: plugin_id.to_string(),
                permission: permission.to_string(),
            })
        }
    }

    /// Check a filesystem access. `workspace_root` substitutes `${workspace}`
    /// inside scoped roots and is always allowed for `workspace:read/write`.
    pub fn check_fs(
        &self,
        plugin_id: &str,
        write: bool,
        path: &Path,
        workspace_root: Option<&Path>,
    ) -> Result<PathBuf> {
        let permission = if write {
            "filesystem:write"
        } else {
            "filesystem:read"
        };
        let grants = self.grants.read_or_recover().get(plugin_id).cloned();
        let Some(grants) = grants else {
            return Err(PermissionError::Missing {
                plugin: plugin_id.to_string(),
                permission: permission.to_string(),
            });
        };

        // `workspace:read` / `workspace:write` are aliases for filesystem
        // access scoped to the workspace root.
        let mut roots: Vec<PathBuf> = Vec::new();
        let ws_perm = if write {
            "workspace:write"
        } else {
            "workspace:read"
        };
        if grants.has(ws_perm) {
            if let Some(root) = workspace_root {
                roots.push(root.to_path_buf());
            }
        }
        if grants.has(permission) {
            if let Some(restriction) = grants.permissions.get(permission) {
                if let Some(declared) = &restriction.roots {
                    for root in declared {
                        roots.push(substitute_workspace(root, workspace_root));
                    }
                } else {
                    // Unrestricted filesystem permission.
                    return resolve(path).map_err(|e| PermissionError::Io(e.to_string()));
                }
            }
        }
        if roots.is_empty() {
            return Err(PermissionError::Missing {
                plugin: plugin_id.to_string(),
                permission: permission.to_string(),
            });
        }

        // Canonicalize the requested path so that `..` segments and symlinks
        // are resolved before the prefix check.
        let canonical = resolve(path).map_err(|e| PermissionError::Io(e.to_string()))?;
        let canonical_str = canonical.to_string_lossy();
        for root in &roots {
            let root = resolve(root).map_err(|e| PermissionError::Io(e.to_string()))?;
            if canonical.starts_with(&root) {
                return Ok(canonical);
            }
        }
        Err(PermissionError::PathOutsideScope {
            plugin: plugin_id.to_string(),
            path: canonical_str.to_string(),
        })
    }

    /// Check a network access against the host allow-list.
    pub fn check_network(&self, plugin_id: &str, host: &str) -> Result<()> {
        let grants = self.grants.read_or_recover().get(plugin_id).cloned();
        let Some(grants) = grants else {
            return Err(PermissionError::Missing {
                plugin: plugin_id.to_string(),
                permission: "network".to_string(),
            });
        };
        if !grants.has("network") {
            return Err(PermissionError::Missing {
                plugin: plugin_id.to_string(),
                permission: "network".to_string(),
            });
        }
        if let Some(restriction) = grants.permissions.get("network") {
            if let Some(hosts) = &restriction.hosts {
                let allowed = hosts.iter().any(|pattern| host_matches(host, pattern));
                if !allowed {
                    return Err(PermissionError::HostNotAllowed {
                        plugin: plugin_id.to_string(),
                        host: host.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Human-readable description used by the install permission prompt.
    pub fn describe_grants(&self, plugin_id: &str) -> Vec<serde_json::Value> {
        match self.grants.read_or_recover().get(plugin_id) {
            None => vec![],
            Some(grants) => grants
                .permissions
                .iter()
                .map(|(name, restriction)| {
                    serde_json::json!({
                        "permission": name,
                        "roots": restriction.roots,
                        "hosts": restriction.hosts,
                    })
                })
                .collect(),
        }
    }
}

fn substitute_workspace(root: &str, workspace_root: Option<&Path>) -> PathBuf {
    if let Some(ws) = workspace_root {
        PathBuf::from(root.replace("${workspace}", &ws.to_string_lossy()))
    } else {
        PathBuf::from(root)
    }
}

fn host_matches(host: &str, pattern: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host == suffix || host.ends_with(&format!(".{suffix}"))
    } else {
        host == pattern
    }
}

/// Resolve a path to its canonical form without requiring the final component
/// to exist (needed for write operations). Symlinks in existing components are
/// resolved, which defeats symlink-escape attempts.
pub fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Lexical normalization first: resolve `.` and `..` without touching the
    // filesystem so the containment check stays strict and predictable.
    let mut normalized = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "path escapes the filesystem root",
                    ));
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(comp.as_os_str());
            }
        }
    }
    if normalized.exists() {
        return normalized.canonicalize();
    }
    let parent = normalized.parent().unwrap_or(Path::new("/"));
    let file = normalized.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path ends in `..`")
    })?;
    let mut canonical_parent = if parent.exists() {
        parent.canonicalize()?
    } else {
        // Deep non-existing parents: resolve the closest existing ancestor.
        let mut p = parent.to_path_buf();
        let mut tail = Vec::new();
        while !p.exists() {
            match (
                p.parent().map(|x| x.to_path_buf()),
                p.file_name().map(|x| x.to_os_string()),
            ) {
                (Some(par), Some(name)) => {
                    tail.push(name);
                    p = par;
                }
                _ => break,
            }
        }
        let mut c = p.canonicalize()?;
        for name in tail.into_iter().rev() {
            c.push(name);
        }
        c
    };
    if !file.as_encoded_bytes().iter().all(|b| *b != 0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL byte in path",
        ));
    }
    canonical_parent.push(file);
    Ok(canonical_parent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluator_with(plugin: &str, decls: &[PermissionDeclaration]) -> Evaluator {
        let ev = Evaluator::new();
        ev.set_grants(plugin, parse_declarations(decls).unwrap());
        ev
    }

    #[test]
    fn flag_grant_allows() {
        let ev = evaluator_with("a.b", &[PermissionDeclaration::Flag("notification".into())]);
        assert!(ev.check_flag("a.b", "notification").is_ok());
        assert!(ev.check_flag("a.b", "process:spawn").is_err());
    }

    #[test]
    fn no_grants_denies_everything() {
        let ev = Evaluator::new();
        assert!(ev.check_flag("x.y", "notification").is_err());
        assert!(ev.check_fs("x.y", false, Path::new("/tmp"), None).is_err());
        assert!(ev.check_network("x.y", "example.org").is_err());
    }

    #[test]
    fn scoped_fs_roots() {
        let ev = evaluator_with(
            "a.b",
            &[PermissionDeclaration::Scoped(HashMap::from([(
                "filesystem:read".to_string(),
                ScopeRestriction {
                    roots: Some(vec!["${workspace}/Papers".into()]),
                    hosts: None,
                },
            )]))],
        );
        let ws = Path::new("/tmp/ed-perm-ws");
        std::fs::create_dir_all(ws.join("Papers")).unwrap();
        std::fs::create_dir_all(ws.join("Secret")).unwrap();
        assert!(ev
            .check_fs("a.b", false, &ws.join("Papers/paper.pdf"), Some(ws))
            .is_ok());
        assert!(ev
            .check_fs("a.b", false, &ws.join("Secret/key.pem"), Some(ws))
            .is_err());
        assert!(ev
            .check_fs("a.b", true, &ws.join("Papers/paper.pdf"), Some(ws))
            .is_err());
    }

    #[test]
    fn workspace_alias_grants_root() {
        let ev = evaluator_with(
            "a.b",
            &[PermissionDeclaration::Flag("workspace:write".into())],
        );
        let ws = Path::new("/tmp/ed-perm-ws2");
        std::fs::create_dir_all(ws.join("Notes")).unwrap();
        assert!(ev
            .check_fs("a.b", true, &ws.join("Notes/a.md"), Some(ws))
            .is_ok());
        assert!(ev
            .check_fs("a.b", true, Path::new("/etc/passwd"), Some(ws))
            .is_err());
        assert!(ev
            .check_fs("a.b", false, Path::new("/etc/passwd"), Some(ws))
            .is_err());
    }

    #[test]
    fn path_traversal_is_contained() {
        let ev = evaluator_with(
            "a.b",
            &[PermissionDeclaration::Flag("workspace:read".into())],
        );
        let ws = Path::new("/tmp/ed-perm-ws3");
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        let evil = ws.join("docs/../../etc");
        assert!(ev.check_fs("a.b", false, &evil, Some(ws)).is_err());
    }

    #[test]
    fn symlink_escape_is_contained() {
        let base = std::env::temp_dir().join("ed-perm-symlink");
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("ws");
        std::fs::create_dir_all(ws.join("docs")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside"), ws.join("docs/escape")).unwrap();
        let ev = evaluator_with(
            "a.b",
            &[PermissionDeclaration::Flag("workspace:read".into())],
        );
        assert!(ev
            .check_fs("a.b", false, &ws.join("docs/escape/x"), Some(&ws))
            .is_err());
    }

    #[test]
    fn network_host_scoping() {
        let ev = evaluator_with(
            "a.b",
            &[PermissionDeclaration::Scoped(HashMap::from([(
                "network".to_string(),
                ScopeRestriction {
                    roots: None,
                    hosts: Some(vec!["api.crossref.org".into(), "*.arxiv.org".into()]),
                },
            )]))],
        );
        assert!(ev.check_network("a.b", "api.crossref.org").is_ok());
        assert!(ev.check_network("a.b", "export.arxiv.org").is_ok());
        assert!(ev.check_network("a.b", "evil.example").is_err());
        assert!(ev.check_network("a.b", "notarxiv.org").is_err());
    }

    #[test]
    fn unrestricted_network() {
        let ev = evaluator_with("a.b", &[PermissionDeclaration::Flag("network".into())]);
        assert!(ev.check_network("a.b", "anything.example").is_ok());
    }

    #[test]
    fn invalid_permission_name_rejected() {
        let decls = [PermissionDeclaration::Flag("Network!".into())];
        assert!(parse_declarations(&decls).is_err());
    }

    #[test]
    fn resolve_normalizes_parents() {
        let p = resolve(Path::new("/tmp/a/../b")).unwrap();
        assert_eq!(p, PathBuf::from("/tmp/b"));
    }
}
