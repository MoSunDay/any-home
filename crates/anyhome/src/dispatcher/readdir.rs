//! Readdir over the magic namespace (extracted from [`crate::dispatcher`],
//! init-pro-8 TODO-47). [`AnyHomeFs::magic_readdir`] handles every synthetic
//! directory; the trait method in the parent module delegates here and keeps
//! the real-FS fallthrough + root magic-dir append for itself.

use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, ReadDirResult};

use crate::magic::{
    magic_attr, magic_attr_sized, magic_category, magic_file_id, scoped_name, is_magic,
    MagicEntry, CAT_AGENT, CAT_INDEX, CAT_SEARCH_K_DIR, AGENTS_DIR,
};

use super::AnyHomeFs;

impl AnyHomeFs {
    /// Handle readdir for a synthetic directory: `/agents` (DB listing,
    /// auth-filtered), `/agents/<id>` (fixed subdirs), `/search_k<N>` (not
    /// enumerable), and the file-bearing magic dirs (`/index` sized from the
    /// DB, `/search` & `/where` from the in-memory registry).
    ///
    /// Returns `Ok(None)` when `dirid` is none of those, so the caller falls
    /// through to the real filesystem.
    pub(super) async fn magic_readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> std::result::Result<Option<ReadDirResult>, nfsstat3> {
        // /agents: list known agent directories from the DB, hiding any the
        // current caller is not authorized to access.
        if dirid == AGENTS_DIR {
            let caller = self.current_caller();
            let agents = self.db.list_agents().await.unwrap_or_default();
            let mut entries: Vec<DirEntry> = agents
                .into_iter()
                .filter(|agent_id| self.auth.permits(caller, agent_id))
                .map(|agent_id| {
                    let id = magic_file_id(CAT_AGENT, &agent_id);
                    self.magic
                        .register(id, MagicEntry::new(&agent_id, &agent_id, CAT_AGENT));
                    DirEntry {
                        fileid: id,
                        name: agent_id.into_bytes().into(),
                        attr: magic_attr(id),
                    }
                })
                .collect();
            entries.sort_by_key(|e| e.fileid);
            entries.retain(|e| e.fileid > start_after);
            let end = entries.len() <= max_entries;
            entries.truncate(max_entries);
            return Ok(Some(ReadDirResult { entries, end }));
        }

        // /agents/<id>: fixed listing of search/ index/ where/.
        if is_magic(dirid) && magic_category(dirid) == CAT_AGENT {
            let agent_id = self
                .magic
                .get_entry(dirid)
                .map(|e| e.agent_id)
                .unwrap_or_default();
            self.require_agent(&agent_id)?;
            let subdirs: [(&str, u8); 3] = [
                ("search", crate::magic::CAT_AGENT_SEARCH),
                ("index", crate::magic::CAT_AGENT_INDEX),
                ("where", crate::magic::CAT_AGENT_WHERE),
            ];
            let mut entries: Vec<DirEntry> = subdirs
                .iter()
                .map(|(name, cat)| {
                    let scoped = scoped_name(&agent_id, name);
                    let id = magic_file_id(*cat, &scoped);
                    self.magic
                        .register(id, MagicEntry::new(&agent_id, name, *cat));
                    DirEntry {
                        fileid: id,
                        name: name.as_bytes().to_vec().into(),
                        attr: magic_attr(id),
                    }
                })
                .collect();
            entries.sort_by_key(|e| e.fileid);
            entries.retain(|e| e.fileid > start_after);
            let end = entries.len() <= max_entries;
            entries.truncate(max_entries);
            return Ok(Some(ReadDirResult { entries, end }));
        }

        // /search_k<N> dirs: synthetic variants, deliberately not enumerable
        // (consistent with the DB layer's non-enumerable query surface).
        if is_magic(dirid) && magic_category(dirid) == CAT_SEARCH_K_DIR {
            return Ok(Some(ReadDirResult {
                entries: Vec::new(),
                end: true,
            }));
        }

        // Magic dirs with files (global + agent-scoped).
        if let Some(cat) = self.dir_category(dirid) {
            let agent_id = self.agent_id_of_dir(dirid);
            self.require_agent(&agent_id)?;
            if cat == CAT_INDEX {
                let mut sized: Vec<(fileid3, String, u64)> = self
                    .db
                    .list_with_size(&agent_id)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, sz)| {
                        let scoped = scoped_name(&agent_id, &key);
                        (magic_file_id(CAT_INDEX, &scoped), key, sz)
                    })
                    .collect();
                for (id, key, _) in &sized {
                    self.magic
                        .register(*id, MagicEntry::new(&agent_id, key, CAT_INDEX));
                }
                sized.sort_by_key(|(id, _, _)| *id);
                let mut entries: Vec<DirEntry> = sized
                    .into_iter()
                    .filter(|(id, _, _)| *id > start_after)
                    .map(|(id, name, sz)| DirEntry {
                        fileid: id,
                        name: name.into_bytes().into(),
                        attr: magic_attr_sized(id, sz),
                    })
                    .collect();
                let end = entries.len() <= max_entries;
                entries.truncate(max_entries);
                return Ok(Some(ReadDirResult { entries, end }));
            }
            // /search, /where: in-memory registry listing (virtual sizes).
            let mut rows = self.magic.list_for_cat(cat, &agent_id);
            rows.sort_by_key(|(id, _)| *id);
            let mut entries: Vec<DirEntry> = rows
                .into_iter()
                .filter(|(id, _)| *id > start_after)
                .map(|(id, name)| DirEntry {
                    fileid: id,
                    name: name.into_bytes().into(),
                    attr: magic_attr(id),
                })
                .collect();
            let end = entries.len() <= max_entries;
            entries.truncate(max_entries);
            return Ok(Some(ReadDirResult { entries, end }));
        }

        Ok(None)
    }
}
