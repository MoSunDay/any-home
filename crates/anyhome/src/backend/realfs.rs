//! On-disk filesystem backend: a thin `tokio::fs` wrapper that implements
//! [`NFSFileSystem`] against a real directory.
//!
//! `fileid` == the underlying inode (via `MetadataExt::ino()`), except the root
//! directory is pinned to [`ROOT_ID`] (NFS reserves fileid 0). A small in-memory
//! map translates `fileid <-> path` so attribute/read/write calls can resolve a
//! path from the id the client holds. The map is process-local; NFS file handles
//! expire when the server restarts (the generation number changes), so that is
//! safe.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use nfsserve::fs_util::{exists_no_traverse, file_setattr, metadata_to_fattr3, path_setattr};
use nfsserve::nfs::*;
use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

/// Reserved fileid for the root directory. NFS forbids 0 (see `vfs.rs`).
pub const ROOT_ID: fileid3 = 1;

#[derive(Default)]
struct Maps {
    id_to_path: HashMap<fileid3, PathBuf>,
    path_to_id: HashMap<PathBuf, fileid3>,
}

/// A real-directory NFS backend. Clone shares the same id/path maps.
#[derive(Clone)]
pub struct RealFs {
    root: PathBuf,
    state: Arc<Mutex<Maps>>,
}

impl RealFs {
    /// Open the backend rooted at `root` (must exist and be a directory).
    pub fn new(root: PathBuf) -> Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            anyhow::bail!("fs_root is not a directory: {:?}", root);
        }
        let mut maps = Maps::default();
        maps.id_to_path.insert(ROOT_ID, root.clone());
        maps.path_to_id.insert(root.clone(), ROOT_ID);
        Ok(RealFs {
            root,
            state: Arc::new(Mutex::new(maps)),
        })
    }

    /// Absolute path of the root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- map helpers (lock briefly; never held across an await) ---

    fn path_of(&self, id: fileid3) -> std::result::Result<PathBuf, nfsstat3> {
        let maps = self.state.lock().unwrap();
        maps.id_to_path
            .get(&id)
            .cloned()
            .ok_or(nfsstat3::NFS3ERR_STALE)
    }

    /// Register a path under its real inode. Root is always ROOT_ID.
    fn register(&self, path: PathBuf) -> std::result::Result<fileid3, nfsstat3> {
        if path == self.root {
            return Ok(ROOT_ID);
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        self.register_with_meta(path, meta)
    }

    fn register_with_meta(
        &self,
        path: PathBuf,
        meta: std::fs::Metadata,
    ) -> std::result::Result<fileid3, nfsstat3> {
        if path == self.root {
            return Ok(ROOT_ID);
        }
        let mut maps = self.state.lock().unwrap();
        if let Some(&id) = maps.path_to_id.get(&path) {
            return Ok(id);
        }
        let id = meta.ino() as fileid3;
        maps.id_to_path.insert(id, path.clone());
        maps.path_to_id.insert(path, id);
        Ok(id)
    }

    /// fileid of the parent directory of `id` (root's parent is root).
    fn parent_id(&self, id: fileid3) -> std::result::Result<fileid3, nfsstat3> {
        let path = self.path_of(id)?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => self.root.clone(),
        };
        // Clamp anything that escapes the export root back to root.
        let parent = if parent.starts_with(&self.root) {
            parent
        } else {
            self.root.clone()
        };
        self.register(parent)
    }
}

#[async_trait]
impl NFSFileSystem for RealFs {
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
            return self.parent_id(dirid);
        }
        let dirpath = self.path_of(dirid)?;
        let child = dirpath.join(OsStr::from_bytes(filename));
        if !exists_no_traverse(&child) {
            return Err(nfsstat3::NFS3ERR_NOENT);
        }
        self.register(child)
    }

    async fn getattr(&self, id: fileid3) -> std::result::Result<fattr3, nfsstat3> {
        let path = self.path_of(id)?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(metadata_to_fattr3(id, &meta))
    }

    async fn setattr(
        &self,
        id: fileid3,
        setattr: sattr3,
    ) -> std::result::Result<fattr3, nfsstat3> {
        let path = self.path_of(id)?;
        path_setattr(&path, &setattr).await?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(metadata_to_fattr3(id, &meta))
    }

    async fn read(
        &self,
        id: fileid3,
        offset: u64,
        count: u32,
    ) -> std::result::Result<(Vec<u8>, bool), nfsstat3> {
        let path = self.path_of(id)?;
        let mut f = File::open(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_NOENT)?;
        let len = f
            .metadata()
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?
            .len();
        if offset >= len {
            return Ok((Vec::new(), true));
        }
        f.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let to_read = std::cmp::min(count as u64, len - offset) as usize;
        let mut buf = vec![0u8; to_read];
        f.read_exact(&mut buf)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let eof = offset + to_read as u64 >= len;
        Ok((buf, eof))
    }

    async fn write(
        &self,
        id: fileid3,
        offset: u64,
        data: &[u8],
    ) -> std::result::Result<fattr3, nfsstat3> {
        let path = self.path_of(id)?;
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        f.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        f.write_all(data)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let _ = f.flush().await;
        let meta = f
            .metadata()
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(metadata_to_fattr3(id, &meta))
    }

    async fn create(
        &self,
        dirid: fileid3,
        filename: &filename3,
        attr: sattr3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let path = dirpath.join(OsStr::from_bytes(filename));
        let file = std::fs::File::create(&path).map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let _ = file_setattr(&file, &attr).await;
        let id = self.register(path.clone())?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok((id, metadata_to_fattr3(id, &meta)))
    }

    async fn create_exclusive(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<fileid3, nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let path = dirpath.join(OsStr::from_bytes(filename));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| nfsstat3::NFS3ERR_EXIST)?;
        self.register(path)
    }

    async fn mkdir(
        &self,
        dirid: fileid3,
        dirname: &filename3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let path = dirpath.join(OsStr::from_bytes(dirname));
        tokio::fs::create_dir(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let id = self.register(path.clone())?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok((id, metadata_to_fattr3(id, &meta)))
    }

    async fn remove(
        &self,
        dirid: fileid3,
        filename: &filename3,
    ) -> std::result::Result<(), nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let path = dirpath.join(OsStr::from_bytes(filename));
        let meta = std::fs::symlink_metadata(&path).map_err(|_| nfsstat3::NFS3ERR_NOENT)?;
        let cached_id = {
            let maps = self.state.lock().unwrap();
            maps.path_to_id.get(&path).copied()
        };
        let res = if meta.is_dir() {
            tokio::fs::remove_dir(&path).await
        } else {
            tokio::fs::remove_file(&path).await
        };
        res.map_err(|_| nfsstat3::NFS3ERR_IO)?;
        if let Some(id) = cached_id {
            let mut maps = self.state.lock().unwrap();
            maps.id_to_path.remove(&id);
            maps.path_to_id.remove(&path);
        }
        Ok(())
    }

    async fn rename(
        &self,
        from_dirid: fileid3,
        from_filename: &filename3,
        to_dirid: fileid3,
        to_filename: &filename3,
    ) -> std::result::Result<(), nfsstat3> {
        let from_dir = self.path_of(from_dirid)?;
        let to_dir = self.path_of(to_dirid)?;
        let from_path = from_dir.join(OsStr::from_bytes(from_filename));
        let to_path = to_dir.join(OsStr::from_bytes(to_filename));
        tokio::fs::rename(&from_path, &to_path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        // Same inode: keep the id, just repoint the path mapping.
        let mut maps = self.state.lock().unwrap();
        if let Some(&id) = maps.path_to_id.get(&from_path) {
            maps.path_to_id.remove(&from_path);
            maps.path_to_id.insert(to_path.clone(), id);
            maps.id_to_path.insert(id, to_path);
        }
        Ok(())
    }

    async fn symlink(
        &self,
        dirid: fileid3,
        linkname: &filename3,
        symlink: &nfspath3,
        _attr: &sattr3,
    ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let path = dirpath.join(OsStr::from_bytes(linkname));
        tokio::fs::symlink(OsStr::from_bytes(symlink), &path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        let id = self.register(path.clone())?;
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok((id, metadata_to_fattr3(id, &meta)))
    }

    async fn readlink(&self, id: fileid3) -> std::result::Result<nfspath3, nfsstat3> {
        let path = self.path_of(id)?;
        let target = tokio::fs::read_link(&path)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?;
        Ok(target.as_os_str().as_bytes().into())
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> std::result::Result<ReadDirResult, nfsstat3> {
        let dirpath = self.path_of(dirid)?;
        let mut items: Vec<DirEntry> = Vec::new();
        let mut rd = tokio::fs::read_dir(&dirpath)
            .await
            .map_err(|_| nfsstat3::NFS3ERR_NOTDIR)?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|_| nfsstat3::NFS3ERR_IO)?
        {
            let name = entry.file_name();
            let name_bytes = name.as_bytes().to_vec();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            let child = entry.path();
            let meta = std::fs::symlink_metadata(&child).map_err(|_| nfsstat3::NFS3ERR_IO)?;
            let id = self.register_with_meta(child, meta.clone())?;
            items.push(DirEntry {
                fileid: id,
                name: name_bytes.into(),
                attr: metadata_to_fattr3(id, &meta),
            });
        }
        items.sort_by_key(|d| d.fileid);
        let mut filtered: Vec<DirEntry> = items
            .into_iter()
            .filter(|d| d.fileid > start_after)
            .collect();
        let end = filtered.len() <= max_entries;
        filtered.truncate(max_entries);
        Ok(ReadDirResult {
            entries: filtered,
            end,
        })
    }
}
