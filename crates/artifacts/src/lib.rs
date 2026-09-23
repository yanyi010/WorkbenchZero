//! Artifact registry (spec §22-24).
//!
//! Artifacts let plugins refer to each other's data without a common storage
//! schema. An `ArtifactRef` carries a stable URI (`memo://...`, `task://...`,
//! `file://...`); the owning plugin resolves it. The registry persists
//! metadata + recency in SQLite (derived index data, spec §68).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("invalid artifact uri `{0}`")]
    InvalidUri(String),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("artifact `{0}` not found")]
    NotFound(String),
}

/// Stable, globally-unique-inside-workspace artifact URI (spec §23).
/// `scheme://opaque` where scheme is `[a-z][a-z0-9+.-]*`, total length <= 2048,
/// no whitespace or control characters.
pub fn validate_uri(uri: &str) -> Result<(), ArtifactError> {
    let invalid = || ArtifactError::InvalidUri(uri.to_string());
    if uri.len() > 2048 || uri.is_empty() {
        return Err(invalid());
    }
    if uri.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid());
    }
    let (scheme, rest) = uri.split_once("://").ok_or_else(invalid)?;
    if scheme.is_empty() || rest.is_empty() {
        return Err(invalid());
    }
    let mut chars = scheme.chars();
    let first = chars.next().ok_or_else(invalid)?;
    if !first.is_ascii_lowercase() {
        return Err(invalid());
    }
    if !scheme
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '-' || c == '.')
    {
        return Err(invalid());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub uri: String,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<String>,
}

pub struct ArtifactRegistry<'a> {
    conn: &'a Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS artifacts (
    uri TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    title TEXT,
    plugin_id TEXT NOT NULL,
    metadata_json TEXT,
    created_at TEXT,
    updated_at TEXT,
    last_opened_at TEXT
);
CREATE INDEX IF NOT EXISTS artifacts_plugin ON artifacts(plugin_id);
CREATE INDEX IF NOT EXISTS artifacts_opened ON artifacts(last_opened_at DESC);
"#;

impl<'a> ArtifactRegistry<'a> {
    pub fn init(conn: &Connection) -> Result<(), ArtifactError> {
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Upsert a batch of artifacts owned by one plugin (full sync semantics
    /// for the given URIs).
    pub fn upsert_many(&self, records: &[ArtifactRecord]) -> Result<(), ArtifactError> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO artifacts (uri, type, title, plugin_id, metadata_json, created_at, updated_at, last_opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(uri) DO UPDATE SET
                   type=excluded.type, title=excluded.title, plugin_id=excluded.plugin_id,
                   metadata_json=excluded.metadata_json, updated_at=excluded.updated_at,
                   last_opened_at=COALESCE(artifacts.last_opened_at, excluded.last_opened_at)",
            )?;
            for r in records {
                validate_uri(&r.uri)?;
                stmt.execute(rusqlite::params![
                    r.uri,
                    r.r#type,
                    r.title,
                    r.plugin_id,
                    r.metadata.as_ref().map(|m| m.to_string()),
                    r.created_at,
                    r.updated_at,
                    r.last_opened_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, uri: &str) -> Result<bool, ArtifactError> {
        let n = self
            .conn
            .execute("DELETE FROM artifacts WHERE uri = ?1", [uri])?;
        Ok(n > 0)
    }

    pub fn remove_by_plugin(&self, plugin_id: &str) -> Result<usize, ArtifactError> {
        let n = self
            .conn
            .execute("DELETE FROM artifacts WHERE plugin_id = ?1", [plugin_id])?;
        Ok(n)
    }

    fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<ArtifactRecord> {
        Ok(ArtifactRecord {
            uri: row.get(0)?,
            r#type: row.get(1)?,
            title: row.get(2)?,
            plugin_id: row.get(3)?,
            metadata: row
                .get::<_, Option<String>>(4)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            last_opened_at: row.get(7)?,
        })
    }

    const COLS: &'static str =
        "uri, type, title, plugin_id, metadata_json, created_at, updated_at, last_opened_at";

    pub fn describe(&self, uri: &str) -> Result<ArtifactRecord, ArtifactError> {
        self.conn
            .query_row(
                &format!("SELECT {} FROM artifacts WHERE uri = ?1", Self::COLS),
                [uri],
                Self::row_to_record,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => ArtifactError::NotFound(uri.to_string()),
                other => other.into(),
            })
    }

    pub fn list_recent(&self, limit: u32) -> Result<Vec<ArtifactRecord>, ArtifactError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM artifacts WHERE last_opened_at IS NOT NULL
             ORDER BY last_opened_at DESC LIMIT ?1",
            Self::COLS
        ))?;
        let rows = stmt.query_map([limit], Self::row_to_record)?;
        Ok(rows.flatten().collect())
    }

    pub fn list_by_type(&self, type_name: &str) -> Result<Vec<ArtifactRecord>, ArtifactError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM artifacts WHERE type = ?1 ORDER BY updated_at DESC",
            Self::COLS
        ))?;
        let rows = stmt.query_map([type_name], Self::row_to_record)?;
        Ok(rows.flatten().collect())
    }

    pub fn list_by_plugin(&self, plugin_id: &str) -> Result<Vec<ArtifactRecord>, ArtifactError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM artifacts WHERE plugin_id = ?1 ORDER BY updated_at DESC",
            Self::COLS
        ))?;
        let rows = stmt.query_map([plugin_id], Self::row_to_record)?;
        Ok(rows.flatten().collect())
    }

    pub fn mark_opened(&self, uri: &str) -> Result<(), ArtifactError> {
        let now = chrono::Utc::now().to_rfc3339();
        let n = self.conn.execute(
            "UPDATE artifacts SET last_opened_at = ?1 WHERE uri = ?2",
            [&now, uri],
        )?;
        if n == 0 {
            return Err(ArtifactError::NotFound(uri.to_string()));
        }
        Ok(())
    }

    pub fn count(&self) -> Result<u64, ArtifactError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM artifacts", [], |r| r.get::<_, i64>(0))?
            as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ArtifactRegistry::init(&conn).unwrap();
        conn
    }

    fn rec(uri: &str, plugin: &str, ty: &str) -> ArtifactRecord {
        ArtifactRecord {
            uri: uri.into(),
            r#type: ty.into(),
            title: Some(format!("Title {uri}")),
            plugin_id: plugin.into(),
            metadata: Some(serde_json::json!({"tags": ["a"]})),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some("2026-01-01T00:00:00Z".into()),
            last_opened_at: None,
        }
    }

    #[test]
    fn upsert_describe_roundtrip() {
        let c = conn();
        let reg = ArtifactRegistry::new(&c);
        reg.upsert_many(&[rec("memo://1", "zero.memo", "memo")])
            .unwrap();
        let got = reg.describe("memo://1").unwrap();
        assert_eq!(got.title.as_deref(), Some("Title memo://1"));
        assert_eq!(got.metadata.unwrap()["tags"][0], "a");
    }

    #[test]
    fn upsert_replaces() {
        let c = conn();
        let reg = ArtifactRegistry::new(&c);
        reg.upsert_many(&[rec("memo://1", "zero.memo", "memo")])
            .unwrap();
        let mut r2 = rec("memo://1", "zero.memo", "memo");
        r2.title = Some("New title".into());
        reg.upsert_many(&[r2]).unwrap();
        assert_eq!(
            reg.describe("memo://1").unwrap().title.as_deref(),
            Some("New title")
        );
    }

    #[test]
    fn recent_ordering_and_mark_opened() {
        let c = conn();
        let reg = ArtifactRegistry::new(&c);
        reg.upsert_many(&[
            rec("memo://a", "zero.memo", "memo"),
            rec("memo://b", "zero.memo", "memo"),
        ])
        .unwrap();
        reg.mark_opened("memo://a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        reg.mark_opened("memo://b").unwrap();
        let recent = reg.list_recent(10).unwrap();
        assert_eq!(recent[0].uri, "memo://b");
        assert_eq!(recent[1].uri, "memo://a");
    }

    #[test]
    fn remove_by_plugin() {
        let c = conn();
        let reg = ArtifactRegistry::new(&c);
        reg.upsert_many(&[
            rec("memo://1", "zero.memo", "memo"),
            rec("task://2", "zero.tasks", "task"),
        ])
        .unwrap();
        assert_eq!(reg.remove_by_plugin("zero.memo").unwrap(), 1);
        assert!(reg.describe("memo://1").is_err());
        assert!(reg.describe("task://2").is_ok());
    }

    #[test]
    fn uri_validation() {
        assert!(validate_uri("memo://019392").is_ok());
        assert!(validate_uri("file:///home/me/x.dat").is_ok());
        assert!(validate_uri("paper://doi/10.1103/x").is_ok());
        assert!(validate_uri("terminal://session/a1c2").is_ok());
        assert!(validate_uri("memo:no-scheme").is_err());
        assert!(validate_uri("://empty").is_err());
        assert!(validate_uri("Memo://upper").is_err());
        assert!(validate_uri("memo://sp ace").is_err());
        assert!(validate_uri(&format!("memo://{}", "x".repeat(3000))).is_err());
    }
}
