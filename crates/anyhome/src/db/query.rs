//! Read-side queries: KNN vector search and FTS5/prefix `/where` (split out
//! of `db/mod.rs`, init-pro-9 TODO-52; behavior-neutral inherent-impl
//! continuation).

use anyhow::{Context, Result};
use libsql::params;

use super::{vec_to_json, MemoryDb, RowGet, WhereOutcome};

impl MemoryDb {
    /// Return up to `k` nearest memories `(key, text)` for `agent_id`, ordered
    /// by ascending cosine distance to `query_emb`.
    ///
    /// Scoped exact KNN: distances are computed only over the agent's own rows
    /// (init-pro-12 TODO-68 defect E). The previous shape ran
    /// `vector_top_k(..., k)` over the GLOBAL index and filtered by agent
    /// afterwards, so other agents' nearer rows consumed the k slots and an
    /// agent-scoped query could come back empty despite having memories
    /// (proven by `/agents/<id>/search_k1` in e2e facet 6). Brute-force cosine
    /// over one agent's partition is exact and cheap at memory-store scale;
    /// `id` breaks distance ties deterministically.
    pub async fn search(
        &self,
        agent_id: &str,
        query_emb: &[f32],
        k: i64,
    ) -> Result<Vec<(String, String)>> {
        let json = vec_to_json(query_emb);
        let mut rows = self
            .conn
            .query(
                "SELECT key, text FROM memories \
                 WHERE agent_id = ? AND emb IS NOT NULL \
                 ORDER BY vector_distance_cos(emb, vector(?)) ASC, id ASC \
                 LIMIT ?",
                params![agent_id, json, k],
            )
            .await
            .context("scoped knn query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch search row")? {
            out.push((row.get_key()?, row.get_text()?));
        }
        Ok(out)
    }

    /// Full-text / prefix search for `/where/<cond>`, scoped to `agent_id`.
    ///
    /// - `key:<prefix>` — filter memories whose key starts with `<prefix>`
    ///   (parameterized `LIKE`, never string-interpolated).
    /// - anything else — an FTS5 MATCH expression run against `memories_fts`
    ///   (bound as a parameter, so this stays injection-safe).
    ///
    /// Returns [`WhereOutcome::Rows`] ordered by relevance (FTS) or key
    /// (prefix). An empty/whitespace condition returns no rows rather than
    /// erroring. A malformed MATCH expression returns
    /// [`WhereOutcome::InvalidFts`] instead of failing opaquely, so callers
    /// can tell "no results" from "bad query".
    pub async fn where_search(
        &self,
        agent_id: &str,
        cond: &str,
        k: i64,
    ) -> Result<WhereOutcome> {
        let cond = cond.trim();
        if cond.is_empty() {
            return Ok(WhereOutcome::Rows(Vec::new()));
        }
        if let Some(prefix) = cond.strip_prefix("key:") {
            let rows = self.key_prefix_search(agent_id, prefix.trim(), k).await?;
            return Ok(WhereOutcome::Rows(rows));
        }
        // FTS5 MATCH. A bound parameter keeps this injection-safe even though
        // the expression syntax is user-controlled. Malformed expressions
        // surface as SqliteFailure at fetch time and are classified below —
        // the surrounding SQL is fixed, so those messages can only originate
        // from the user-supplied MATCH value.
        let mut rows = match self
            .conn
            .query(
                "SELECT m.key, m.text FROM memories_fts f \
                 JOIN memories m ON m.rowid = f.rowid \
                 WHERE memories_fts MATCH ? AND m.agent_id = ? \
                 ORDER BY rank LIMIT ?",
                params![cond, agent_id, k],
            )
            .await
        {
            Ok(rows) => rows,
            Err(e) => return classify_fts_error(e, cond),
        };
        let mut out: Vec<(String, String)> = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => out.push((row.get_key()?, row.get_text()?)),
                Ok(None) => break,
                Err(e) => return classify_fts_error(e, cond),
            }
        }
        Ok(WhereOutcome::Rows(out))
    }

    async fn key_prefix_search(
        &self,
        agent_id: &str,
        prefix: &str,
        k: i64,
    ) -> Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = self
            .conn
            .query(
                "SELECT key, text FROM memories \
                 WHERE agent_id = ? AND key LIKE ? || '%' ORDER BY key LIMIT ?",
                params![agent_id, prefix, k],
            )
            .await
            .context("key prefix query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch prefix row")? {
            out.push((row.get_key()?, row.get_text()?));
        }
        Ok(out)
    }
}

/// Classify an FTS5 fetch failure: SQLite messages that can only be caused by
/// the user-supplied MATCH expression become [`WhereOutcome::InvalidFts`];
/// anything else is a real database error and is propagated. The message
/// shapes were verified empirically against bundled libsql SQLite:
/// `fts5: syntax error near ...`, `unterminated string`,
/// `unknown special query: ...`.
fn classify_fts_error(e: libsql::Error, cond: &str) -> Result<WhereOutcome> {
    if let libsql::Error::SqliteFailure(_, msg) = &e {
        let m = msg.to_lowercase();
        let user_caused = m.starts_with("fts5:")
            || m.starts_with("unterminated string")
            || m.starts_with("unknown special query");
        if user_caused {
            return Ok(WhereOutcome::InvalidFts(cond.to_string()));
        }
    }
    Err(e).context("fts5 match query")
}
