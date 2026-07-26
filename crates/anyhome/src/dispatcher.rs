//! The dispatcher: an [`NFSFileSystem`] that routes each call either to the
//! real on-disk backend ([`RealFs`]) or to the DB layer via magic paths.
//!
//! ## Global magic paths (backward-compatible, default agent `agent_id = ""`)
//! - `/search/<query>` — read returns KNN results for `<query>` as text.
//! - `/index/<key>`    — write embeds and upserts the written text under `<key>`.
//! - `/where/<cond>`   — read returns FTS5 full-text matches, or `key:<prefix>`.
//!
//! ## Agent-scoped magic paths (multi-agent namespace isolation)
//! - `/agents/<id>/search/<query>` — KNN scoped to agent `<id>`.
//! - `/agents/<id>/index/<key>`    — write scoped to agent `<id>`.
//! - `/agents/<id>/where/<cond>`   — FTS/prefix scoped to agent `<id>`.
//!
//! Agents are created implicitly on first write. `/agents` readdir lists known
//! agents via `SELECT DISTINCT agent_id`.

use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::backend::{RealFs, ROOT_ID};
use crate::db::{Embedder, MemoryDb};
use crate::magic::{
    percent_decode, slice_read, magic_attr, magic_attr_sized, magic_category, magic_file_id,
    scoped_name, agent_dir_base_cat, is_magic, is_magic_dir, MagicEntry, MagicRegistry,
    CAT_AGENT, CAT_INDEX, CAT_SEARCH, CAT_WHERE, DEFAULT_K,
    INDEX_DIR, SEARCH_DIR, WHERE_DIR, AGENTS_DIR,
};

/// The dispatcher filesystem: real FS + DB, joined by magic paths.
pub struct AnyHomeFs {
    pub real: RealFs,
    pub db: MemoryDb,
    pub embedder: Arc<dyn Embedder>,
    magic: MagicRegistry,
}

impl AnyHomeFs {
    pub fn new(real: RealFs, db: MemoryDb, embedder: Arc<dyn Embedder>) -> Self {
        AnyHomeFs {
            real,
            db,
            embedder,
            magic: MagicRegistry::default(),
        }
    }

    // ---- DB-backed operations (all scoped by agent_id) ----

    /// Embed `query` and return up to [`DEFAULT_K`] KNN rows as `<key>\t<text>\n`.
    async fn run_search(
        &self,
        agent_id: &str,
        query: &str,
    ) -> std::result::Result<Vec<u8>, nfsstat3> {
        let emb = self
            .embedder
            .embed(query)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let rows = self
            .db
            .search(agent_id, &emb, DEFAULT_K)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut out = String::new();
        for (key, text) in rows {
            out.push_str(&format!("{}\t{}\n", key, text));
        }
        Ok(out.into_bytes())
    }

    /// Embed `text` and upsert it under `(agent_id, key)`.
    async fn run_index(
        &self,
        agent_id: &str,
        key: &str,
        text: &str,
    ) -> std::result::Result<(), nfsstat3> {
        let emb = self
            .embedder
            .embed(text)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        self.db
            .upsert(agent_id, key, text, &emb)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)
    }

    /// Full-text / prefix query for `/where/<cond>` -> `<key>\t<text>\n` lines.
    async fn run_where(
        &self,
        agent_id: &str,
        cond: &str,
    ) -> std::result::Result<Vec<u8>, nfsstat3> {
        let rows = self
            .db
            .where_search(agent_id, cond, DEFAULT_K)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut out = String::new();
        for (key, text) in rows {
            out.push_str(&format!("{}\t{}\n", key, text));
        }
        Ok(out.into_bytes())
    }

    /// Read back the stored text for an indexed key.
    async fn index_text(
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

    // ---- Routing helpers ----

    /// Resolve the base file category for a magic directory (global or
    /// agent-scoped). Returns `None` if `dirid` is not a magic dir that
    /// contains magic files.
    fn dir_category(&self, dirid: fileid3) -> Option<u8> {
        match dirid {
            SEARCH_DIR => Some(CAT_SEARCH),
            INDEX_DIR => Some(CAT_INDEX),
            WHERE_DIR => Some(CAT_WHERE),
            _ => agent_dir_base_cat(magic_category(dirid)),
        }
    }

    /// The agent_id that a magic directory belongs to. Global magic dirs
    /// return `""`; agent-scoped dirs look up the registry (which is populated
    /// by `lookup` before any operation).
    fn agent_id_of_dir(&self, dirid: fileid3) -> String {
        if matches!(dirid, SEARCH_DIR | INDEX_DIR | WHERE_DIR) {
            return String::new();
        }
        self.magic
            .get_entry(dirid)
            .map(|e| e.agent_id)
            .unwrap_or_default()
    }

    /// Parent directory for `..` lookups in the magic namespace.
    fn magic_parent_of(&self, id: fileid3) -> fileid3 {
        if matches!(id, SEARCH_DIR | INDEX_DIR | WHERE_DIR | AGENTS_DIR) {
            return ROOT_ID;
        }
        match magic_category(id) {
            CAT_AGENT => AGENTS_DIR,
            c if agent_dir_base_cat(c).is_some() => self
                .magic
                .get_entry(id)
                .map(|e| magic_file_id(CAT_AGENT, &e.agent_id))
                .unwrap_or(AGENTS_DIR),
            _ => ROOT_ID,
        }
    }

    /// Recover `(agent_id, key)` from an index fileid. Tries the registry
    /// first; falls back to scanning the default agent's keys (for restart
    /// recovery of global `/index` files).
    async fn index_name_for(&self, id: fileid3) -> Option<(String, String)> {
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
    async fn index_attr(&self, id: fileid3) -> fattr3 {
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

#[async_trait]
impl NFSFileSystem for AnyHomeFs {
    fn capabilities(&self) -> VFSCapabilities {
        VFSCapabilities::ReadWrite
    }

    fn root_dir(&self) -> fileid3 {
        ROOT_ID
    }

    async fn lookup(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<fileid3, nfsstat3> {
        if filename.as_slice() == b"." {
            return Ok(dirid);
        }
        if filename.as_slice() == b".." {
            return Ok(if is_magic(dirid) {
                self.magic_parent_of(dirid)
            } else {
                self.real.lookup(dirid, &b"..".to_vec().into()).await?
            });
        }

        // Root level: global magic dirs + /agents.
        if dirid == ROOT_ID {
            match filename.as_slice() {
                b"search" => return Ok(SEARCH_DIR),
                b"index" => return Ok(INDEX_DIR),
                b"where" => return Ok(WHERE_DIR),
                b"agents" => return Ok(AGENTS_DIR),
                _ => {}
            }
        }

        // /agents level: any name becomes an agent container dir.
        if dirid == AGENTS_DIR {
            let agent_id = percent_decode(filename);
            let id = magic_file_id(CAT_AGENT, &agent_id);
            self.magic
                .register(id, MagicEntry::new(&agent_id, &agent_id, CAT_AGENT));
            return Ok(id);
        }

        // /agents/<id> level: only search/index/where subdirs.
        if is_magic(dirid) && magic_category(dirid) == CAT_AGENT {
            let agent_id = self
                .magic
                .get_entry(dirid)
                .map(|e| e.agent_id)
                .unwrap_or_default();
            let (sub, cat) = match filename.as_slice() {
                b"search" => ("search", crate::magic::CAT_AGENT_SEARCH),
                b"index" => ("index", crate::magic::CAT_AGENT_INDEX),
                b"where" => ("where", crate::magic::CAT_AGENT_WHERE),
                _ => return Err(nfsstat3::NFS3ERR_NOENT),
            };
            let scoped = scoped_name(&agent_id, sub);
            let id = magic_file_id(cat, &scoped);
            self.magic
                .register(id, MagicEntry::new(&agent_id, sub, cat));
            return Ok(id);
        }

        // Magic dir file lookup (global + agent-scoped).
        if let Some(cat) = self.dir_category(dirid) {
            let name = percent_decode(filename);
            let agent_id = self.agent_id_of_dir(dirid);
            let scoped = scoped_name(&agent_id, &name);
            let id = magic_file_id(cat, &scoped);
            self.magic
                .register(id, MagicEntry::new(&agent_id, &name, cat));
            return Ok(id);
        }

        self.real.lookup(dirid, filename).await
    }

    async fn getattr(&self, id: fileid3) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if !is_magic_dir(id) && magic_category(id) == CAT_INDEX {
                return Ok(self.index_attr(id).await);
            }
            return Ok(magic_attr(id));
        }
        self.real.getattr(id).await
    }

    async fn setattr(
        &self,
        id: fileid3,
        setattr: sattr3,
    ) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if !is_magic_dir(id) {
                match magic_category(id) {
                    CAT_SEARCH | CAT_WHERE => return Err(nfsstat3::NFS3ERR_ROFS),
                    CAT_INDEX => return Ok(self.index_attr(id).await),
                    _ => {}
                }
            }
            return Ok(magic_attr(id));
        }
        self.real.setattr(id, setattr).await
    }

    async fn read(
        &self,
        id: fileid3,
        offset: u64,
        count: u32,
    ) -> std::result::Result<(Vec<u8>, bool), nfsstat3> {
        if is_magic(id) {
            if is_magic_dir(id) {
                return Err(nfsstat3::NFS3ERR_ISDIR);
            }
            let cat = magic_category(id);
            let entry = match cat {
                CAT_INDEX => {
                    let (aid, name) = self
                        .index_name_for(id)
                        .await
                        .ok_or(nfsstat3::NFS3ERR_NOENT)?;
                    MagicEntry::new(&aid, &name, CAT_INDEX)
                }
                _ => self.magic.get_entry(id).ok_or(nfsstat3::NFS3ERR_NOENT)?,
            };
            let content = match cat {
                CAT_SEARCH => self.run_search(&entry.agent_id, &entry.name).await?,
                CAT_WHERE => self.run_where(&entry.agent_id, &entry.name).await?,
                CAT_INDEX => self.index_text(&entry.agent_id, &entry.name).await?,
                _ => Vec::new(),
            };
            return Ok(slice_read(&content, offset, count));
        }
        self.real.read(id, offset, count).await
    }

    async fn write(
        &self,
        id: fileid3,
        offset: u64,
        data: &[u8],
    ) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if is_magic_dir(id) {
                return Err(nfsstat3::NFS3ERR_ISDIR);
            }
            match magic_category(id) {
                CAT_INDEX => {
                    let (agent_id, key) = self
                        .index_name_for(id)
                        .await
                        .ok_or(nfsstat3::NFS3ERR_NOENT)?;
                    if !self.magic.has_buffer(id) {
                        if let Ok(Some(existing)) = self.db.get_text(&agent_id, &key).await {
                            self.magic.seed_buffer(id, existing.into_bytes());
                        }
                    }
                    let full = self.magic.write_into(id, offset, data);
                    let text = String::from_utf8(full).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
                    self.run_index(&agent_id, &key, &text).await?;
                    return Ok(self.index_attr(id).await);
                }
                CAT_SEARCH | CAT_WHERE => return Err(nfsstat3::NFS3ERR_ROFS),
                _ => return Ok(magic_attr(id)),
            }
        }
        self.real.write(id, offset, data).await
    }

    async fn create(
        &self,
        dirid: fileid3,
        filename: &filename3,
        attr: sattr3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        if let Some(cat) = self.dir_category(dirid) {
            let name = percent_decode(filename);
            let agent_id = self.agent_id_of_dir(dirid);
            let scoped = scoped_name(&agent_id, &name);
            let id = magic_file_id(cat, &scoped);
            self.magic
                .register(id, MagicEntry::new(&agent_id, &name, cat));
            return Ok((id, magic_attr(id)));
        }
        self.real.create(dirid, filename, attr).await
    }

    async fn create_exclusive(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<fileid3, nfsstat3> {
        if let Some(cat) = self.dir_category(dirid) {
            let name = percent_decode(filename);
            let agent_id = self.agent_id_of_dir(dirid);
            let scoped = scoped_name(&agent_id, &name);
            let id = magic_file_id(cat, &scoped);
            let exists = match cat {
                CAT_INDEX => self.db.exists(&agent_id, &name).await.unwrap_or(false),
                _ => self.magic.get_entry(id).is_some(),
            };
            if exists {
                return Err(nfsstat3::NFS3ERR_EXIST);
            }
            self.magic
                .register(id, MagicEntry::new(&agent_id, &name, cat));
            return Ok(id);
        }
        self.real.create_exclusive(dirid, filename).await
    }

    async fn mkdir(
        &self,
        dirid: fileid3,
        dirname: &filename3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        if is_magic(dirid) {
            return Err(nfsstat3::NFS3ERR_NOTSUPP);
        }
        self.real.mkdir(dirid, dirname).await
    }

    async fn remove(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<(), nfsstat3> {
        if let Some(cat) = self.dir_category(dirid) {
            let name = percent_decode(filename);
            let agent_id = self.agent_id_of_dir(dirid);
            let scoped = scoped_name(&agent_id, &name);
            let id = magic_file_id(cat, &scoped);
            if cat == CAT_INDEX {
                let _ = self.db.delete(&agent_id, &name).await;
            }
            self.magic.forget(id);
            return Ok(());
        }
        self.real.remove(dirid, filename).await
    }

    async fn rename(
        &self,
        from_dirid: fileid3,
        from_filename: &filename3,
        to_dirid: fileid3,
        to_filename: &filename3,
    ) -> std::result::Result<(), nfsstat3> {
        if is_magic(from_dirid) || is_magic(to_dirid) {
            return Err(nfsstat3::NFS3ERR_NOTSUPP);
        }
        self.real
            .rename(from_dirid, from_filename, to_dirid, to_filename)
            .await
    }

    async fn symlink(
        &self,
        dirid: fileid3,
        linkname: &filename3,
        symlink: &nfspath3,
        attr: &sattr3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        if is_magic(dirid) {
            return Err(nfsstat3::NFS3ERR_NOTSUPP);
        }
        self.real.symlink(dirid, linkname, symlink, attr).await
    }

    async fn readlink(&self, id: fileid3) -> std::result::Result<nfspath3, nfsstat3> {
        if is_magic(id) {
            return Err(nfsstat3::NFS3ERR_NOTSUPP);
        }
        self.real.readlink(id).await
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> std::result::Result<ReadDirResult, nfsstat3> {
        // /agents: list known agent directories from the DB.
        if dirid == AGENTS_DIR {
            let agents = self.db.list_agents().await.unwrap_or_default();
            let mut entries: Vec<DirEntry> = agents
                .into_iter()
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
            return Ok(ReadDirResult { entries, end });
        }

        // /agents/<id>: fixed listing of search/ index/ where/.
        if is_magic(dirid) && magic_category(dirid) == CAT_AGENT {
            let agent_id = self
                .magic
                .get_entry(dirid)
                .map(|e| e.agent_id)
                .unwrap_or_default();
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
            return Ok(ReadDirResult { entries, end });
        }

        // Magic dirs with files (global + agent-scoped).
        if let Some(cat) = self.dir_category(dirid) {
            let agent_id = self.agent_id_of_dir(dirid);
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
                return Ok(ReadDirResult { entries, end });
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
            return Ok(ReadDirResult { entries, end });
        }

        // Root: real entries first, then the magic dirs (including /agents).
        let mut res = self.real.readdir(dirid, start_after, max_entries).await?;
        if dirid == ROOT_ID && res.end {
            for (id, name) in [
                (SEARCH_DIR, b"search".as_slice()),
                (INDEX_DIR, b"index".as_slice()),
                (WHERE_DIR, b"where".as_slice()),
                (AGENTS_DIR, b"agents".as_slice()),
            ] {
                if id <= start_after {
                    continue;
                }
                if res.entries.len() >= max_entries {
                    res.end = false;
                    break;
                }
                res.entries.push(DirEntry {
                    fileid: id,
                    name: name.to_vec().into(),
                    attr: magic_attr(id),
                });
            }
        }
        Ok(res)
    }
}
