//! Dispatcher routing: magic paths reach the DB, real paths reach RealFs.
//! Runs entirely in-process (no NFS server / network).

use std::sync::Arc;

use nfsserve::nfs::{filename3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, MemoryDb, RealFs};

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

async fn make() -> AnyHomeFs {
    // into_path(): the TempDir would otherwise be deleted when make() returns.
    let root = tempdir().expect("tempdir").keep();
    let real = RealFs::new(root).unwrap();
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
    AnyHomeFs::new(real, db, emb)
}

#[tokio::test]
async fn magic_dirs_are_visible_at_root() {
    let fs = make().await;
    let root = fs.root_dir();

    assert_eq!(fs.lookup(root, &nm(b"search")).await.unwrap(), anyhome::magic::SEARCH_DIR);
    assert_eq!(fs.lookup(root, &nm(b"index")).await.unwrap(), anyhome::magic::INDEX_DIR);
    assert_eq!(fs.lookup(root, &nm(b"where")).await.unwrap(), anyhome::magic::WHERE_DIR);

    // Root readdir advertises them.
    let res = fs.readdir(root, 0, 64).await.unwrap();
    let names: Vec<Vec<u8>> = res.entries.iter().map(|e| e.name.clone().0).collect();
    assert!(names.iter().any(|n| n == b"search"));
    assert!(names.iter().any(|n| n == b"index"));
    assert!(names.iter().any(|n| n == b"where"));
}

#[tokio::test]
async fn index_then_search_roundtrips_via_magic_paths() {
    let fs = make().await;
    let root = fs.root_dir();

    // /index/greeting  <- write text
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let greeting_id = fs.lookup(index_dir, &nm(b"greeting")).await.unwrap();
    let text = "hello anyhome";
    fs.write(greeting_id, 0, text.as_bytes()).await.unwrap();

    // /search/hello anyhome  -> read KNN
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q_id = fs.lookup(search_dir, &nm(text.as_bytes())).await.unwrap();
    let (data, _eof) = fs.read(q_id, 0, 4096).await.unwrap();
    let out = String::from_utf8(data).unwrap();

    // Exact-match (DummyEmbedder): the stored memory ranks top-1.
    let first = out.lines().next().expect("at least one result line");
    let (key, body) = first.split_once('\t').expect("key\\ttext format");
    assert_eq!(key, "greeting", "matched memory is keyed 'greeting'");
    assert_eq!(body, text, "matched memory text is what we wrote");
}

#[tokio::test]
async fn magic_file_has_reg_attr_and_is_readable() {
    let fs = make().await;
    let root = fs.root_dir();
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q_id = fs.lookup(search_dir, &nm(b"anything")).await.unwrap();

    let attr = fs.getattr(q_id).await.unwrap();
    assert_eq!(attr.size, anyhome::magic::MAGIC_FILE_SIZE, "magic file reports fixed size");
    // Reading /search on an empty index yields a valid (empty) body.
    let (data, _eof) = fs.read(q_id, 0, 4096).await.unwrap();
    assert!(String::from_utf8_lossy(&data).trim().is_empty(), "empty index -> empty result");
}

#[tokio::test]
async fn real_paths_still_work_alongside_magic() {
    let fs = make().await;
    let root = fs.root_dir();

    // A plain file under root (not magic) hits RealFs.
    let (id, _) = fs.create(root, &nm(b"plain.txt"), sattr3::default()).await.unwrap();
    fs.write(id, 0, b"plain data").await.unwrap();
    let (data, _) = fs.read(id, 0, 4096).await.unwrap();
    assert_eq!(data, b"plain data");

    // Root readdir lists the real file too.
    let res = fs.readdir(root, 0, 64).await.unwrap();
    assert!(res.entries.iter().any(|e| e.name.0 == b"plain.txt"));
}

#[tokio::test]
async fn writing_to_search_is_a_noop_but_does_not_error() {
    let fs = make().await;
    let root = fs.root_dir();
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q_id = fs.lookup(search_dir, &nm(b"q")).await.unwrap();
    // Writes to /search/* are ignored (only /index/* upserts).
    fs.write(q_id, 0, b"ignored").await.unwrap();
}

// ---- /where full-text (TODO-09) and /index persistence (TODO-08) ----

async fn index_doc(fs: &AnyHomeFs, key: &str, text: &[u8]) {
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(key.as_bytes())).await.unwrap();
    fs.write(id, 0, text).await.unwrap();
}

#[tokio::test]
async fn where_fulltext_returns_matching_keys_only() {
    let fs = make().await;
    index_doc(&fs, "rust", b"The Rust language focuses on safety").await;
    index_doc(&fs, "py", b"Python is popular for data science").await;

    let root = fs.root_dir();
    let where_dir = fs.lookup(root, &nm(b"where")).await.unwrap();
    let id = fs.lookup(where_dir, &nm(b"rust")).await.unwrap();
    let (data, _) = fs.read(id, 0, 4096).await.unwrap();
    let out = String::from_utf8_lossy(&data);
    assert!(
        out.starts_with("rust\t"),
        "/where/rust must surface the rust doc: {out:?}"
    );
    assert!(
        !out.contains("python"),
        "/where/rust must not match the unrelated doc: {out:?}"
    );
}

#[tokio::test]
async fn where_key_prefix_filters_by_key() {
    let fs = make().await;
    index_doc(&fs, "alpha", b"first doc").await;
    index_doc(&fs, "alfa", b"near match").await;
    index_doc(&fs, "beta", b"other").await;

    let root = fs.root_dir();
    let where_dir = fs.lookup(root, &nm(b"where")).await.unwrap();
    // percent-encode the ':' is unnecessary, but the slash-free segment is key:alpha
    let id = fs.lookup(where_dir, &nm(b"key:alpha")).await.unwrap();
    let (data, _) = fs.read(id, 0, 4096).await.unwrap();
    let out = String::from_utf8_lossy(&data);
    assert!(out.contains("alpha"), "key:alpha matches alpha: {out:?}");
    assert!(!out.contains("beta"), "key:alpha excludes beta: {out:?}");
}

#[tokio::test]
async fn index_key_reads_back_its_text() {
    let fs = make().await;
    index_doc(&fs, "note", b"the readback text").await;

    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"note")).await.unwrap();
    let (data, _) = fs.read(id, 0, 4096).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&data), "the readback text");
}

#[tokio::test]
async fn index_listing_and_text_survive_restart() {
    // A file-backed DB + a brand-new AnyHomeFs (fresh in-memory registry) models
    // a server restart: history must be rebuilt from the DB, not lost.
    let dir = tempdir().expect("tempdir").keep();
    let db_path = dir.join("mem.db");
    let fs_root = dir.join("fs");
    std::fs::create_dir_all(&fs_root).unwrap();

    // session 1: write two memories
    {
        let real = RealFs::new(fs_root.clone()).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);
        index_doc(&fs, "k1", b"first memory").await;
        index_doc(&fs, "k2", b"second memory").await;
    }

    // session 2: fresh registry, same DB file -> history rebuilt from DB
    {
        let real = RealFs::new(fs_root).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16).await.unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);

        let root = fs.root_dir();
        let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
        let res = fs.readdir(index_dir, 0, 64).await.unwrap();
        let names: Vec<String> = res
            .entries
            .iter()
            .map(|e| String::from_utf8_lossy(&e.name.0).into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "k1"), "k1 rebuilt after restart: {names:?}");
        assert!(names.iter().any(|n| n == "k2"), "k2 rebuilt after restart: {names:?}");

        // The stored text is still readable.
        let k1 = fs.lookup(index_dir, &nm(b"k1")).await.unwrap();
        let (data, _) = fs.read(k1, 0, 4096).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&data), "first memory");
    }
}
