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
