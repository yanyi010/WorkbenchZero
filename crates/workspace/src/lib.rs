//! Workspace manager (spec §8-9).
//!
//! A workspace is the principal user context and MAY map to a filesystem
//! directory. State lives under `<root>/.workbench/`:
//!
//!   workspace.json   canonical workspace metadata (schema versioned)
//!   settings.json    workspace-scoped settings
//!   layout.json      serialized layout (owned by the shell)
//!   index.sqlite     derived: FTS index, artifacts, recents
//!   plugin-state/    per-plugin canonical state
//!   cache/           disposable
//!
//! The global workspace registry (`~/.config/eigendesk/workspaces.json`) lists
//! known workspaces and recency. Schema changes use explicit migrations with
//! backup before destructive steps (spec §69).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current workspace format schema version (spec §101: explicit, independent).
pub const WORKSPACE_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace root `{0}` does not exist")]
    RootMissing(PathBuf),
    #[error("`{0}` exists but is not a directory")]
    NotADirectory(PathBuf),
    #[error("workspace metadata is corrupt: {0}")]
    Corrupt(String),
    #[error("workspace schema version {0} is newer than supported {1}")]
    NewerSchema(i64, i64),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("registry io error: {0}")]
    Registry(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub created_at: String,
    pub last_opened_at: String,
}

/// On-disk `workspace.json` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMeta {
    pub schema_version: i64,
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// An open workspace with resolved paths.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub record: WorkspaceRecord,
    pub workbench_dir: PathBuf,
    pub state_root: PathBuf,
    pub cache_dir: PathBuf,
    pub settings_path: PathBuf,
    pub layout_path: PathBuf,
    pub sqlite_path: PathBuf,
}

impl Workspace {
    pub fn root(&self) -> &Path {
        &self.record.root
    }

    pub fn load_layout(&self) -> serde_json::Value {
        read_json(&self.layout_path).unwrap_or(serde_json::Value::Null)
    }

    pub fn save_layout(&self, layout: &serde_json::Value) -> Result<(), WorkspaceError> {
        write_json(&self.layout_path, layout)
    }
}

pub struct WorkspaceManager {
    registry_path: PathBuf,
    records: Vec<WorkspaceRecord>,
}

impl WorkspaceManager {
    pub fn new(registry_path: PathBuf) -> Result<Self, WorkspaceError> {
        let records = if registry_path.exists() {
            let raw = std::fs::read_to_string(&registry_path)?;
            if raw.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&raw)?
            }
        } else {
            Vec::new()
        };
        Ok(Self { registry_path, records })
    }

    fn save_registry(&self) -> Result<(), WorkspaceError> {
        if let Some(parent) = self.registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.registry_path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.records)?)?;
        std::fs::rename(&tmp, &self.registry_path)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<WorkspaceRecord> {
        let mut records = self.records.clone();
        records.sort_by(|a, b| b.last_opened_at.cmp(&a.last_opened_at));
        records
    }

    pub fn most_recent(&self) -> Option<WorkspaceRecord> {
        self.list().into_iter().next()
    }

    pub fn get(&self, id: &str) -> Option<WorkspaceRecord> {
        self.records.iter().find(|r| r.id == id).cloned()
    }

    /// Create a new workspace rooted at `root` (must exist or `create_root`
    /// must be true). Returns the opened workspace.
    pub fn create(&mut self, name: &str, root: PathBuf, create_root: bool) -> Result<Workspace, WorkspaceError> {
        if create_root && !root.exists() {
            std::fs::create_dir_all(&root)?;
        }
        if !root.exists() {
            return Err(WorkspaceError::RootMissing(root));
        }
        if !root.is_dir() {
            return Err(WorkspaceError::NotADirectory(root));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        let record = WorkspaceRecord {
            id: id.clone(),
            name: name.to_string(),
            root: root.clone(),
            created_at: now.clone(),
            last_opened_at: now,
        };
        let workbench = root.join(".workbench");
        std::fs::create_dir_all(workbench.join("plugin-state"))?;
        std::fs::create_dir_all(workbench.join("cache"))?;
        let meta = WorkspaceMeta {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            id,
            name: name.to_string(),
            created_at: record.created_at.clone(),
        };
        write_json(&workbench.join("workspace.json"), &serde_json::to_value(&meta)?)?;
        self.records.retain(|r| r.root != record.root);
        self.records.push(record.clone());
        self.save_registry()?;
        Ok(self.materialize(record)?)
    }

    /// Register an existing workspace directory (contains `.workbench/`).
    pub fn register_existing(&mut self, root: PathBuf) -> Result<Workspace, WorkspaceError> {
        if !root.is_dir() {
            return Err(WorkspaceError::NotADirectory(root));
        }
        let workbench = root.join(".workbench");
        let meta_path = workbench.join("workspace.json");
        if !meta_path.exists() {
            return Err(WorkspaceError::Corrupt(format!(
                "`{}` is not a workspace (missing .workbench/workspace.json)",
                root.display()
            )));
        }
        let meta: WorkspaceMeta = serde_json::from_str(&std::fs::read_to_string(&meta_path)?)?;
        if meta.schema_version > WORKSPACE_SCHEMA_VERSION {
            return Err(WorkspaceError::NewerSchema(meta.schema_version, WORKSPACE_SCHEMA_VERSION));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let record = WorkspaceRecord {
            id: meta.id,
            name: meta.name,
            root: root.clone(),
            created_at: meta.created_at,
            last_opened_at: now,
        };
        self.records.retain(|r| r.root != record.root);
        self.records.push(record.clone());
        self.save_registry()?;
        Ok(self.materialize(record)?)
    }

    /// Open a registered workspace by id, verifying it still exists.
    pub fn open(&mut self, id: &str) -> Result<Workspace, WorkspaceError> {
        let record = self.get(id).ok_or_else(|| WorkspaceError::Corrupt(format!("unknown workspace id {id}")))?;
        if !record.root.is_dir() {
            return Err(WorkspaceError::RootMissing(record.root));
        }
        let workbench = record.root.join(".workbench");
        let meta_path = workbench.join("workspace.json");
        if !meta_path.exists() {
            return Err(WorkspaceError::Corrupt(format!(
                "workspace at `{}` lost its .workbench/workspace.json",
                record.root.display()
            )));
        }
        let meta: WorkspaceMeta = serde_json::from_str(&std::fs::read_to_string(&meta_path)?)?;
        if meta.schema_version > WORKSPACE_SCHEMA_VERSION {
            return Err(WorkspaceError::NewerSchema(meta.schema_version, WORKSPACE_SCHEMA_VERSION));
        }
        let now = chrono::Utc::now().to_rfc3339();
        let mut record = record;
        record.last_opened_at = now;
        std::fs::create_dir_all(workbench.join("plugin-state"))?;
        std::fs::create_dir_all(workbench.join("cache"))?;
        self.records.retain(|r| r.id != record.id);
        self.records.push(record.clone());
        self.save_registry()?;
        Ok(self.materialize(record)?)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), WorkspaceError> {
        self.records.retain(|r| r.id != id);
        self.save_registry()
    }

    fn materialize(&self, record: WorkspaceRecord) -> Result<Workspace, WorkspaceError> {
        let workbench_dir = record.root.join(".workbench");
        Ok(Workspace {
            state_root: workbench_dir.join("plugin-state"),
            cache_dir: workbench_dir.join("cache"),
            settings_path: workbench_dir.join("settings.json"),
            layout_path: workbench_dir.join("layout.json"),
            sqlite_path: workbench_dir.join("index.sqlite"),
            workbench_dir,
            record,
        })
    }
}

/// Migration for the workspace-local sqlite index (owned by the kernel).
/// Each migration is ordered and recorded; destructive steps back up first.
pub const SQLITE_MIGRATIONS: &[(&str, &str)] = &[(
    "0001_initial",
    r#"
    CREATE TABLE IF NOT EXISTS schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);
    "#,
)];

pub fn apply_sqlite_migrations(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
    )?;
    for (name, sql) in SQLITE_MIGRATIONS {
        let applied: bool = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations WHERE name = ?1", [name], |r| {
                r.get::<_, i64>(0)
            })?
            > 0;
        if !applied {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                rusqlite::params![name, chrono::Utc::now().to_rfc3339()],
            )?;
            tracing::info!(migration = name, "applied sqlite migration");
        }
    }
    Ok(())
}

/// Back up a database file before a destructive migration (spec §69).
pub fn backup_sqlite(path: &Path) -> Result<Option<PathBuf>, WorkspaceError> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_extension(format!("bak-{}", chrono::Utc::now().format("%Y%m%d%H%M%S")));
    std::fs::copy(path, &backup)?;
    tracing::info!(from = %path.display(), to = %backup.display(), "sqlite backup created");
    Ok(Some(backup))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), WorkspaceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ed-ws-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().elapsed().unwrap().subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn create_open_cycle() {
        let base = temp();
        let registry = base.join("workspaces.json");
        let root = base.join("Research");
        let mut mgr = WorkspaceManager::new(registry.clone()).unwrap();
        let ws = mgr.create("Research", root.clone(), true).unwrap();
        assert!(ws.workbench_dir.join("workspace.json").exists());
        assert!(ws.state_root.is_dir());
        assert_eq!(mgr.list().len(), 1);

        // Reopen via a fresh manager (registry persistence).
        let mut mgr2 = WorkspaceManager::new(registry).unwrap();
        let ws2 = mgr2.open(&ws.record.id).unwrap();
        assert_eq!(ws2.record.name, "Research");
        assert_eq!(ws2.root(), root.as_path());
    }

    #[test]
    fn open_missing_root_errors() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let ws = mgr.create("X", base.join("x"), true).unwrap();
        std::fs::remove_dir_all(ws.root()).unwrap();
        assert!(matches!(mgr.open(&ws.record.id), Err(WorkspaceError::RootMissing(_))));
    }

    #[test]
    fn register_existing_requires_workspace_meta() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        std::fs::create_dir_all(base.join("plain-dir")).unwrap();
        assert!(mgr.register_existing(base.join("plain-dir")).is_err());
    }

    #[test]
    fn recency_ordering() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let a = mgr.create("A", base.join("a"), true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = mgr.create("B", base.join("b"), true).unwrap();
        let recent: Vec<String> = mgr.list().into_iter().map(|r| r.name).collect();
        assert_eq!(recent, vec!["B", "A"]);
        mgr.open(&a.record.id).unwrap();
        let recent: Vec<String> = mgr.list().into_iter().map(|r| r.name).collect();
        assert_eq!(recent, vec!["A", "B"]);
        drop(b);
    }

    #[test]
    fn migrations_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply_sqlite_migrations(&conn).unwrap();
        apply_sqlite_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, SQLITE_MIGRATIONS.len() as i64);
    }

    #[test]
    fn layout_roundtrip() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let ws = mgr.create("L", base.join("l"), true).unwrap();
        assert!(ws.load_layout().is_null());
        ws.save_layout(&serde_json::json!({"tabs": []})).unwrap();
        assert_eq!(ws.load_layout()["tabs"].as_array().unwrap().len(), 0);
    }
}
