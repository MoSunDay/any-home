//! The dispatcher: an [`NFSFileSystem`] that routes each call either to the
//! real on-disk backend ([`RealFs`]) or to the DB layer via magic paths.
//!
//! ## Global magic paths (backward-compatible, default agent `agent_id = ""`)
//! - `/search/<query>` — read returns KNN results for `<query>` as text
//!   (top-k = 16, [`DEFAULT_K`]).
//! - `/search_k<N>/<query>` — like `/search` with an explicit top-k
//!   (`N` clamped to 1..=64). Not enumerated by readdir.
//! - `/index/<key>`    — write embeds and upserts the written text under `<key>`.
//! - `/where/<cond>`   — read returns FTS5 full-text matches, or `key:<prefix>`.
//!   A malformed FTS expression yields a first line `# error: invalid FTS
//!   query ...` instead of an opaque I/O error.
//!
//! ## Agent-scoped magic paths (multi-agent namespace isolation)
//! - `/agents/<id>/search/<query>` — KNN scoped to agent `<id>`.
//! - `/agents/<id>/search_k<N>/<query>` — explicit top-k, scoped.
//! - `/agents/<id>/index/<key>`    — write scoped to agent `<id>`.
//! - `/agents/<id>/where/<cond>`   — FTS/prefix scoped to agent `<id>`.
//!
//! Agents are created implicitly on first write. `/agents` readdir lists known
//! agents via `SELECT DISTINCT agent_id`.

use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::*;

use crate::auth::{AuthPolicy, CallerCred};
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::backend::{RealFs, ROOT_ID};
use crate::db::{Embedder, MemoryDb, WhereOutcome};
use crate::magic::{
    percent_decode, slice_read, magic_attr, magic_attr_sized, magic_category, magic_file_id,
    scoped_name, agent_dir_base_cat, is_magic, is_magic_dir,
    parse_search_k, search_k_dir_id, search_scoped_name,
    MagicEntry, MagicRegistry,
    CAT_AGENT, CAT_INDEX, CAT_SEARCH, CAT_SEARCH_K_DIR, CAT_WHERE, DEFAULT_K,
    INDEX_DIR, SEARCH_DIR, WHERE_DIR, AGENTS_DIR,
};

/// The dispatcher filesystem: real FS + DB, joined by magic paths.
pub struct AnyHomeFs {
    pub real: RealFs,
    pub db: MemoryDb,
    pub embedder: Arc<dyn Embedder>,
    magic: MagicRegistry,
    auth: AuthPolicy,
}

impl AnyHomeFs {
    pub fn new(real: RealFs, db: MemoryDb, embedder: Arc<dyn Embedder>) -> Self {
        AnyHomeFs {
            real,
            db,
            embedder,
            magic: MagicRegistry::default(),
            auth: AuthPolicy::default(),
        }
    }

    /// Attach an authorization policy. When omitted the policy stays
    /// [`crate::auth::AuthPolicy::Open`] (auth disabled, identical to
    /// init-pro-4) so existing callers/tests are unaffected.
    pub fn with_auth(mut self, auth: AuthPolicy) -> Self {
        self.auth = auth;
        self
    }

    /// The AUTH_SYS credential of the current RPC caller, surfaced from the
    /// RPC layer's task-local. Anonymous (`uid == 0`) when no AUTH_SYS
    /// credential was supplied or when running outside the server (unit tests).
    fn current_caller(&self) -> CallerCred {
        CallerCred {
            uid: nfsserve::identity::current_identity().uid,
        }
    }

    /// Reject the call unless the current caller may act as `agent_id`.
    /// Returns [`nfsstat3::NFS3ERR_ACCES`] on impersonation. A no-op under
    /// [`crate::auth::AuthPolicy::Open`] and for the default agent (`""`).
    fn require_agent(&self, agent_id: &str) -> std::result::Result<(), nfsstat3> {
        if self.auth.permits(self.current_caller(), agent_id) {
            Ok(())
        } else {
            Err(nfsstat3::NFS3ERR_ACCES)
        }
    }

    // ---- DB-backed operations (all scoped by agent_id) ----

    /// Embed `query` and return up to `k` KNN rows as `<key>\t<text>\n`.
    /// `k` comes from the entry ([`DEFAULT_K`] for plain `/search`).
    async fn run_search(
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

    /// Full-text / prefix query for `/where/<cond>` -> `<key>\t<text>\n`
    /// lines. A malformed FTS condition renders as a single first line
    /// `# error: invalid FTS query ...`: NFS READ has no errno channel for
    /// per-call syntax problems, and a text marker lets agents distinguish
    /// "no results" from "query written wrong" (valid queries are unchanged).
    async fn run_where(
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
    fn magic_file_in(
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
            // `/search_k<N>`: same KNN surface with an explicit top-k.
            if let Some(k) = parse_search_k(filename.as_slice()) {
                let id = search_k_dir_id("", k);
                self.magic
                    .register(id, MagicEntry::with_k("", "search", CAT_SEARCH_K_DIR, k));
                return Ok(id);
            }
        }

        // /agents level: any name becomes an agent container dir.
        // Authorization gate: the caller must be permitted to act as this agent.
        if dirid == AGENTS_DIR {
            let agent_id = percent_decode(filename);
            self.require_agent(&agent_id)?;
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
            self.require_agent(&agent_id)?;
            if let Some(k) = parse_search_k(filename.as_slice()) {
                let id = search_k_dir_id(&agent_id, k);
                self.magic.register(
                    id,
                    MagicEntry::with_k(&agent_id, "search", CAT_SEARCH_K_DIR, k),
                );
                return Ok(id);
            }
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

        // Magic dir file lookup (global + agent-scoped + search_k).
        if let Some(cat) = self.dir_category(dirid) {
            let name = percent_decode(filename);
            let agent_id = self.agent_id_of_dir(dirid);
            self.require_agent(&agent_id)?;
            let (id, entry) = self.magic_file_in(dirid, &agent_id, &name, cat);
            self.magic.register(id, entry);
            return Ok(id);
        }

        self.real.lookup(dirid, filename).await
    }

    async fn getattr(&self, id: fileid3) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if !is_magic_dir(id) && magic_category(id) == CAT_INDEX {
                // Size-oracle closure (init-pro-7): the byte size of an
                // agent's memory is itself protected data — an unauthenticated
                // `getattr` on a leaked/guessed fileid used to reveal the real
                // stored length. Gate it exactly like read/write. The default
                // agent stays public; directories and real-FS metadata are
                // not intercepted.
                if let Some((agent_id, _)) = self.index_name_for(id).await {
                    self.require_agent(&agent_id)?;
                    return Ok(self.index_attr(id).await);
                }
                // Unresolvable fileid (stale handle / never registered): keep
                // reporting a zero-sized regular file instead of ACCES, so
                // restart semantics stay identical to init-pro-6.
                return Ok(magic_attr_sized(id, 0));
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
                    CAT_INDEX => {
                        // Same gate as getattr: metadata of an agent's index
                        // file must not be reachable by unauthenticated callers.
                        if let Some((agent_id, _)) = self.index_name_for(id).await {
                            self.require_agent(&agent_id)?;
                            return Ok(self.index_attr(id).await);
                        }
                        return Ok(magic_attr_sized(id, 0));
                    }
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
            self.require_agent(&entry.agent_id)?;
            let content = match cat {
                CAT_SEARCH => self.run_search(&entry.agent_id, &entry.name, entry.k).await?,
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
                    self.require_agent(&agent_id)?;
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
            self.require_agent(&agent_id)?;
            let (id, entry) = self.magic_file_in(dirid, &agent_id, &name, cat);
            self.magic.register(id, entry);
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
            self.require_agent(&agent_id)?;
            let (id, entry) = self.magic_file_in(dirid, &agent_id, &name, cat);
            let exists = match cat {
                CAT_INDEX => self.db.exists(&agent_id, &name).await.unwrap_or(false),
                _ => self.magic.get_entry(id).is_some(),
            };
            if exists {
                return Err(nfsstat3::NFS3ERR_EXIST);
            }
            self.magic.register(id, entry);
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
            self.require_agent(&agent_id)?;
            let (id, _) = self.magic_file_in(dirid, &agent_id, &name, cat);
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
            return Ok(ReadDirResult { entries, end });
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
            return Ok(ReadDirResult { entries, end });
        }

        // /search_k<N> dirs: synthetic variants, deliberately not enumerable
        // (consistent with the DB layer's non-enumerable query surface).
        if is_magic(dirid) && magic_category(dirid) == CAT_SEARCH_K_DIR {
            return Ok(ReadDirResult {
                entries: Vec::new(),
                end: true,
            });
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
