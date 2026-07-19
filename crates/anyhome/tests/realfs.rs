//! RealFs behavior via the NFSFileSystem trait: create/write/read/mkdir/readdir/
//! remove/rename against a real tempdir.

use std::sync::Arc;

use nfsserve::nfs::{filename3, nfsstat3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::RealFs;

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

#[tokio::test]
async fn root_is_real_and_rootdir_is_root_id() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    assert_eq!(fs.root_dir(), 1, "ROOT_ID == 1");
    assert!(fs.root().is_dir());
}

#[tokio::test]
async fn create_write_read_roundtrip() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    let root = fs.root_dir();

    let (id, attr) = fs.create(root, &nm(b"hello.txt"), sattr3::default()).await.unwrap();
    assert_eq!(attr.size, 0, "newly created file is empty");

    let payload = b"hello anyhome";
    fs.write(id, 0, payload).await.unwrap();
    let attr = fs.getattr(id).await.unwrap();
    assert_eq!(attr.size as usize, payload.len(), "size reflects written bytes");

    let (data, _eof) = fs.read(id, 0, 4096).await.unwrap();
    assert_eq!(data, payload);
}

#[tokio::test]
async fn lookup_finds_created_file() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    let root = fs.root_dir();

    let (created_id, _) = fs.create(root, &nm(b"look.txt"), sattr3::default()).await.unwrap();
    let looked_id = fs.lookup(root, &nm(b"look.txt")).await.unwrap();
    assert_eq!(created_id, looked_id, "lookup returns the create fileid");

    let err = fs.lookup(root, &nm(b"missing")).await.unwrap_err();
    assert!(matches!(err, nfsstat3::NFS3ERR_NOENT));
}

#[tokio::test]
async fn mkdir_readdir_remove() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    let root = fs.root_dir();

    let (sub_id, _) = fs.mkdir(root, &nm(b"sub")).await.unwrap();
    let (f_id, _) = fs.create(root, &nm(b"f.txt"), sattr3::default()).await.unwrap();

    // Root readdir lists both entries.
    let res = fs.readdir(root, 0, 64).await.unwrap();
    let names: Vec<Vec<u8>> = res.entries.iter().map(|e| e.name.clone().0).collect();
    assert!(names.iter().any(|n| n == b"sub"), "readdir has sub");
    assert!(names.iter().any(|n| n == b"f.txt"), "readdir has f.txt");

    // Remove the file.
    fs.remove(root, &nm(b"f.txt")).await.unwrap();
    assert!(matches!(fs.lookup(root, &nm(b"f.txt")).await.unwrap_err(), nfsstat3::NFS3ERR_NOENT));

    // remove() purges the id/path map, so the old fileid is now STALE.
    assert!(matches!(fs.getattr(f_id).await.unwrap_err(), nfsstat3::NFS3ERR_STALE));
    let _ = sub_id;
}

#[tokio::test]
async fn rename_moves_a_file() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    let root = fs.root_dir();

    fs.create(root, &nm(b"old.txt"), sattr3::default()).await.unwrap();
    fs.rename(root, &nm(b"old.txt"), root, &nm(b"new.txt")).await.unwrap();

    assert!(matches!(fs.lookup(root, &nm(b"old.txt")).await.unwrap_err(), nfsstat3::NFS3ERR_NOENT));
    let _ = fs.lookup(root, &nm(b"new.txt")).await.unwrap();
}

#[tokio::test]
async fn realfs_is_clone_and_shares_state() {
    let dir = tempdir().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf()).unwrap();
    let fs2 = fs.clone();
    let root = fs.root_dir();
    let (id, _) = fs.create(root, &nm(b"shared.txt"), sattr3::default()).await.unwrap();
    // The clone can resolve the fileid created via the original.
    let attr = fs2.getattr(id).await.unwrap();
    assert_eq!(attr.fileid, id);
    // Arc count sanity: both clones reference the same maps.
    let _: Arc<_> = Arc::new(()); // placeholder to keep Arc import meaningful
}
