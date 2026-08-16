//! CRUD operations on the `memories` table (split out of `db/mod.rs`,
//! init-pro-9 TODO-52; behavior-neutral inherent-impl continuation).

use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use libsql::params;

use super::{vec_to_json, MemoryDb};

impl MemoryDb {
    /// Insert or replace the memory under `(agent_id, key)`.
    pub async fn upsert(&self, agent_id: &str, key: &str, text: &str, emb: &[f32]) -> Result<()> {
        // Dim gate formerly enforced by the vector-index insert path (schema v2
        // retired the index; see `check_emb_dim`).
        self.check_emb_dim(emb)?;
        let json = vec_to_json(emb);
        self.conn
            .execute(
                "INSERT INTO memories(agent_id, key, text, emb) \
                 VALUES (?, ?, ?, vector(?)) \
                 ON CONFLICT(agent_id, key) DO UPDATE SET \
                     text = excluded.text, emb = excluded.emb",
                params![agent_id, key, text, json],
            )
            .await
            .map(|_| ())
            .context("upsert memory")
    }

    /// Remove the memory under `(agent_id, key)` (no error if absent).
    pub async fn delete(&self, agent_id: &str, key: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM memories WHERE agent_id = ? AND key = ?",
                params![agent_id, key],
            )
            .await
            .map(|_| ())
            .context("delete memory")
    }

    /// All known non-default agent_ids, ordered (for `/agents` directory listing).
    pub async fn list_agents(&self) -> Result<Vec<String>> {
        let mut rows = self
            .conn
            .query(
                "SELECT DISTINCT agent_id FROM memories \
                 WHERE agent_id != '' ORDER BY agent_id",
                (),
            )
            .await
            .context("list agents")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch agent row")? {
            out.push(row.get::<String>(0)?);
        }
        Ok(out)
    }

    /// All stored keys for `agent_id`, ordered (used to rebuild `/index`
    /// listings after restart).
    pub async fn list_keys(&self, agent_id: &str) -> Result<Vec<String>> {
        let mut rows = self
            .conn
            .query(
                "SELECT key FROM memories WHERE agent_id = ? ORDER BY key",
                params![agent_id],
            )
            .await
            .context("list keys")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch key row")? {
            out.push(row.get::<String>(0)?);
        }
        Ok(out)
    }

    /// All stored `(key, byte_size)` pairs for `agent_id`, ordered by key.
    /// Used to render the `/index` listing with accurate file sizes without a
    /// follow-up `getattr` per entry. `length(CAST(text AS BLOB))` is the byte
    /// length of the text (TEXT `length()` would count characters, not bytes).
    pub async fn list_with_size(&self, agent_id: &str) -> Result<Vec<(String, u64)>> {
        let mut rows = self
            .conn
            .query(
                "SELECT key, length(CAST(text AS BLOB)) FROM memories \
                 WHERE agent_id = ? ORDER BY key",
                params![agent_id],
            )
            .await
            .context("list keys with size")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch sized key row")? {
            let key = row.get::<String>(0)?;
            let sz = row.get::<i64>(1)? as u64;
            out.push((key, sz));
        }
        Ok(out)
    }

    /// The stored text for `(agent_id, key)`, if present.
    pub async fn get_text(&self, agent_id: &str, key: &str) -> Result<Option<String>> {
        self.text_reads.fetch_add(1, Ordering::Relaxed);
        let mut rows = self
            .conn
            .query(
                "SELECT text FROM memories WHERE agent_id = ? AND key = ?",
                params![agent_id, key],
            )
            .await
            .context("get text")?;
        let row = rows.next().await.context("fetch text row")?;
        Ok(row.map(|r| r.get::<String>(0).unwrap_or_default()))
    }

    /// Whether a memory exists under `(agent_id, key)` (used by exclusive-create).
    pub async fn exists(&self, agent_id: &str, key: &str) -> Result<bool> {
        let mut rows = self
            .conn
            .query(
                "SELECT 1 FROM memories WHERE agent_id = ? AND key = ? LIMIT 1",
                params![agent_id, key],
            )
            .await
            .context("exists check")?;
        Ok(rows.next().await.context("fetch exists row")?.is_some())
    }
}
