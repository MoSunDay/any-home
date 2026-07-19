//! The dispatcher: an [`NFSFileSystem`] that routes each call either to the
//! real on-disk backend ([`RealFs`]) or to the DB layer via magic paths.
//!
//! - `/search/<query>` — read returns KNN results for `<query>` as text.
//! - `/index/<key>`    — write embeds and upserts the written text under `<key>`.
//! - `/where/<cond>`   — reserved (full-text/filter; not implemented yet).

use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};

use crate::backend::{RealFs, ROOT_ID};
use crate::db::{Embedder, MemoryDb};
use crate::magic::{
    percent_decode, slice_read, magic_attr, magic_category, magic_file_id, is_magic, is_magic_dir,
    MagicRegistry, CAT_INDEX, CAT_SEARCH, CAT_WHERE, DEFAULT_K, INDEX_DIR, SEARCH_DIR, WHERE_DIR,
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
            let name = self.magic.get(id).ok_or(nfsstat3::NFS3ERR_NOENT)?;
            let content = match cat {
                CAT_SEARCH => self.run_search(&name).await?,
                CAT_WHERE => {
                    format!("# /where/{} (not implemented; use /search)\n", name).into_bytes()
                }
                CAT_INDEX => format!("# index key: {}\n", name).into_bytes(),
                _ => Vec::new(),
            };
            return Ok(slice_read(&content, offset, count));
        }
        self.real.read(id, offset, count).await
    }

    async fn write(
        &self,
        id: fileid3,
        _offset: u64,
        data: &[u8],
    ) -> std::result::Result<fattr3, nfsstat3> {
        if is_magic(id) {
            if is_magic_dir(id) {
                return Err(nfsstat3::NFS3ERR_ISDIR);
            }
            if magic_category(id) == CAT_INDEX {
                let key = self.magic.get(id).ok_or(nfsstat3::NFS3ERR_NOENT)?;
                let text = String::from_utf8_lossy(data).into_owned();
                self.run_index(&key, &text).await?;
            }
            return Ok(magic_attr(id));
        }
        self.real.write(id, _offset, data).await
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
