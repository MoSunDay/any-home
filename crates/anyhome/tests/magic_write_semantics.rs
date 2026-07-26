//! Magic-path write/handle semantics (init-pro-3, TODO-14..19):
//! positional WRITE composition, exclusive create, read-only views, real
//! attribute sizes, restart-surviving handles, and UTF-8 safety.

use std::sync::Arc;

use nfsserve::nfs::{filename3, nfsstat3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, MemoryDb, RealFs};

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

async fn make() -> AnyHomeFs {
    let root = tempdir().expect("tempdir").keep();
    let real = RealFs::new(root).unwrap();
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
    AnyHomeFs::new(real, db, emb)
}

async fn index_doc(fs: &AnyHomeFs, key: &str, text: &[u8]) {
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(key.as_bytes())).await.unwrap();
    fs.write(id, 0, text).await.unwrap();
}

// ---- TODO-14: positional WRITE composition ----

#[tokio::test]
async fn index_write_appends_at_offset() {
    let fs = make().await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"greet")).await.unwrap();

    // Two writes at consecutive offsets must compose, not overwrite.
    fs.write(id, 0, b"Hello").await.unwrap();
    fs.write(id, 5, b", world").await.unwrap();

    let stored = fs.db.get_text("", "greet").await.unwrap().unwrap();
    assert_eq!(stored, "Hello, world");
}

#[tokio::test]
async fn index_write_composes_many_chunks() {
    let fs = make().await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"big")).await.unwrap();

    // A large write delivered as many small WRITE RPCs at increasing offsets
    // (how the NFS client fragments a single `echo`/`cp`).
    let chunk = b"abcdefghij"; // 10 bytes
    let repeats = 200; // 2000 bytes total (>= 2x any single payload)
    let mut offset = 0u64;
    for _ in 0..repeats {
        fs.write(id, offset, chunk).await.unwrap();
        offset += chunk.len() as u64;
    }

    let expected: Vec<u8> = (0..repeats).flat_map(|_| chunk.iter().copied()).collect();
    // The DB stores the composed full text byte-for-byte.
    let stored = fs.db.get_text("", "big").await.unwrap().unwrap();
    assert_eq!(stored.as_bytes(), expected.as_slice());
    // And a full read returns it with EOF.
    let (data, eof) = fs.read(id, 0, expected.len() as u32 + 100).await.unwrap();
    assert_eq!(data, expected);
    assert!(eof);
}

#[tokio::test]
async fn index_write_composes_with_existing_content() {
    let fs = make().await;
    // Seed content, then open a fresh handle (simulating reopen) and append.
    index_doc(&fs, "log", b"line one\n").await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"log")).await.unwrap();
    // Write past the existing end: the buffer is seeded from the DB first.
    fs.write(id, "line one\n".len() as u64, b"line two").await.unwrap();
    let stored = fs.db.get_text("", "log").await.unwrap().unwrap();
    assert_eq!(stored, "line one\nline two");
}

// ---- TODO-15: exclusive create honors existence ----

#[tokio::test]
async fn index_create_exclusive_conflicts_with_existing() {
    let fs = make().await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();

    // Create + write so the key is durably present in the DB.
    let (id, _) = fs.create(index_dir, &nm(b"dup"), sattr3::default()).await.unwrap();
    fs.write(id, 0, b"first").await.unwrap();

    // A second exclusive create of the same key must fail (EXISTS).
    let err = fs.create_exclusive(index_dir, &nm(b"dup")).await.unwrap_err();
    assert_eq!((err) as u32, nfsstat3::NFS3ERR_EXIST as u32);

    // A fresh key still succeeds.
    fs.create_exclusive(index_dir, &nm(b"new")).await.unwrap();
}

#[tokio::test]
async fn search_create_exclusive_conflicts_after_lookup() {
    let fs = make().await;
    let root = fs.root_dir();
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    fs.lookup(search_dir, &nm(b"q")).await.unwrap(); // registers "q"
    let err = fs.create_exclusive(search_dir, &nm(b"q")).await.unwrap_err();
    assert_eq!((err) as u32, nfsstat3::NFS3ERR_EXIST as u32);
}

// ---- TODO-16: read-only views reject mutation ----

#[tokio::test]
async fn read_only_views_reject_write_and_setattr() {
    let fs = make().await;
    let root = fs.root_dir();
    for dir_name in ["search".as_bytes(), b"where".as_ref()] {
        let d = fs.lookup(root, &nm(dir_name)).await.unwrap();
        let id = fs.lookup(d, &nm(b"q")).await.unwrap();
        assert_eq!(
            fs.write(id, 0, b"x").await.unwrap_err() as u32,
            nfsstat3::NFS3ERR_ROFS as u32,
            "write to /{}/q must be ROFS",
            String::from_utf8_lossy(dir_name)
        );
        assert_eq!(
            fs.setattr(id, sattr3::default()).await.unwrap_err() as u32,
            nfsstat3::NFS3ERR_ROFS as u32
        );
    }
}

// ---- TODO-17: getattr/readdir report real byte size ----

#[tokio::test]
async fn index_getattr_and_readdir_report_real_byte_size() {
    let fs = make().await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"sized")).await.unwrap();

    let text = "the quick brown fox"; // 19 bytes
    fs.write(id, 0, text.as_bytes()).await.unwrap();

    let attr = fs.getattr(id).await.unwrap();
    assert_eq!(attr.size, text.len() as u64, "getattr size == content length");

    let res = fs.readdir(index_dir, 0, 64).await.unwrap();
    let entry = res
        .entries
        .iter()
        .find(|e| e.name.0 == b"sized")
        .expect("sized entry listed");
    assert_eq!(entry.attr.size, text.len() as u64, "readdir size == content length");
}

#[tokio::test]
async fn search_getattr_keeps_virtual_size() {
    // /search content is dynamic per query: its size stays a virtual ceiling.
    let fs = make().await;
    let root = fs.root_dir();
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let id = fs.lookup(search_dir, &nm(b"anything")).await.unwrap();
    let attr = fs.getattr(id).await.unwrap();
    assert_eq!(attr.size, anyhome::magic::MAGIC_FILE_SIZE);
}

// ---- TODO-18: handles survive a restart ----

#[tokio::test]
async fn index_handle_survives_restart() {
    let dir = tempdir().expect("tempdir").keep();
    let db_path = dir.join("mem.db");
    let fs_root = dir.join("fs");
    std::fs::create_dir_all(&fs_root).unwrap();

    {
        let real = RealFs::new(fs_root.clone()).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);
        index_doc(&fs, "persist", b"survives the restart").await;
    }

    // Fresh AnyHomeFs (empty in-memory registry) models a process restart.
    {
        let real = RealFs::new(fs_root).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);

        let root = fs.root_dir();
        let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
        let id = fs.lookup(index_dir, &nm(b"persist")).await.unwrap();

        let (data, _) = fs.read(id, 0, 4096).await.unwrap();
        assert_eq!(&String::from_utf8_lossy(&data), "survives the restart");

        // Real size is recovered after restart too.
        let attr = fs.getattr(id).await.unwrap();
        assert_eq!(attr.size, "survives the restart".len() as u64);
    }
}

// ---- TODO-19: writes are UTF-8-safe (no silent corruption) ----

#[tokio::test]
async fn index_write_rejects_invalid_utf8() {
    let fs = make().await;
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"bin")).await.unwrap();

    // 0xFF is not a valid UTF-8 byte sequence: reject explicitly (INVAL).
    let err = fs.write(id, 0, &[0xFF]).await.unwrap_err();
    assert_eq!((err) as u32, nfsstat3::NFS3ERR_INVAL as u32);

    // Nothing was persisted under the key.
    assert!(fs.db.get_text("", "bin").await.unwrap().is_none());
}
