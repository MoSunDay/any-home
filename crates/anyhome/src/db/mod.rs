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

pub mod embedder;
#[cfg(feature = "embed-fastembed")]
pub mod fastembed;

use anyhow::{anyhow, Context, Result};
use libsql::{params, Builder, Connection, Row};

pub use embedder::{dummy_embed, DummyEmbedder, Embedder, DUMMY_DIM};

/// Current schema version understood by this code.
const SCHEMA_VERSION: i64 = 1;

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

    async fn init_schema(&self) -> Result<()> {
        // `dim` must be a literal (it cannot be a bound parameter in FLOAT32(N)).
        let create_table = format!(
            "CREATE TABLE IF NOT EXISTS memories(\
               id INTEGER PRIMARY KEY,\
               agent_id TEXT NOT NULL DEFAULT '',\
               key TEXT NOT NULL,\
               text TEXT,\
               emb FLOAT32({dim}),\
               UNIQUE(agent_id, key)\
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
        self.create_fts_triggers().await?;
        Ok(())
    }

    /// Recreate the three FTS5 sync triggers on `memories`. Called both during
    /// initial schema creation and after a table-rebuild migration (DROP TABLE
    /// removes triggers). Uses `CREATE TRIGGER IF NOT EXISTS` so it is safe to
    /// call redundantly.
    async fn create_fts_triggers(&self) -> Result<()> {
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

    /// Read the recorded schema version from `meta` (0 if not yet set).
    async fn schema_version(&self) -> Result<i64> {
        let mut rows = self
            .conn
            .query("SELECT value FROM meta WHERE key = 'schema_version'", ())
            .await
            .context("read schema version")?;
        let row = rows.next().await.context("fetch schema_version row")?;
        drop(rows);
        match row {
            None => Ok(0),
            Some(r) => Ok(r.get::<String>(0)?.parse().unwrap_or(0)),
        }
    }

    /// Persist the schema version stamp.
    async fn set_schema_version(&self, v: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES ('schema_version', ?) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![v.to_string()],
            )
            .await
            .context("record schema version")?;
        Ok(())
    }

    /// Check whether column `col` exists on `table`.
    async fn has_column(&self, table: &str, col: &str) -> Result<bool> {
        // `table` and `col` are fixed internal identifiers, never user input.
        let sql = format!("PRAGMA table_info({table})");
        let mut rows = self.conn.query(&sql, ()).await.context("pragma table_info")?;
        while let Some(row) = rows.next().await.context("fetch pragma row")? {
            // PRAGMA table_info columns: cid, name, type, notnull, dflt_value, pk
            let name: String = row.get::<String>(1).unwrap_or_default();
            if name == col {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Idempotent migration from older schema versions to [`SCHEMA_VERSION`].
    ///
    /// v0 -> v1: adds the `agent_id` column and changes the unique constraint
    /// from `key` (column-level) to `(agent_id, key)` (table-level). Since
    /// SQLite cannot ALTER a column constraint, this is done via the standard
    /// table-rebuild pattern: create new table → copy data → drop old → rename
    /// → recreate vector index + FTS triggers.
    async fn migrate_schema(&self) -> Result<()> {
        let version = self.schema_version().await?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        if !self.has_column("memories", "agent_id").await? {
            // v0 table has `key TEXT UNIQUE` but no `agent_id`. Rebuild to the
            // current schema, preserving all existing rows with agent_id = ''.
            let create = format!(
                "CREATE TABLE memories_new(\
                   id INTEGER PRIMARY KEY,\
                   agent_id TEXT NOT NULL DEFAULT '',\
                   key TEXT NOT NULL,\
                   text TEXT,\
                   emb FLOAT32({dim}),\
                   UNIQUE(agent_id, key)\
                 )",
                dim = self.dim
            );
            self.conn.execute(&create, ()).await.context("create memories_new")?;
            self.conn
                .execute(
                    "INSERT INTO memories_new(id, agent_id, key, text, emb) \
                     SELECT id, '', key, text, emb FROM memories",
                    (),
                )
                .await
                .context("copy memories data during migration")?;
            self.conn.execute("DROP TABLE memories", ()).await.context("drop old memories")?;
            self.conn
                .execute("ALTER TABLE memories_new RENAME TO memories", ())
                .await
                .context("rename memories_new")?;
            // Recreate vector index (not carried over by the rebuild).
            self.conn
                .execute(
                    "CREATE INDEX IF NOT EXISTS memories_vec_idx \
                     ON memories ( libsql_vector_idx(emb) )",
                    (),
                )
                .await
                .context("recreate vector index after migration")?;
            // Recreate FTS5 triggers (DROP TABLE removed them).
            self.create_fts_triggers().await?;
            // Rebuild FTS index to ensure rowid consistency.
            self.conn
                .execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", ())
                .await
                .context("rebuild fts after migration")?;
        }
        self.set_schema_version(SCHEMA_VERSION).await?;
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

    /// Insert or replace the memory under `(agent_id, key)`.
    pub async fn upsert(&self, agent_id: &str, key: &str, text: &str, emb: &[f32]) -> Result<()> {
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

    /// Return up to `k` nearest memories `(key, text)` for `agent_id`, ordered
    /// by ascending cosine distance to `query_emb`. The vector index is global;
    /// results are filtered by agent_id after the KNN lookup.
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
                "SELECT m.key, m.text \
                 FROM vector_top_k('memories_vec_idx', vector(?), ?) v \
                 JOIN memories m ON m.rowid = v.id \
                 WHERE m.agent_id = ?",
                params![json, k, agent_id],
            )
            .await
            .context("vector_top_k query")?;
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

/// Distinguishable outcome of a `/where` query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WhereOutcome {
    /// Matching `(key, text)` rows.
    Rows(Vec<(String, String)>),
    /// The user-supplied FTS5 MATCH expression is malformed; carries the
    /// offending condition. Rendered by the dispatcher as a `# error:` line.
    InvalidFts(String),
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
