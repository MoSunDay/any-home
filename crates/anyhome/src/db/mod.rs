//! Vector-backed memory store built on embedded libSQL (core-only).
//!
//! Each memory is an `(agent_id, key, text, embedding)` row. Search is KNN over
//! the vector index; `/where` is full-text over an FTS5 shadow table kept in
//! sync by triggers. A single [`libsql::Connection`] is shared (it is `Clone`
//! and backed by an `Arc`, with SQLite compiled in SERIALIZED mode), so no
//! `Mutex` is needed.
//!
//! Multi-agent namespace isolation (TODO-21..25): every row carries an
//! `agent_id` that scopes all queries. The global magic paths (`/search`,
//! `/index`, `/where`) operate on the default agent (`agent_id = ""`), while
//! `/agents/<id>/...` paths scope their operations to the named agent.
//! `list_agents()` enumerates known non-default agents for directory listings.
//!
//! Persistence: the embedding dimension is recorded in a `meta` table on first
//! open and validated on every reopen. A `schema_version` key in the same table
//! drives idempotent migrations (v0 -> v1 adds the `agent_id` column and
//! changes the unique constraint from `key` to `(agent_id, key)`).
//!
//! Layout (init-pro-9 TODO-52): this file keeps the struct, open, durability
//! configuration, and the shared public helpers ([`WhereOutcome`],
//! [`vec_to_json`], [`RowGet`]). Inherent-impl method clusters live in
//! private submodules: [`schema`] (schema lifecycle/migrations), [`crud`]
//! (upsert/delete/list/get), [`query`] (KNN + FTS search).

pub mod embedder;
#[cfg(feature = "embed-fastembed")]
pub mod fastembed;

mod crud;
mod query;
mod schema;

use anyhow::{Context, Result};
use libsql::{Builder, Connection, Row};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub use embedder::{dummy_embed, DummyEmbedder, Embedder, DUMMY_DIM};

/// A vector memory store. Cheap to clone (shares one SQLite connection).
#[derive(Clone)]
pub struct MemoryDb {
    conn: Connection,
    dim: usize,
    /// `get_text` call counter (write-path read-amplification spike,
    /// init-pro-8 TODO-46). Shared across clones so the count always reflects
    /// the one underlying connection. Purely observational: no behavior change.
    text_reads: Arc<AtomicU64>,
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
        let store = MemoryDb { conn, dim, text_reads: Arc::new(AtomicU64::new(0)) };
        store.configure_durability().await?;
        store.init_schema().await?;
        store.migrate_schema().await?;
        store.reconcile_dim().await?;
        store.ensure_fts_populated().await?;
        Ok(store)
    }

    /// Establish explicit, crash-safe durability. Sets WAL journal mode (so a
    /// concurrent NFS `read` never blocks the writer) and `synchronous=FULL`
    /// (every committed write is fsynced before the RPC returns). This makes
    /// the durability guarantee explicit instead of relying on the bundled
    /// SQLite's compile-time defaults. `journal_mode=WAL` returns a row (the
    /// resulting mode), so it must go through `query`, not `execute` (which
    /// rejects row-returning statements). A `:memory:` DB is unaffected (WAL does
    /// not apply there) and the pragma returns `"memory"` without error.
    async fn configure_durability(&self) -> Result<()> {
        self.conn
            .query("PRAGMA journal_mode=WAL", ())
            .await
            .context("set WAL journal mode")?;
        self.conn
            .query("PRAGMA synchronous=FULL", ())
            .await
            .context("set synchronous=FULL")?;
        Ok(())
    }

    /// `(journal_mode, synchronous)` currently in effect. `synchronous`:
    /// 0=OFF, 1=NORMAL, 2=FULL, 3=EXTRA.
    pub async fn durability(&self) -> Result<(String, i64)> {
        let mut rows = self
            .conn
            .query("PRAGMA journal_mode", ())
            .await
            .context("query journal_mode")?;
        let mode: String = rows
            .next()
            .await
            .context("fetch journal_mode row")?
            .context("journal_mode row missing")?
            .get(0)
            .context("journal_mode value")?;
        let mut rows = self
            .conn
            .query("PRAGMA synchronous", ())
            .await
            .context("query synchronous")?;
        let sync: i64 = rows
            .next()
            .await
            .context("fetch synchronous row")?
            .context("synchronous row missing")?
            .get(0)
            .context("synchronous value")?;
        Ok((mode, sync))
    }

    /// Dimensionality of the embedding column.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// How many `get_text` calls have been served so far (spike
    /// instrumentation, see the field docs). Shared across clones.
    pub fn text_read_count(&self) -> u64 {
        self.text_reads.load(Ordering::Relaxed)
    }
}

/// Distinguishable outcome of a `/where` query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WhereOutcome {
    /// Matching `(key, text)` rows.
    Rows(Vec<(String, String)>),
    /// The user-supplied FTS5 MATCH expression is malformed; carries the
    /// offending condition. Rendered by the dispatcher as a `# error:` line.
    InvalidFts(String),
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
