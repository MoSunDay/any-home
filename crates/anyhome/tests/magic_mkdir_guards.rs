//! Guards against real-FS shadowing of virtual dirs.
//!
//! Linux may skip the LOOKUP of the final component of mkdir(2) (GNU
//! `mkdir -p` relies on that), so MKDIR/CREATE can arrive for reserved
//! magic names without a prior lookup. The dispatcher must claim those
//! names as existing virtual dirs instead of leaking real entries under
//! fs-root (a real `agents/` dir would shadow `/agents` and route index
//! writes to disk instead of the DB).

use std::sync::Arc;

use nfsserve::nfs::{filename3, nfsstat3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::magic::{AGENTS_DIR, INDEX_DIR, SEARCH_DIR, WHERE_DIR};
use anyhome::{AnyHomeFs, MemoryDb, RealFs};

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

fn noattr() -> sattr3 {
    sattr3::default()
}

const EXIST: u32 = nfsstat3::NFS3ERR_EXIST as u32;
const INVAL: u32 = nfsstat3::NFS3ERR_INVAL as u32;
const NOTSUPP: u32 = nfsstat3::NFS3ERR_NOTSUPP as u32;

/// Build an `AnyHomeFs` backed by a temp dir + in-memory DB + dummy embedder.
/// Returns the on-disk root path too (for contamination setups).
async fn make() -> (AnyHomeFs, std::path::PathBuf) {
    let root = tempdir().expect("tempdir").keep();
    let real = RealFs::new(root.clone()).unwrap();
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
    (AnyHomeFs::new(real, db, emb), root)
}

#[tokio::test]
async fn mkdir_reserved_root_names_claims_existing() {
    let (fs, _) = make().await;
    let root = fs.root_dir();
    for name in [b"search".as_slice(), b"index", b"where", b"agents", b"search_k5"] {
        let err = fs.mkdir(root, &nm(name)).await.unwrap_err();
        assert_eq!(err as u32, EXIST, "mkdir root/{name:?} must claim EXIST");
    }
}

#[tokio::test]
async fn mkdir_real_name_at_root_still_creates_on_disk() {
    let (fs, path) = make().await;
    let root = fs.root_dir();
    let (id, attr) = fs.mkdir(root, &nm(b"plain")).await.expect("real mkdir");
    assert!(path.join("plain").is_dir());
    assert_eq!(fs.lookup(root, &nm(b"plain")).await.unwrap(), id);
    assert_eq!(attr.fileid, id);
}

#[tokio::test]
async fn mkdir_agent_and_subdirs_claim_existing() {
    let (fs, _) = make().await;
    let agents = fs.lookup(fs.root_dir(), &nm(b"agents")).await.unwrap();
    assert_eq!(agents, AGENTS_DIR);

    // Agent container dirs are virtual: EXIST, no auth bypass needed.
    let err = fs.mkdir(agents, &nm(b"botA")).await.unwrap_err();
    assert_eq!(err as u32, EXIST);

    let bot_a = fs.lookup(agents, &nm(b"botA")).await.unwrap();
    for name in [b"search".as_slice(), b"index", b"where", b"search_k3"] {
        let err = fs.mkdir(bot_a, &nm(name)).await.unwrap_err();
        assert_eq!(err as u32, EXIST, "mkdir botA/{name:?} must claim EXIST");
    }
    // Non-canonical names under an agent dir stay unsupported.
    let err = fs.mkdir(bot_a, &nm(b"bogus")).await.unwrap_err();
    assert_eq!(err as u32, NOTSUPP);
}

#[tokio::test]
async fn create_and_remove_reserved_root_names_are_blocked() {
    let (fs, path) = make().await;
    let root = fs.root_dir();

    let err = fs.create(root, &nm(b"agents"), noattr()).await.unwrap_err();
    assert_eq!(err as u32, EXIST);
    let err = fs.create_exclusive(root, &nm(b"where")).await.unwrap_err();
    assert_eq!(err as u32, EXIST);
    let err = fs.remove(root, &nm(b"search")).await.unwrap_err();
    assert_eq!(err as u32, INVAL);
    assert!(!path.join("agents").exists());
}

#[tokio::test]
async fn rename_and_symlink_reserved_root_names_are_blocked() {
    let (fs, _) = make().await;
    let root = fs.root_dir();

    // seed a real file to rename
    fs.create(root, &nm(b"victim"), noattr()).await.unwrap();
    let err = fs
        .rename(root, &nm(b"victim"), root, &nm(b"index"))
        .await
        .unwrap_err();
    assert_eq!(err as u32, EXIST);
    let err = fs
        .rename(root, &nm(b"agents"), root, &nm(b"victim"))
        .await
        .unwrap_err();
    assert_eq!(err as u32, INVAL);

    let err = fs
        .symlink(root, &nm(b"search"), &b"target".to_vec().into(), &noattr())
        .await
        .unwrap_err();
    assert_eq!(err as u32, EXIST);
}

#[tokio::test]
async fn readdir_root_hides_shadowed_real_entries() {
    let (fs, path) = make().await;
    let root = fs.root_dir();

    // Simulate contamination from an old buggy server: a real `agents` dir
    // on disk under fs-root.
    std::fs::create_dir(path.join("agents")).unwrap();

    let res = fs.readdir(root, 0, 64).await.unwrap();
    let names: Vec<Vec<u8>> = res.entries.iter().map(|e| e.name.clone().0).collect();
    let count = names.iter().filter(|n| *n == b"agents").count();
    assert_eq!(count, 1, "agents must appear exactly once, got {names:?}");

    // lookup still resolves the virtual dir.
    assert_eq!(fs.lookup(root, &nm(b"agents")).await.unwrap(), AGENTS_DIR);
}

#[tokio::test]
async fn mkdir_p_flow_lands_index_write_in_db() {
    // Mirrors the kernel flow of `mkdir -p /agents/botA/index` followed by
    // `echo text > /agents/botA/index/greeting`: MKDIRs may arrive without
    // prior LOOKUPs; each EXIST forces a re-resolve through lookup().
    let (fs, path) = make().await;
    let root = fs.root_dir();

    assert!(fs.mkdir(root, &nm(b"agents")).await.is_err());
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    assert!(fs.mkdir(agents, &nm(b"botA")).await.is_err());
    let bot_a = fs.lookup(agents, &nm(b"botA")).await.unwrap();
    assert!(fs.mkdir(bot_a, &nm(b"index")).await.is_err());
    let index_dir = fs.lookup(bot_a, &nm(b"index")).await.unwrap();

    let (gid, _) = fs
        .create(index_dir, &nm(b"greeting"), noattr())
        .await
        .expect("magic create");
    fs.write(gid, 0, b"agent memory one").await.unwrap();

    // No real dirs leaked under fs-root.
    assert!(!path.join("agents").exists());

    // Agent-scoped where resolves the memory from the DB.
    let where_dir = fs.lookup(bot_a, &nm(b"where")).await.unwrap();
    let q = fs.lookup(where_dir, &nm(b"agent")).await.unwrap();
    let (data, _eof) = fs.read(q, 0, 4096).await.unwrap();
    let out = String::from_utf8(data).unwrap();
    assert!(out.contains("greeting"), "where/agent rows: {out:?}");

    // Global surfaces keep working.
    assert_eq!(fs.lookup(root, &nm(b"search")).await.unwrap(), SEARCH_DIR);
    assert_eq!(fs.lookup(root, &nm(b"index")).await.unwrap(), INDEX_DIR);
    assert_eq!(fs.lookup(root, &nm(b"where")).await.unwrap(), WHERE_DIR);
}
