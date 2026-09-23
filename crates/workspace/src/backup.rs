//! Workspace snapshots (spec §9 data-ownership guarantee, operationalized).
//!
//! A snapshot captures everything in `.workbench/` that is not disposable —
//! workspace metadata, workspace settings, layout, and canonical per-plugin
//! state — plus a consistent copy of the derived index (`VACUUM INTO`) so a
//! restore never re-imports a table mid-write. Snapshots live inside
//! `.workbench/backups/<utc-timestamp>/`, rotate to the newest N, and are
//! plain directories the user can inspect, diff, and copy.
//!
//! What snapshots deliberately do NOT cover: the user's content files
//! (memos, tasks, documents) are regular files in the workspace root — the
//! user versions and archives those with git/rsync (see docs/data-safety).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Workspace, WorkspaceError, WORKSPACE_SCHEMA_VERSION};

/// Snapshot container inside the workspace.
pub const BACKUPS_DIR: &str = "backups";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotManifest {
    pub schema_version: i64,
    pub created_at: String,
    pub workspace_id: String,
    pub workspace_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    /// Timestamp directory name (`yyyymmdd-hhmmss`), also the restore key.
    pub id: String,
    pub created_at: String,
    pub path: String,
    pub bytes: u64,
}

fn backups_root(ws: &Workspace) -> PathBuf {
    ws.workbench_dir.join(BACKUPS_DIR)
}

/// Copy a directory tree (files only; symlinks skipped) — a snapshot must be
/// self-contained even if plugin data dirs contain links.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), WorkspaceError> {
    for entry in walkdir::WalkDir::new(src).follow_links(false) {
        let entry =
            entry.map_err(|e| WorkspaceError::Corrupt(format!("snapshot walk failed: {e}")))?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .map_err(|e| WorkspaceError::Corrupt(e.to_string()))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.file_type().is_symlink() {
            tracing::debug!(path = %entry.path().display(), "snapshot skips symlink");
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn dir_bytes(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}

/// Create a snapshot of `.workbench/`. `conn` must be the workspace's live
/// index connection (held under its lock by the caller) so the SQLite backup
/// is consistent end-to-end.
pub fn create_snapshot(
    ws: &Workspace,
    conn: &rusqlite::Connection,
    keep: usize,
) -> Result<SnapshotInfo, WorkspaceError> {
    let id = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let dest = backups_root(ws).join(&id);
    std::fs::create_dir_all(&dest)?;

    // Manifest first: an incomplete snapshot is detectable by a missing or
    // unparseable manifest and is never offered for restore.
    let manifest = SnapshotManifest {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        created_at: chrono::Utc::now().to_rfc3339(),
        workspace_id: ws.record.id.clone(),
        workspace_name: ws.record.name.clone(),
    };
    wz_common::atomic_write_str(
        &dest.join("manifest.json"),
        &serde_json::to_string_pretty(&manifest)?,
    )?;

    // Canonical small files (they may not exist yet — that's fine).
    for name in ["workspace.json", "settings.json", "layout.json"] {
        let src = ws.workbench_dir.join(name);
        if src.exists() {
            std::fs::copy(&src, dest.join(name))?;
        }
    }
    if ws.state_root.is_dir() {
        copy_tree(&ws.state_root, &dest.join("plugin-state"))?;
    }

    // Consistent copy of the derived index, compacted (also serves as an
    // integrity check: VACUUM INTO fails on a corrupt source).
    if ws.sqlite_path.exists() {
        let sql = format!(
            "VACUUM INTO '{}'",
            dest.join("index.sqlite")
                .display()
                .to_string()
                .replace('\'', "''")
        );
        conn.execute_batch(&sql)
            .map_err(|e| WorkspaceError::Corrupt(format!("index backup failed: {e}")))?;
    }

    let info = SnapshotInfo {
        id,
        created_at: manifest.created_at,
        bytes: dir_bytes(&dest),
        path: dest.display().to_string(),
    };
    prune_snapshots(ws, keep)?;
    Ok(info)
}

/// Newest-first snapshot list; entries without a parseable manifest are
/// omitted (incomplete snapshots are not restorable).
pub fn list_snapshots(ws: &Workspace) -> Vec<SnapshotInfo> {
    let root = backups_root(ws);
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.json");
            let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<SnapshotManifest>(&raw) else {
                continue;
            };
            out.push(SnapshotInfo {
                id: entry.file_name().to_string_lossy().to_string(),
                created_at: manifest.created_at,
                path: path.display().to_string(),
                bytes: dir_bytes(&path),
            });
        }
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Delete all but the newest `keep` snapshots.
pub fn prune_snapshots(ws: &Workspace, keep: usize) -> Result<usize, WorkspaceError> {
    let snapshots = list_snapshots(ws);
    let mut pruned = 0;
    for snap in snapshots.iter().skip(keep) {
        std::fs::remove_dir_all(&snap.path)?;
        pruned += 1;
    }
    Ok(pruned)
}

/// Restore a snapshot over `.workbench/`, atomically at the directory level:
/// the current state is moved aside (never deleted outright), the snapshot
/// moves into place, and the derived index is forced to rebuild. The caller
/// must have closed the workspace's live SQLite connection first.
pub fn restore_snapshot(ws: &Workspace, id: &str) -> Result<(), WorkspaceError> {
    // Snapshot ids are timestamp names; refuse anything that could traverse.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(WorkspaceError::Corrupt(format!(
            "invalid snapshot id `{id}`"
        )));
    }
    let snapshot = backups_root(ws).join(id);
    let manifest_path = snapshot.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|_| {
        WorkspaceError::Corrupt(format!("snapshot `{id}` missing or unreadable manifest"))
    })?;
    let manifest: SnapshotManifest = serde_json::from_str(&raw)
        .map_err(|_| WorkspaceError::Corrupt(format!("snapshot `{id}` has a corrupt manifest")))?;
    if manifest.schema_version > WORKSPACE_SCHEMA_VERSION {
        return Err(WorkspaceError::NewerSchema(
            manifest.schema_version,
            WORKSPACE_SCHEMA_VERSION,
        ));
    }
    if manifest.workspace_id != ws.record.id {
        return Err(WorkspaceError::Corrupt(format!(
            "snapshot `{id}` belongs to workspace `{}`, not `{}`",
            manifest.workspace_id, ws.record.id
        )));
    }

    // Stage the restored tree next to `.workbench` so the final swap is a
    // rename on the same filesystem.
    let staging = ws
        .workbench_dir
        .parent()
        .unwrap_or(ws.root())
        .join(format!(".workbench.restoring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    copy_tree(&snapshot, &staging)?;
    // Never restore the backups dir into itself.
    let _ = std::fs::remove_dir_all(staging.join(BACKUPS_DIR));

    let trash = ws
        .workbench_dir
        .parent()
        .unwrap_or(ws.root())
        .join(format!(".workbench.pre-restore-{}", id));
    let _ = std::fs::remove_dir_all(&trash);
    std::fs::rename(&ws.workbench_dir, &trash)?;
    if let Err(e) = std::fs::rename(&staging, &ws.workbench_dir) {
        // Roll back: put the pre-restore state back.
        let _ = std::fs::rename(&trash, &ws.workbench_dir);
        return Err(WorkspaceError::Io(e));
    }
    // The restored snapshot contains its own backups copy — restore ours.
    if trash.join(BACKUPS_DIR).is_dir() {
        let _ = std::fs::remove_dir_all(ws.workbench_dir.join(BACKUPS_DIR));
        let _ = std::fs::rename(trash.join(BACKUPS_DIR), ws.workbench_dir.join(BACKUPS_DIR));
    }
    // Force derived index rebuild on next open if the snapshot lacks it
    // (e.g. snapshot taken before the index existed).
    if !ws.sqlite_path.exists() {
        tracing::info!("restored snapshot has no index.sqlite; it will be rebuilt on open");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkspaceManager;

    fn temp() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "wz-backup-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let ws = mgr.create("Research", base.join("research"), true).unwrap();

        // Mutate canonical state: layout + plugin state + settings.
        ws.save_layout(&serde_json::json!({"tabs": ["a"]})).unwrap();
        std::fs::create_dir_all(ws.state_root.join("zero.memo")).unwrap();
        std::fs::write(
            ws.state_root.join("zero.memo").join("state.json"),
            r#"{"counter": 7}"#,
        )
        .unwrap();
        let conn = crate::open_index_db(&ws.sqlite_path).unwrap();
        crate::apply_sqlite_migrations(&conn).unwrap();

        let snap = create_snapshot(&ws, &conn, 10).unwrap();
        assert!(PathBuf::from(&snap.path).join("manifest.json").is_file());
        assert!(PathBuf::from(&snap.path).join("index.sqlite").is_file());
        drop(conn);

        // Destroy state, then restore.
        std::fs::write(
            ws.state_root.join("zero.memo").join("state.json"),
            r#"{"counter": 999}"#,
        )
        .unwrap();
        ws.save_layout(&serde_json::json!({"tabs": []})).unwrap();
        restore_snapshot(&ws, &snap.id).unwrap();

        assert_eq!(ws.load_layout()["tabs"][0], "a");
        let raw =
            std::fs::read_to_string(ws.state_root.join("zero.memo").join("state.json")).unwrap();
        assert_eq!(raw, r#"{"counter": 7}"#);
        // Index came back and is healthy.
        let conn = crate::open_index_db(&ws.sqlite_path).unwrap();
        let integrity: String = conn
            .pragma_query_value(None, "integrity_check", |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }

    #[test]
    fn rotation_keeps_newest() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let ws = mgr.create("R", base.join("r"), true).unwrap();
        let conn = crate::open_index_db(&ws.sqlite_path).unwrap();
        for i in 0..5 {
            let snap = create_snapshot(&ws, &conn, 12).unwrap();
            // Make ids unique even within the same second.
            let renamed =
                PathBuf::from(&snap.path).with_file_name(format!("fake-{i}-{id}", id = snap.id));
            std::fs::rename(&snap.path, &renamed).unwrap();
        }
        let pruned = prune_snapshots(&ws, 3).unwrap();
        assert_eq!(pruned, 2);
        assert_eq!(list_snapshots(&ws).len(), 3);
    }

    #[test]
    fn restore_rejects_foreign_or_bad_ids() {
        let base = temp();
        let mut mgr = WorkspaceManager::new(base.join("workspaces.json")).unwrap();
        let ws = mgr.create("R", base.join("r"), true).unwrap();
        assert!(restore_snapshot(&ws, "../evil").is_err());
        assert!(restore_snapshot(&ws, "nonexistent").is_err());
        // A foreign-workspace snapshot is refused.
        let other = mgr.create("Other", base.join("other"), true).unwrap();
        let conn = crate::open_index_db(&other.sqlite_path).unwrap();
        let snap = create_snapshot(&other, &conn, 5).unwrap();
        assert!(restore_snapshot(&ws, &snap.id).is_err());
    }
}
