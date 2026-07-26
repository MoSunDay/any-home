//! The dispatcher: an [`NFSFileSystem`] that routes each call either to the
//! real on-disk backend ([`RealFs`]) or to the DB layer via magic paths.
//!
//! - `/search/<query>` — read returns KNN results for `<query>` as text.
//! - `/index/<key>`    — write embeds and upserts the written text under `<key>`.
//! - `/where/<cond>`   — read returns FTS5 full-text matches, or `key:<prefix>` key-filter.

use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::backend::{RealFs, ROOT_ID};
use crate::db::{Embedder, MemoryDb};
use crate::magic::{
    percent_decode, slice_read, magic_attr, magic_attr_sized, magic_category, magic_file_id,
    is_magic, is_magic_dir, MagicRegistry, CAT_INDEX, CAT_SEARCH, CAT_WHERE, DEFAULT_K,
    INDEX_DIR, SEARCH_DIR, WHERE_DIR,
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

    /// Embed `query` and return up to [`DEFAULT_K`] KNN rows as `<id>\t<text>\n`.
    async fn run_search(&self, query: &str) -> std::result::Result<Vec<u8>, nfsstat3> {
        let emb = self
            .embedder
            .embed(query)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let rows = self
            .db
            .search(&emb, DEFAULT_K)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut out = String::new();
        for (key, text) in rows {
            out.push_str(&format!("{}\t{}\n", key, text));
        }
        Ok(out.into_bytes())
    }

    /// Embed `text` and upsert it under `key` (MVP: each write is an upsert).
    async fn run_index(&self, key: &str, text: &str) -> std::result::Result<(), nfsstat3> {
        let emb = self
            .embedder
            .embed(text)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        self.db
            .upsert(key, text, &emb)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)
    }

    /// Full-text / prefix query for `/where/<cond>` -> `<key>\t<text>\n` lines.
    async fn run_where(&self, cond: &str) -> std::result::Result<Vec<u8>, nfsstat3> {
        let rows = self
            .db
            .where_search(cond, DEFAULT_K)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let mut out = String::new();
        for (key, text) in rows {
            out.push_str(&format!("{}\t{}\n", key, text));
        }
        Ok(out.into_bytes())
    }

    /// Read back the stored text for an indexed key (so `/index/<key>` round-trips
    /// and survives a restart, since the registry is rebuilt from the DB).
    async fn index_text(&self, key: &str) -> std::result::Result<Vec<u8>, nfsstat3> {
        match self.db.get_text(key).await {
            Ok(Some(text)) => Ok(text.into_bytes()),
            Ok(None) => Ok(format!("# no indexed memory under key: {}\n", key).into_bytes()),
            Err(_) => Err(nfsstat3::NFS3ERR_IO),
        }
    }

    /// Resolve the decoded name for an `/index` fileid, recovering it from the
    /// DB when the in-memory registry has no entry (e.g. after a restart, before
    /// the client re-`lookup`s). The recovered name is cached so later calls are
    /// O(1). `/search`/`/where` names are the query itself with no DB backing, so
    /// they cannot be recovered this way (they require a fresh `lookup`, which is
    /// standard NFS reconnect behavior).
    async fn index_name_for(&self, id: fileid3) -> Option<String> {
        if let Some(name) = self.magic.get(id) {
            return Some(name);
        }
        let keys = self.db.list_keys().await.ok()?;
        for key in keys {
            if magic_file_id(CAT_INDEX, &key) == id {
                self.magic.register(id, key.clone());
                return Some(key);
            }
        }
        None
    }

    /// Attributes for an `/index` file reflecting its real stored byte size.
    async fn index_attr(&self, id: fileid3) -> fattr3 {
        let size = match self.index_name_for(id).await {
            Some(key) => self
                .db
                .get_text(&key)
                .await
                .map(|t| t.map(|s| s.len() as u64).unwrap_or(0))
                .unwrap_or(0),
            None => 0,
        };
        magic_attr_sized(id, size)
    }
}

fn dir_category(dirid: fileid3) -> Option<u8> {
    match dirid {
        SEARCH_DIR => Some(CAT_SEARCH),
        INDEX_DIR => Some(CAT_INDEX),
        WHERE_DIR => Some(CAT_WHERE),
        _ => None,
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
        if dirid == ROOT_ID {
            match filename.as_slice() {
                b"search" => return Ok(SEARCH_DIR),
                b"index" => return Ok(INDEX_DIR),
                b"where" => return Ok(WHERE_DIR),
                _ => {}
            }
        }
        if filename.as_slice() == b"." {
            return Ok(dirid);
        }
        if filename.as_slice() == b".." {
            return Ok(if is_magic(dirid) {
                ROOT_ID
            } else {
                self.real.lookup(dirid, &b"..".to_vec().into()).await?
            });
        }
        if let Some(cat) = dir_category(dirid) {
            let name = percent_decode(filename);
            let id = magic_file_id(cat, &name);
            self.magic.register(id, name);
            return Ok(id);
        }
        self.real.lookup(dirid, filename).await
    }

    async fn getattr(&self, id: fileid3) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if !is_magic_dir(id) && magic_category(id) == CAT_INDEX {
                // `/index` files report their real byte size so `stat`/`ls -l`
                // match the content read back. `/search` and `/where` are
                // dynamic per query and keep the virtual ceiling (magic_attr).
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
            // `/search` and `/where` are read-only query views: mutating their
            // attributes is rejected (ROFS) rather than silently ignored.
            // `/index` accepts (ignores) setattr for clients that chmod/truncate
            // before writing, and reports its real size.
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
            let name = match cat {
                // `/index` names are recoverable from the DB on a registry miss.
                CAT_INDEX => self.index_name_for(id).await.ok_or(nfsstat3::NFS3ERR_NOENT)?,
                _ => self.magic.get(id).ok_or(nfsstat3::NFS3ERR_NOENT)?,
            };
            let content = match cat {
                CAT_SEARCH => self.run_search(&name).await?,
                CAT_WHERE => self.run_where(&name).await?,
                CAT_INDEX => self.index_text(&name).await?,
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
                    let key = self.index_name_for(id).await.ok_or(nfsstat3::NFS3ERR_NOENT)?;
                    // Compose positional WRITEs into one buffer: a single `echo`
                    // or large `cp` arrives as multiple chunks at increasing
                    // offsets, which would otherwise overwrite each other. Seed
                    // from the DB on the first write so writes compose with the
                    // existing content.
                    if !self.magic.has_buffer(id) {
                        if let Ok(Some(existing)) = self.db.get_text(&key).await {
                            self.magic.seed_buffer(id, existing.into_bytes());
                        }
                    }
                    let full = self.magic.write_into(id, offset, data);
                    // The DB `text` column requires valid UTF-8: reject rather
                    // than silently corrupt via lossy replacement.
                    let text = String::from_utf8(full).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
                    self.run_index(&key, &text).await?;
                    return Ok(self.index_attr(id).await);
                }
                // `/search` and `/where` are read-only query views.
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
        if let Some(cat) = dir_category(dirid) {
            let name = percent_decode(filename);
            let id = magic_file_id(cat, &name);
            self.magic.register(id, name);
            return Ok((id, magic_attr(id)));
        }
        self.real.create(dirid, filename, attr).await
    }

    async fn create_exclusive(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<fileid3, nfsstat3> {
        if let Some(cat) = dir_category(dirid) {
            let name = percent_decode(filename);
            let id = magic_file_id(cat, &name);
            // Honor the exclusive (O_EXCL) contract: fail if the target already
            // exists. `/index` checks the durable DB; the query views check the
            // in-memory registry.
            let exists = match cat {
                CAT_INDEX => self.db.exists(&name).await.unwrap_or(false),
                _ => self.magic.get(id).is_some(),
            };
            if exists {
                return Err(nfsstat3::NFS3ERR_EXIST);
            }
            self.magic.register(id, name);
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
        if let Some(cat) = dir_category(dirid) {
            let name = percent_decode(filename);
            let id = magic_file_id(cat, &name);
            if cat == CAT_INDEX {
                let _ = self.db.delete(&name).await;
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
        if let Some(cat) = dir_category(dirid) {
            // `/index` is durable storage: rebuild its listing from the DB with
            // real byte sizes so a restart (which clears the in-memory registry)
            // still shows history with accurate attributes.
            if cat == CAT_INDEX {
                let mut sized: Vec<(fileid3, String, u64)> = self
                    .db
                    .list_with_size()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, sz)| (magic_file_id(CAT_INDEX, &key), key, sz))
                    .collect();
                for (id, key, _) in &sized {
                    self.magic.register(*id, key.clone());
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
            // `/search`, `/where`: in-memory registry listing (virtual sizes).
            let mut rows = self.magic.list_for_cat(cat);
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

        // Root: real entries first, then the three magic dirs (ids sort last).
        let mut res = self.real.readdir(dirid, start_after, max_entries).await?;
        if dirid == ROOT_ID && res.end {
            for (id, name) in [
                (SEARCH_DIR, b"search".as_slice()),
                (INDEX_DIR, b"index".as_slice()),
                (WHERE_DIR, b"where".as_slice()),
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
