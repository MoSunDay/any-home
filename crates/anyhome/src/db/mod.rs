//! Vector-backed memory store built on embedded libSQL (core-only).
//!
//! Each memory is a `(key, text, embedding)` row. Search is KNN over the vector
//! index. A single [`libsql::Connection`] is shared (it is `Clone` and backed by
//! an `Arc`, with SQLite compiled in SERIALIZED mode), so no `Mutex` is needed.

pub mod embedder;

use anyhow::{Context, Result};
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
    /// vectors; it cannot change for a given DB file without recreating the table.
    pub async fn open(path: &str, dim: usize) -> Result<Self> {
        let db = Builder::new_local(path).build().await.context("open libsql database")?;
        let conn = db.connect().context("connect libsql database")?;
        let store = MemoryDb { conn, dim };
        store.init_schema().await?;
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
        Ok(())
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
