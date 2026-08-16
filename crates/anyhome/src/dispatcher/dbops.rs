//! DB-backed operations behind the magic paths (extracted from
//! [`crate::dispatcher`], init-pro-8 TODO-47). All are scoped by `agent_id`
//! and read/write through [`crate::db::MemoryDb`] / the shared
//! [`crate::db::Embedder`].

use nfsserve::nfs::*;

use crate::db::WhereOutcome;
use crate::magic::{magic_attr_sized, magic_file_id, MagicEntry, CAT_INDEX, DEFAULT_K};

use super::AnyHomeFs;

impl AnyHomeFs {
    // ---- DB-backed operations (all scoped by agent_id) ----

    /// Embed `query` and return up to `k` KNN rows as `<key>\t<text>\n`.
    /// `k` comes from the entry ([`DEFAULT_K`] for plain `/search`).
    pub(super) async fn run_search(
        &self,
        agent_id: &str,
        query: &str,
        k: i64,
    ) -> std::result::Result<Vec<u8>, nfsstat3> {
        let emb = self
            .embedder
            .embed(query)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let rows = self
            .db
            .search(agent_id, &emb, k)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut out = String::new();
        for (key, text) in rows {
            out.push_str(&format!("{}\t{}\n", key, text));
        }
        Ok(out.into_bytes())
    }

    /// Embed `text` and upsert it under `(agent_id, key)`. Returns the
    /// persisted byte size (`text.len()`) so the WRITE path can size its
    /// fattr without reading the row back — the upsert just stored exactly
    /// these bytes (init-pro-8 TODO-48: one fewer full-text DB read per
    /// WRITE RPC on the hot path).
    pub(super) async fn run_index(
        &self,
        agent_id: &str,
        key: &str,
        text: &str,
    ) -> std::result::Result<u64, nfsstat3> {
        let emb = self
            .embedder
            .embed(text)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        self.db
            .upsert(agent_id, key, text, &emb)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(text.len() as u64)
    }

    /// Full-text / prefix query for `/where/<cond>` -> `<key>\t<text>\n`
    /// lines. A malformed FTS condition renders as a single first line
    /// `# error: invalid FTS query ...`: NFS READ has no errno channel for
    /// per-call syntax problems, and a text marker lets agents distinguish
    /// "no results" from "query written wrong" (valid queries are unchanged).
    pub(super) async fn run_where(
        &self,
        agent_id: &str,
        cond: &str,
    ) -> std::result::Result<Vec<u8>, nfsstat3> {
        let mut out = String::new();
        match self
            .db
            .where_search(agent_id, cond, DEFAULT_K)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?
        {
            WhereOutcome::Rows(rows) => {
                for (key, text) in rows {
                    out.push_str(&format!("{}\t{}\n", key, text));
                }
            }
            WhereOutcome::InvalidFts(bad) => {
                out.push_str(&format!("# error: invalid FTS query: {bad}\n"));
            }
        }
        Ok(out.into_bytes())
    }

    /// Read back the stored text for an indexed key.
    pub(super) async fn index_text(
        &self,
        agent_id: &str,
        key: &str,
    ) -> std::result::Result<Vec<u8>, nfsstat3> {
        match self.db.get_text(agent_id, key).await {
            Ok(Some(text)) => Ok(text.into_bytes()),
            Ok(None) => Ok(format!("# no indexed memory under key: {}\n", key).into_bytes()),
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    /// Recover `(agent_id, key)` from an index fileid. Tries the registry
    /// first; falls back to scanning the default agent's keys (for restart
    /// recovery of global `/index` files).
    pub(super) async fn index_name_for(&self, id: fileid3) -> Option<(String, String)> {
        if let Some(entry) = self.magic.get_entry(id) {
            return Some((entry.agent_id, entry.name));
        }
        // Fallback: default agent only (agent-scoped files need a fresh lookup).
        let keys = self.db.list_keys("").await.ok()?;
        for key in keys {
            if magic_file_id(CAT_INDEX, &key) == id {
                self.magic
                    .register(id, MagicEntry::new("", &key, CAT_INDEX));
                return Some((String::new(), key));
            }
        }
        None
    }

    /// Attributes with the real content byte size for an index file.
    pub(super) async fn index_attr(&self, id: fileid3) -> fattr3 {
        let size = match self.index_name_for(id).await {
            Some((agent_id, key)) => self
                .db
                .get_text(&agent_id, &key)
                .await
                .ok()
                .flatten()
                .map(|t| t.len() as u64)
                .unwrap_or(0),
            None => 0,
        };
        magic_attr_sized(id, size)
    }
}

#[cfg(test)]
mod tests {
    //! Direct unit tests for the DB-backed magic-path operations
    //! (init-pro-9 TODO-51): `run_index` / `run_search` / `run_where`.

    use std::sync::Arc;

    use tempfile::tempdir;

    use crate::backend::RealFs;
    use crate::db::{DummyEmbedder, Embedder, MemoryDb};

    use super::*;

    async fn make() -> AnyHomeFs {
        let root = tempdir().expect("tempdir").keep();
        let real = RealFs::new(root).unwrap();
        let db = MemoryDb::open(":memory:", 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        AnyHomeFs::new(real, db, emb)
    }

    #[tokio::test]
    async fn run_index_writes_through_and_returns_byte_size() {
        let fs = make().await;
        let size = fs.run_index("", "k1", "hello world").await.unwrap();
        assert_eq!(size, "hello world".len() as u64);
        assert_eq!(
            fs.db.get_text("", "k1").await.unwrap().as_deref(),
            Some("hello world"),
            "run_index must upsert write-through"
        );
    }

    #[tokio::test]
    async fn run_search_ranks_exact_match_first() {
        let fs = make().await;
        fs.run_index("", "k1", "hello world").await.unwrap();
        fs.run_index("", "k2", "totally unrelated words")
            .await
            .unwrap();

        let out = fs.run_search("", "hello world", 16).await.unwrap();
        let text = String::from_utf8(out).unwrap();
        let first = text.lines().next().expect("at least one result");
        assert_eq!(first, "k1\thello world", "exact match ranks top-1");
    }

    #[tokio::test]
    async fn run_search_is_agent_scoped() {
        let fs = make().await;
        fs.run_index("alpha", "k", "shared text").await.unwrap();

        let out = fs.run_search("", "shared text", 16).await.unwrap();
        assert!(out.is_empty(), "default agent must not see alpha's rows");

        let out = fs.run_search("alpha", "shared text", 16).await.unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "k\tshared text\n");
    }

    #[tokio::test]
    async fn run_where_matches_fts_and_key_prefix() {
        let fs = make().await;
        fs.run_index("", "notes/a", "alpha beta gamma")
            .await
            .unwrap();
        fs.run_index("", "other", "delta epsilon").await.unwrap();

        let out = fs.run_where("", "beta").await.unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "notes/a\talpha beta gamma\n"
        );

        let out = fs.run_where("", "key:notes/").await.unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "notes/a\talpha beta gamma\n"
        );

        let out = fs.run_where("", "zebra").await.unwrap();
        assert!(out.is_empty(), "no-hit where returns empty rows");
    }

    #[tokio::test]
    async fn run_where_renders_malformed_fts_as_error_line() {
        let fs = make().await;
        fs.run_index("", "k", "some text").await.unwrap();
        let out = fs.run_where("", "\"unclosed").await.unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("# error: invalid FTS query: "),
            "malformed FTS must render an error marker, got: {text:?}"
        );
    }
}
