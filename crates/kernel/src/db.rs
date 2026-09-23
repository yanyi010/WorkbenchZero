//! Small helpers for workspace-scoped SQLite services (artifacts + search).

use eigendesk_artifacts::ArtifactRegistry;
use eigendesk_search::SearchIndex;

use crate::{KResult, WorkspaceState};

pub fn with_conn<T>(
    ws: &WorkspaceState,
    f: impl FnOnce(&rusqlite::Connection) -> KResult<T>,
) -> KResult<T> {
    let conn = ws.conn.lock().unwrap();
    f(&conn)
}

pub fn artifacts<'a>(conn: &'a rusqlite::Connection) -> ArtifactRegistry<'a> {
    ArtifactRegistry::new(conn)
}

pub fn search<'a>(conn: &'a rusqlite::Connection) -> SearchIndex<'a> {
    SearchIndex::new(conn)
}
