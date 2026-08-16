//! Schema lifecycle for [`MemoryDb`]: table/index/FTS creation, sync
//! triggers, idempotent migrations, and dim bookkeeping. Split out of
//! `db/mod.rs` (init-pro-9 TODO-52); behavior-neutral inherent-impl
//! continuation, all methods stay crate-private via `pub(super)`.

use anyhow::{anyhow, Context, Result};
use libsql::params;

use super::MemoryDb;

/// Current schema version understood by this code.
const SCHEMA_VERSION: i64 = 1;

impl MemoryDb {
    pub(super) async fn init_schema(&self) -> Result<()> {
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
    pub(super) async fn create_fts_triggers(&self) -> Result<()> {
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
    pub(super) async fn schema_version(&self) -> Result<i64> {
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
    pub(super) async fn set_schema_version(&self, v: i64) -> Result<()> {
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
    pub(super) async fn has_column(&self, table: &str, col: &str) -> Result<bool> {
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
    pub(super) async fn migrate_schema(&self) -> Result<()> {
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
    pub(super) async fn reconcile_dim(&self) -> Result<()> {
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
    pub(super) async fn ensure_fts_populated(&self) -> Result<()> {
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

    pub(super) async fn count(&self, table: &str) -> Result<i64> {
        // `table` is a fixed internal identifier, never user input.
        let sql = format!("SELECT count(*) FROM {table}");
        let mut rows = self.conn.query(&sql, ()).await.context("count rows")?;
        let row = rows.next().await.context("fetch count row")?.unwrap();
        Ok(row.get::<i64>(0)?)
    }
}
