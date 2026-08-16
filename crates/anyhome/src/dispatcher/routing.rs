//! Routing helpers that map magic fileids/dirs to categories, registry
//! entries, owning agents, and parents (extracted from
//! [`crate::dispatcher`], init-pro-8 TODO-47). Pure in-registry resolution:
//! no DB access, no awaiting.

use nfsserve::nfs::*;

use crate::backend::ROOT_ID;
use crate::magic::{
    agent_dir_base_cat, magic_category, magic_file_id, scoped_name, search_scoped_name,
    MagicEntry, CAT_AGENT, CAT_SEARCH, CAT_INDEX, CAT_WHERE, CAT_SEARCH_K_DIR, DEFAULT_K,
    AGENTS_DIR, INDEX_DIR, SEARCH_DIR, WHERE_DIR,
};

use super::AnyHomeFs;

impl AnyHomeFs {
    // ---- Routing helpers ----

    /// Resolve the base file category for a magic directory (global or
    /// agent-scoped). Returns `None` if `dirid` is not a magic dir that
    /// contains magic files.
    pub(super) fn dir_category(&self, dirid: fileid3) -> Option<u8> {
        match dirid {
            SEARCH_DIR => Some(CAT_SEARCH),
            INDEX_DIR => Some(CAT_INDEX),
            WHERE_DIR => Some(CAT_WHERE),
            _ => match magic_category(dirid) {
                CAT_SEARCH_K_DIR => Some(CAT_SEARCH),
                c => agent_dir_base_cat(c),
            },
        }
    }

    /// Resolve the `(fileid, registry entry)` for a magic file named `name`
    /// inside magic dir `dirid` (whose file category is `cat` and owning agent
    /// is `agent_id`). `/search_k<N>` dirs salt the scoped name with `k` so
    /// their fileids never collide with `/search/<query>` (or another N), and
    /// carry `k` in the entry for the read path.
    pub(super) fn magic_file_in(
        &self,
        dirid: fileid3,
        agent_id: &str,
        name: &str,
        cat: u8,
    ) -> (fileid3, MagicEntry) {
        if cat == CAT_SEARCH && magic_category(dirid) == CAT_SEARCH_K_DIR {
            let k = self
                .magic
                .get_entry(dirid)
                .map(|e| e.k)
                .unwrap_or(DEFAULT_K);
            let id = magic_file_id(cat, &search_scoped_name(agent_id, k, name));
            (id, MagicEntry::with_k(agent_id, name, cat, k))
        } else {
            let id = magic_file_id(cat, &scoped_name(agent_id, name));
            (id, MagicEntry::new(agent_id, name, cat))
        }
    }

    /// The agent_id that a magic directory belongs to. Global magic dirs
    /// return `""`; agent-scoped dirs look up the registry (which is populated
    /// by `lookup` before any operation).
    pub(super) fn agent_id_of_dir(&self, dirid: fileid3) -> String {
        if matches!(dirid, SEARCH_DIR | INDEX_DIR | WHERE_DIR) {
            return String::new();
        }
        self.magic
            .get_entry(dirid)
            .map(|e| e.agent_id)
            .unwrap_or_default()
    }

    /// Parent directory for `..` lookups in the magic namespace.
    pub(super) fn magic_parent_of(&self, id: fileid3) -> fileid3 {
        if matches!(id, SEARCH_DIR | INDEX_DIR | WHERE_DIR | AGENTS_DIR) {
            return ROOT_ID;
        }
        match magic_category(id) {
            CAT_AGENT => AGENTS_DIR,
            CAT_SEARCH_K_DIR => match self.magic.get_entry(id) {
                Some(e) if !e.agent_id.is_empty() => {
                    magic_file_id(CAT_AGENT, &e.agent_id)
                }
                _ => ROOT_ID,
            },
            c if agent_dir_base_cat(c).is_some() => self
                .magic
                .get_entry(id)
                .map(|e| magic_file_id(CAT_AGENT, &e.agent_id))
                .unwrap_or(AGENTS_DIR),
            _ => ROOT_ID,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Direct unit tests for the routing helpers (init-pro-9 TODO-51):
    //! behavior pinning for `dir_category` / `magic_file_in` /
    //! `agent_id_of_dir` / `magic_parent_of`.

    use std::sync::Arc;

    use tempfile::tempdir;

    use crate::backend::RealFs;
    use crate::db::{DummyEmbedder, Embedder, MemoryDb};
    use crate::magic::{search_k_dir_id, CAT_AGENT_INDEX, CAT_AGENT_SEARCH};

    use super::*;

    async fn make() -> AnyHomeFs {
        let root = tempdir().expect("tempdir").keep();
        let real = RealFs::new(root).unwrap();
        let db = MemoryDb::open(":memory:", 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        AnyHomeFs::new(real, db, emb)
    }

    #[tokio::test]
    async fn dir_category_maps_magic_dirs_to_file_cats() {
        let fs = make().await;
        assert_eq!(fs.dir_category(SEARCH_DIR), Some(CAT_SEARCH));
        assert_eq!(fs.dir_category(INDEX_DIR), Some(CAT_INDEX));
        assert_eq!(fs.dir_category(WHERE_DIR), Some(CAT_WHERE));

        // Agent-scoped magic dirs resolve to their base file category.
        let a_search = magic_file_id(CAT_AGENT_SEARCH, &scoped_name("alpha", "search"));
        let a_index = magic_file_id(CAT_AGENT_INDEX, &scoped_name("alpha", "index"));
        assert_eq!(fs.dir_category(a_search), Some(CAT_SEARCH));
        assert_eq!(fs.dir_category(a_index), Some(CAT_INDEX));

        // Agent container dirs and non-magic ids hold no magic files.
        let agent_dir = magic_file_id(CAT_AGENT, "alpha");
        assert_eq!(fs.dir_category(agent_dir), None);
        assert_eq!(fs.dir_category(AGENTS_DIR), None);
        assert_eq!(fs.dir_category(ROOT_ID), None);
    }

    #[tokio::test]
    async fn dir_category_of_search_k_dir_is_search() {
        let fs = make().await;
        let id = search_k_dir_id("", 3);
        fs.magic
            .register(id, MagicEntry::with_k("", "search", CAT_SEARCH_K_DIR, 3));
        assert_eq!(fs.dir_category(id), Some(CAT_SEARCH));
    }

    #[tokio::test]
    async fn magic_file_in_builds_scoped_ids_and_entries() {
        let fs = make().await;

        // Global index file: scoped name is "\0name".
        let (id, entry) = fs.magic_file_in(INDEX_DIR, "", "note", CAT_INDEX);
        assert_eq!(id, magic_file_id(CAT_INDEX, &scoped_name("", "note")));
        assert_eq!(entry.agent_id, "");
        assert_eq!(entry.name, "note");
        assert_eq!(entry.category, CAT_INDEX);
        assert_eq!(entry.k, DEFAULT_K);

        // Agent-scoped where file.
        let (id, entry) = fs.magic_file_in(WHERE_DIR, "alpha", "beta", CAT_WHERE);
        assert_eq!(id, magic_file_id(CAT_WHERE, &scoped_name("alpha", "beta")));
        assert_eq!(entry.agent_id, "alpha");

        // search_k dir salts the scoped name and carries k in the entry.
        let dir = search_k_dir_id("alpha", 5);
        fs.magic
            .register(dir, MagicEntry::with_k("alpha", "search", CAT_SEARCH_K_DIR, 5));
        let (id, entry) = fs.magic_file_in(dir, "alpha", "q", CAT_SEARCH);
        assert_eq!(
            id,
            magic_file_id(CAT_SEARCH, &search_scoped_name("alpha", 5, "q"))
        );
        assert_eq!(entry.k, 5);
        // No collision with the same query under plain /search.
        assert_ne!(id, magic_file_id(CAT_SEARCH, &scoped_name("alpha", "q")));
    }

    #[tokio::test]
    async fn agent_id_of_dir_global_vs_scoped() {
        let fs = make().await;
        assert_eq!(fs.agent_id_of_dir(SEARCH_DIR), "");
        assert_eq!(fs.agent_id_of_dir(INDEX_DIR), "");
        assert_eq!(fs.agent_id_of_dir(WHERE_DIR), "");

        let agent_dir = magic_file_id(CAT_AGENT, "alpha");
        fs.magic
            .register(agent_dir, MagicEntry::new("alpha", "alpha", CAT_AGENT));
        assert_eq!(fs.agent_id_of_dir(agent_dir), "alpha");

        // Unregistered magic dir falls back to the default agent.
        assert_eq!(fs.agent_id_of_dir(magic_file_id(CAT_AGENT, "ghost")), "");
    }

    #[tokio::test]
    async fn magic_parent_of_walks_up_the_magic_tree() {
        let fs = make().await;
        for dir in [SEARCH_DIR, INDEX_DIR, WHERE_DIR, AGENTS_DIR] {
            assert_eq!(fs.magic_parent_of(dir), ROOT_ID);
        }

        // Agent container dir -> /agents.
        let agent_dir = magic_file_id(CAT_AGENT, "alpha");
        assert_eq!(fs.magic_parent_of(agent_dir), AGENTS_DIR);

        // Agent-scoped subdir -> that agent's container dir.
        let a_index = magic_file_id(CAT_AGENT_INDEX, &scoped_name("alpha", "index"));
        fs.magic
            .register(a_index, MagicEntry::new("alpha", "index", CAT_AGENT_INDEX));
        assert_eq!(fs.magic_parent_of(a_index), agent_dir);

        // Global search_k dir -> root; agent-scoped -> agent container dir.
        let gk = search_k_dir_id("", 4);
        fs.magic
            .register(gk, MagicEntry::with_k("", "search", CAT_SEARCH_K_DIR, 4));
        assert_eq!(fs.magic_parent_of(gk), ROOT_ID);
        let ak = search_k_dir_id("alpha", 4);
        fs.magic
            .register(ak, MagicEntry::with_k("alpha", "search", CAT_SEARCH_K_DIR, 4));
        assert_eq!(fs.magic_parent_of(ak), agent_dir);
    }
}
