//! Vector-backed memory store built on embedded libSQL (core-only).
//!
//! Each memory is a `(key, text, embedding)` row. Search is KNN over the vector
//! index; `/where` is full-text over an FTS5 shadow table kept in sync by
//! triggers. A single [`libsql::Connection`] is shared (it is `Clone` and backed
//! by an `Arc`, with SQLite compiled in SERIALIZED mode), so no `Mutex` is
//! needed.
//!
//! Persistence (TODO-08): the embedding dimension is recorded in a `meta` table
//! on first open and validated on every reopen, so a stale DB file can never be
//! silently reopened with a mismatched embedder. `list_keys` lets the `/index`
//! directory rebuild its listing from the DB after a restart.

pub mod embedder;
#[cfg(feature = "embed-fastembed")]
pub mod fastembed;

use anyhow::{anyhow, Context, Result};
use libsql::{params, Builder, Connection, Row};

pub use embedder::{dummy_embed, DummyEmbedder, Embedder, DUMMY_DIM};

/// A vector memory store. Cheap to clone (shares one SQLite connection).
#[derive(Clone)]
pub struct MemoryDb {
    conn: Connection,
    dim: usize,
}

impl MemoryDb {
    /// Open (or create) the memory DB at `path`. Use `":memory:"` for ephemeral.
    ///
    /// `dim` is baked into the schema as a `FLOAT32(dim)` literal and into the
    /// vectors; it cannot change for a given DB file without recreating the
    /// table. On reopen the requested `dim` is checked against the value stored
    /// in the `meta` table (written on first open) — a mismatch is a hard error
    /// rather than a silent misalignment with the active embedder.
    pub async fn open(path: &str, dim: usize) -> Result<Self> {
        let db = Builder::new_local(path).build().await.context("open libsql database")?;
        let conn = db.connect().context("connect libsql database")?;
        let store = MemoryDb { conn, dim };
        store.init_schema().await?;
        store.reconcile_dim().await?;
        store.ensure_fts_populated().await?;
        Ok(store)
    }

    /// Dimensionality of the embedding column.
    pub fn dim(&self) -> usize {
        self.dim
    }

    async fn init_schema(&self) -> Result<()> {
        // `dim` must be a literal (it cannot be a bound parameter in FLOAT32(N)).
        let create_table = format!(
            "CREATE TABLE IF NOT EXISTS memories(\
               id INTEGER PRIMARY KEY,\
               key TEXT UNIQUE,\
               text TEXT,\
               emb FLOAT32({dim})\
             )",
            dim = self.dim
        );
        self.conn
            .execute(&create_table, ())
            .await
            .context("create memories table")?;
        // Vector index: no USING clause, default metric = cosine.
        self.conn
            .execute(
                "CREATE INDEX IF NOT EXISTS memories_vec_idx \
                 ON memories ( libsql_vector_idx(emb) )",
                (),
            )
            .await
            .context("create vector index")?;

        // Metadata table (persistence bookkeeping: recorded dim, schema version).
        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
                (),
            )
            .await
            .context("create meta table")?;

        // FTS5 full-text index over `text`, as an external-content table shadowing
        // `memories`. Triggers keep it in sync with inserts/updates/deletes.
        self.conn
            .execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts \
                 USING fts5(text, content='memories', content_rowid='id')",
                (),
            )
            .await
            .context("create memories_fts virtual table")?;
        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN \
                     INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text); \
                 END",
                (),
            )
            .await
            .context("create memories_ai trigger")?;
        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN \
                     INSERT INTO memories_fts(memories_fts, rowid, text) \
                         VALUES('delete', old.id, old.text); \
                 END",
                (),
            )
            .await
            .context("create memories_ad trigger")?;
        self.conn
            .execute(
                "CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN \
                     INSERT INTO memories_fts(memories_fts, rowid, text) \
                         VALUES('delete', old.id, old.text); \
                     INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text); \
                 END",
                (),
            )
            .await
            .context("create memories_au trigger")?;
        Ok(())
    }

    /// Record the dim on first open; on reopen, refuse a mismatched dim.
    async fn reconcile_dim(&self) -> Result<()> {
        let mut rows = self
            .conn
            .query("SELECT value FROM meta WHERE key = 'dim'", ())
            .await
            .context("read stored dim")?;
        let stored = rows.next().await.context("fetch stored dim row")?;
        drop(rows);
        match stored {
            None => {
                self.conn
                    .execute(
                        "INSERT INTO meta(key, value) VALUES ('dim', ?)",
                        params![self.dim.to_string()],
                    )
                    .await
                    .context("record dim")?;
            }
            Some(row) => {
                let v: String = row.get::<String>(0).context("read dim value")?;
                let stored: usize = v
                    .parse()
                    .with_context(|| format!("stored dim is not a number: {v}"))?;
                if stored != self.dim {
                    return Err(anyhow!(
                        "embedding dimension mismatch: db was created with dim={stored} \
                         but embedder/db opened with dim={dim}. \
                         Use a fresh --db-path or recreate the table.",
                        dim = self.dim
                    ));
                }
            }
        }
        Ok(())
    }

    /// Backfill the FTS index for rows that predate it (e.g. an old DB upgraded
    /// in place). Cheap guard: only rebuilds when memories has rows but the FTS
    /// table is empty.
    async fn ensure_fts_populated(&self) -> Result<()> {
        let mem_count = self.count("memories").await?;
        if mem_count == 0 {
            return Ok(());
        }
        let fts_count = self.count("memories_fts").await?;
        if fts_count == 0 {
            self.conn
                .execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", ())
                .await
                .context("rebuild memories_fts")?;
        }
        Ok(())
    }

    async fn count(&self, table: &str) -> Result<i64> {
        // `table` is a fixed internal identifier, never user input.
        let sql = format!("SELECT count(*) FROM {table}");
        let mut rows = self.conn.query(&sql, ()).await.context("count rows")?;
        let row = rows.next().await.context("fetch count row")?.unwrap();
        Ok(row.get::<i64>(0)?)
    }

    /// Insert or replace the memory under `key`.
    pub async fn upsert(&self, key: &str, text: &str, emb: &[f32]) -> Result<()> {
        let json = vec_to_json(emb);
        self.conn
            .execute(
                "INSERT INTO memories(key, text, emb) \
                 VALUES (?, ?, vector(?)) \
                 ON CONFLICT(key) DO UPDATE SET text = excluded.text, emb = excluded.emb",
                params![key, text, json],
            )
            .await
            .map(|_| ())
            .context("upsert memory")
    }

    /// Remove the memory under `key` (no error if absent).
    pub async fn delete(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM memories WHERE key = ?", params![key])
            .await
            .map(|_| ())
            .context("delete memory")
    }

    /// All stored keys, ordered (used to rebuild `/index` listings after restart).
    pub async fn list_keys(&self) -> Result<Vec<String>> {
        let mut rows = self
            .conn
            .query("SELECT key FROM memories ORDER BY key", ())
            .await
            .context("list keys")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch key row")? {
            out.push(row.get::<String>(0)?);
        }
        Ok(out)
    }

    /// All stored `(key, byte_size)` pairs, ordered by key. Used to render the
    /// `/index` listing with accurate file sizes without a follow-up `getattr`
    /// per entry. `length(CAST(text AS BLOB))` is the byte length of the text
    /// (TEXT `length()` would count characters, not bytes).
    pub async fn list_with_size(&self) -> Result<Vec<(String, u64)>> {
        let mut rows = self
            .conn
            .query(
                "SELECT key, length(CAST(text AS BLOB)) FROM memories ORDER BY key",
                (),
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

    /// The stored text for `key`, if present (lets `/index/<key>` be read back).
    pub async fn get_text(&self, key: &str) -> Result<Option<String>> {
        let mut rows = self
            .conn
            .query("SELECT text FROM memories WHERE key = ?", params![key])
            .await
            .context("get text")?;
        let row = rows.next().await.context("fetch text row")?;
        Ok(row.map(|r| r.get::<String>(0).unwrap_or_default()))
    }

    /// Whether a memory exists under `key` (used by exclusive-create).
    pub async fn exists(&self, key: &str) -> Result<bool> {
        let mut rows = self
            .conn
            .query("SELECT 1 FROM memories WHERE key = ? LIMIT 1", params![key])
            .await
            .context("exists check")?;
        Ok(rows.next().await.context("fetch exists row")?.is_some())
    }

    /// Return up to `k` nearest memories `(key, text)`, ordered by ascending
    /// cosine distance to `query_emb`.
    pub async fn search(&self, query_emb: &[f32], k: i64) -> Result<Vec<(String, String)>> {
        // NOTE: returns (key, text), not the SQL rowid — the key is the
        // user-facing identifier written to /index/<key>.
        let json = vec_to_json(query_emb);
        let mut rows = self
            .conn
            .query(
                "SELECT m.key, m.text FROM vector_top_k('memories_vec_idx', vector(?), ?) v JOIN memories m ON m.rowid = v.id",
                params![json, k],
            )
            .await
            .context("vector_top_k query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch search row")? {
            let key: String = row.get_key()?;
            let text: String = row.get_text()?;
            out.push((key, text));
        }
        Ok(out)
    }

    /// Full-text / prefix search for `/where/<cond>`.
    ///
    /// - `key:<prefix>` — filter memories whose key starts with `<prefix>`
    ///   (parameterized `LIKE`, never string-interpolated).
    /// - anything else — an FTS5 MATCH expression run against `memories_fts`
    ///   (bound as a parameter; malformed expressions simply yield no rows).
    ///
    /// Returns `(key, text)` rows ordered by relevance (FTS) or key (prefix).
    /// An empty/whitespace condition returns no rows rather than erroring.
    pub async fn where_search(&self, cond: &str, k: i64) -> Result<Vec<(String, String)>> {
        let cond = cond.trim();
        if cond.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(prefix) = cond.strip_prefix("key:") {
            return self.key_prefix_search(prefix.trim(), k).await;
        }
        // FTS5 MATCH. A bound parameter keeps this injection-safe even though
        // the expression syntax is user-controlled; bad queries -> no rows.
        let mut rows = self
            .conn
            .query(
                "SELECT m.key, m.text FROM memories_fts f \
                 JOIN memories m ON m.rowid = f.rowid \
                 WHERE memories_fts MATCH ? ORDER BY rank LIMIT ?",
                params![cond, k],
            )
            .await
            .context("fts5 match query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.context("fetch where row")? {
            out.push((row.get_key()?, row.get_text()?));
        }
        Ok(out)
    }

    async fn key_prefix_search(&self, prefix: &str, k: i64) -> Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return Ok(Vec::new());
        }
        let mut rows = self
            .conn
            .query(
                "SELECT key, text FROM memories WHERE key LIKE ? || '%' ORDER BY key LIMIT ?",
                params![prefix, k],
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

/// Encode a slice as the JSON array string libSQL's `vector()` accepts:
/// `"[1.0,2.0,3.0]"`. Uses `{:?}` so every float carries a decimal point.
pub fn vec_to_json(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 8 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{:?}", x));
    }
    s.push(']');
    s
}

// Small typed accessors so callers don't repeat `get::<T>(0)?`.
trait RowGet {
    fn get_key(&self) -> Result<String>;
    fn get_text(&self) -> Result<String>;
}

impl RowGet for Row {
    fn get_key(&self) -> Result<String> {
        Ok(self.get::<String>(0)?)
    }
    fn get_text(&self) -> Result<String> {
        Ok(self.get::<String>(1)?)
    }
}
