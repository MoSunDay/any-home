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
//!
//! ## Layout (init-pro-8 TODO-47)
//! This file keeps the struct, auth helpers, and the full
//! [`NFSFileSystem`] trait impl. Inherent-impl helpers live in submodules:
//! [`dbops`] (DB-backed operations), [`routing`] (fileid/dir resolution),
//! [`readdir`] (magic-namespace directory listing).

use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::*;

use crate::auth::{AuthPolicy, CallerCred};
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::backend::{RealFs, ROOT_ID};
use crate::db::{Embedder, MemoryDb};
use crate::magic::{
    percent_decode, slice_read, magic_attr, magic_attr_sized, magic_category, magic_file_id,
    scoped_name, is_magic, is_magic_dir,
    parse_search_k, search_k_dir_id,
    MagicEntry, MagicRegistry,
    CAT_AGENT, CAT_INDEX, CAT_SEARCH, CAT_SEARCH_K_DIR, CAT_WHERE,
    INDEX_DIR, SEARCH_DIR, WHERE_DIR, AGENTS_DIR,
};

mod dbops;
mod readdir;
mod routing;

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
        // Magic namespace: handled by the readdir submodule; Ok(None) falls
        // through to the real FS below.
        if let Some(res) = self.magic_readdir(dirid, start_after, max_entries).await? {
            return Ok(res);
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
