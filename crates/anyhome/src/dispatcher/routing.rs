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
