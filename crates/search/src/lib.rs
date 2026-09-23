//! Full-text search index (SQLite FTS5) and the lexical half of Universal
//! Search (spec §25-27, §70-71).
//!
//! The FTS index is derived data: it may be rebuilt from plugin canonical
//! data at any time. Plugins submit `IndexDocument`s; the kernel merges
//! results from FTS and live providers. Semantic/vector search is optional
//! and never replaces lexical search.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("invalid document: {0}")]
    InvalidDocument(String),
}

/// A document submitted to the generic indexing service (spec §70).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexDocument {
    pub uri: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Owning plugin (stamped by the kernel, not trusted from input).
    #[serde(skip)]
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub uri: String,
    pub title: String,
    pub score: f64,
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS search_docs (
    uri TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS search_docs_plugin ON search_docs(plugin_id);
CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(
    uri UNINDEXED, title, body, tags
);
"#;

pub struct SearchIndex<'a> {
    conn: &'a Connection,
}

impl<'a> SearchIndex<'a> {
    pub fn init(conn: &Connection) -> Result<(), SearchError> {
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn validate(doc: &IndexDocument) -> Result<(), SearchError> {
        if doc.uri.is_empty() || doc.uri.len() > 2048 {
            return Err(SearchError::InvalidDocument(doc.uri.clone()));
        }
        if doc.title.len() > 512 {
            return Err(SearchError::InvalidDocument(format!("title too long: {}", doc.uri)));
        }
        Ok(())
    }

    pub fn upsert_many(&self, docs: &[IndexDocument]) -> Result<(), SearchError> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut del_fts = tx.prepare("DELETE FROM search_fts WHERE uri = ?1")?;
            let mut del_doc = tx.prepare("DELETE FROM search_docs WHERE uri = ?1")?;
            let mut ins_fts = tx.prepare("INSERT INTO search_fts (uri, title, body, tags) VALUES (?1, ?2, ?3, ?4)")?;
            let mut ins_doc = tx.prepare("INSERT INTO search_docs (uri, plugin_id) VALUES (?1, ?2)")?;
            for doc in docs {
                Self::validate(doc)?;
                del_fts.execute([&doc.uri])?;
                del_doc.execute([&doc.uri])?;
                ins_fts.execute(rusqlite::params![doc.uri, doc.title, doc.body, doc.tags.join(" ")])?;
                ins_doc.execute(rusqlite::params![doc.uri, doc.plugin_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove(&self, uri: &str) -> Result<bool, SearchError> {
        let tx = self.conn.unchecked_transaction()?;
        let a = tx.execute("DELETE FROM search_fts WHERE uri = ?1", [uri])?;
        let b = tx.execute("DELETE FROM search_docs WHERE uri = ?1", [uri])?;
        tx.commit()?;
        Ok(a > 0 && b > 0)
    }

    pub fn remove_by_plugin(&self, plugin_id: &str) -> Result<usize, SearchError> {
        let uris: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT uri FROM search_docs WHERE plugin_id = ?1")?;
            let rows = stmt.query_map([plugin_id], |r| r.get::<_, String>(0))?;
            rows.flatten().collect()
        };
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut del_fts = tx.prepare("DELETE FROM search_fts WHERE uri = ?1")?;
            let mut del_doc = tx.prepare("DELETE FROM search_docs WHERE plugin_id = ?1")?;
            for uri in &uris {
                del_fts.execute([uri])?;
            }
            del_doc.execute([plugin_id])?;
        }
        tx.commit()?;
        Ok(uris.len())
    }

    /// Build a safe FTS5 MATCH expression from a user query: each token
    /// becomes a quoted prefix term.
    pub fn build_match(query: &str) -> Result<String, SearchError> {
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect();
        if tokens.is_empty() {
            return Err(SearchError::InvalidQuery(query.to_string()));
        }
        Ok(tokens.join(" "))
    }

    /// Lexical search. Returns normalized scores (best = 1.0).
    pub fn query(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>, SearchError> {
        let matcher = Self::build_match(query)?;
        let mut stmt = self.conn.prepare(
            "SELECT f.uri, f.title, f.rank, d.plugin_id
             FROM search_fts f JOIN search_docs d ON d.uri = f.uri
             WHERE search_fts MATCH ?1
             ORDER BY f.rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![matcher, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut hits: Vec<SearchHit> = rows
            .flatten()
            .map(|(uri, title, rank, plugin_id)| SearchHit {
                uri,
                title,
                score: rank,
                plugin_id,
                snippet: None,
            })
            .collect();
        // Normalize: FTS5 rank is negative-better (bm25).
        if let Some(best) = hits.first().map(|h| h.score) {
            let worst = hits.last().map(|h| h.score).unwrap_or(best);
            let span = (best - worst).abs();
            for hit in &mut hits {
                hit.score = if span > f64::EPSILON { 1.0 - (best - hit.score).abs() / span } else { 1.0 };
            }
        }
        Ok(hits)
    }

    pub fn count(&self) -> Result<u64, SearchError> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM search_docs", [], |r| r.get::<_, i64>(0))? as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_idx(f: impl FnOnce(&Connection, SearchIndex)) {
        let conn = Connection::open_in_memory().unwrap();
        SearchIndex::init(&conn).unwrap();
        let idx = SearchIndex::new(&conn);
        f(&conn, idx);
    }

    fn doc(uri: &str, title: &str, body: &str) -> IndexDocument {
        IndexDocument {
            uri: uri.into(),
            title: title.into(),
            body: body.into(),
            tags: vec!["research".into()],
            metadata: None,
            plugin_id: "eigendesk.memo".into(),
        }
    }

    #[test]
    fn fts5_available() {
        with_idx(|conn, _| {
            conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x)").unwrap();
        });
    }

    #[test]
    fn upsert_and_query() {
        with_idx(|_, idx| {
            idx.upsert_many(&[
                doc("memo://1", "Berry convergence notes", "check Berry convergence carefully"),
                doc("memo://2", "Grocery list", "milk and eggs"),
            ])
            .unwrap();
            let hits = idx.query("berry convergence", 10).unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].uri, "memo://1");
            assert!((hits[0].score - 1.0).abs() < 1e-9);
        });
    }

    #[test]
    fn prefix_query_matches_partial() {
        with_idx(|_, idx| {
            idx.upsert_many(&[doc("memo://1", "Convergence checklist", "body")]).unwrap();
            assert_eq!(idx.query("conver", 10).unwrap().len(), 1);
        });
    }

    #[test]
    fn remove_by_plugin_only_own() {
        with_idx(|_, idx| {
            idx.upsert_many(&[
                doc("memo://1", "a", "a"),
                IndexDocument { uri: "task://1".into(), title: "t".into(), body: "b".into(), tags: vec![], metadata: None, plugin_id: "eigendesk.tasks".into() },
            ])
            .unwrap();
            assert_eq!(idx.remove_by_plugin("eigendesk.memo").unwrap(), 1);
            assert_eq!(idx.query("a", 10).unwrap().len(), 0);
            assert_eq!(idx.count().unwrap(), 1);
        });
    }

    #[test]
    fn upsert_replaces_document() {
        with_idx(|_, idx| {
            idx.upsert_many(&[doc("memo://1", "old title", "old body")]).unwrap();
            idx.upsert_many(&[doc("memo://1", "new title", "new body")]).unwrap();
            let hits = idx.query("new", 10).unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(idx.count().unwrap(), 1);
        });
    }

    #[test]
    fn empty_query_rejected() {
        with_idx(|_, idx| {
            assert!(idx.query("", 10).is_err());
            assert!(idx.query("   ", 10).is_err());
        });
    }

    #[test]
    fn match_expression_is_injection_safe() {
        let m = SearchIndex::build_match("berry \" OR 1=1 --").unwrap();
        assert!(!m.contains("OR 1=1"));
    }
}
