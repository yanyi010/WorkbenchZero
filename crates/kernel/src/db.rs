//! Small helpers for workspace-scoped SQLite services (artifacts + search).

use wz_artifacts::ArtifactRegistry;
use wz_common::MutexRecover;
use wz_search::SearchIndex;

use crate::{KResult, WorkspaceState};

/// Run `f` under the workspace's single SQLite connection. Poisoning is
/// recovered: one panicking query handler must not brick every later
/// artifacts/search call (the conn is in a defined state after unwinding —
/// rusqlite statements are not reentrant, so a poisoned guard's in-flight
/// statement was already dropped).
pub fn with_conn<T>(
    ws: &WorkspaceState,
    f: impl FnOnce(&rusqlite::Connection) -> KResult<T>,
) -> KResult<T> {
    let conn = ws.conn.lock_or_recover();
    f(&conn)
}

pub fn artifacts<'a>(conn: &'a rusqlite::Connection) -> ArtifactRegistry<'a> {
    ArtifactRegistry::new(conn)
}

pub fn search<'a>(conn: &'a rusqlite::Connection) -> SearchIndex<'a> {
    SearchIndex::new(conn)
}
