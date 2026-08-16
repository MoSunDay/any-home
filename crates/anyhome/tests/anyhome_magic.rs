//! Dispatcher routing: magic paths reach the DB, real paths reach RealFs.
//! Runs entirely in-process (no NFS server / network).

use std::sync::Arc;

use nfsserve::nfs::{fileid3, filename3, nfsstat3, sattr3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::magic::{magic_file_id, scoped_name, CAT_INDEX};
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
    // TODO-70: query files report their real content size, not a virtual
    // ceiling — an empty result is 0 bytes. (A virtual 1 MiB size made NFSv3
    // clients NUL-pad every read up to the attr size, RFC 1813.)
    assert_eq!(attr.size, 0, "empty query result reports size 0");
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
async fn writing_to_search_is_rejected_as_read_only() {
    let fs = make().await;
    let root = fs.root_dir();
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q_id = fs.lookup(search_dir, &nm(b"q")).await.unwrap();
    // /search/* are read-only query views: writes are rejected (ROFS) rather
    // than silently dropped.
    let err = fs.write(q_id, 0, b"ignored").await.unwrap_err();
    assert_eq!((err) as u32, nfsstat3::NFS3ERR_ROFS as u32);
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

/// The magic fileid for an agent-scoped entry is derived from the *scoped name*
/// (`"{agent_id}\0{name}"`), so two different agents can hold the same key
/// without their synthetic fileids colliding, and a single agent's two keys
/// never collide either.
#[test]
fn magic_file_id_agent_scoped_no_collision() {
    // Same key, different agents -> different fileids.
    assert_ne!(
        magic_file_id(CAT_INDEX, &scoped_name("agentA", "key1")),
        magic_file_id(CAT_INDEX, &scoped_name("agentB", "key1")),
        "same key under different agents must map to distinct fileids"
    );

    // Same agent, different keys -> different fileids.
    assert_ne!(
        magic_file_id(CAT_INDEX, &scoped_name("agentA", "key1")),
        magic_file_id(CAT_INDEX, &scoped_name("agentA", "key2")),
        "different keys under the same agent must map to distinct fileids"
    );
}

// ---- /search_k<N> explicit top-k (TODO-42, init-pro-7) ----

/// Read a magic file fully via repeated 4096-byte reads.
async fn read_all(fs: &AnyHomeFs, id: fileid3) -> String {
    let mut out = Vec::new();
    let mut off = 0u64;
    loop {
        let (mut chunk, eof) = fs.read(id, off, 4096).await.unwrap();
        out.append(&mut chunk);
        if eof {
            return String::from_utf8_lossy(&out).into_owned();
        }
        off += 4096;
    }
}

/// `/search_k<N>/<query>` caps the result at N rows while `/search` keeps
/// its DEFAULT_K=16 behavior (all 6 docs here).
#[tokio::test]
async fn search_k_limits_knn_results() {
    let fs = make().await;
    for i in 0..6 {
        index_doc(&fs, &format!("k{i}"), format!("shared token doc {i}").as_bytes()).await;
    }
    let root = fs.root_dir();

    let k2_dir = fs.lookup(root, &nm(b"search_k2")).await.unwrap();
    let q2 = fs.lookup(k2_dir, &nm(b"shared token")).await.unwrap();
    let body2 = read_all(&fs, q2).await;
    let lines2: Vec<&str> = body2.lines().collect();
    assert_eq!(lines2.len(), 2, "search_k2 caps at 2: {lines2:?}");

    let k4_dir = fs.lookup(root, &nm(b"search_k4")).await.unwrap();
    let q4 = fs.lookup(k4_dir, &nm(b"shared token")).await.unwrap();
    assert_eq!(read_all(&fs, q4).await.lines().count(), 4);

    // Backward compatibility: plain /search returns everything (<= DEFAULT_K).
    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let q = fs.lookup(search_dir, &nm(b"shared token")).await.unwrap();
    assert_eq!(read_all(&fs, q).await.lines().count(), 6);
}

/// N is clamped to 1..=64; degenerate spellings stay outside the family and
/// fall through to the real FS (NOENT), and search_k dirs are not enumerable.
#[tokio::test]
async fn search_k_clamping_and_rejection() {
    let fs = make().await;
    index_doc(&fs, "a", b"alpha beta").await;
    let root = fs.root_dir();

    // Clamped: search_k999 behaves like search_k64; search_k1 pins k=1.
    let big = fs.lookup(root, &nm(b"search_k999")).await.unwrap();
    assert!(fs.getattr(big).await.is_ok(), "clamped k dir is a valid dir");
    let one = fs.lookup(root, &nm(b"search_k1")).await.unwrap();
    let q1 = fs.lookup(one, &nm(b"alpha")).await.unwrap();
    assert_eq!(read_all(&fs, q1).await.lines().count(), 1, "k=1 returns exactly one row");

    // Not the family: no digits, k0, non-digit suffix -> real FS miss.
    for bad in ["search_k", "search_k0", "search_kx", "search_k-2"] {
        assert!(
            matches!(fs.lookup(root, &nm(bad.as_bytes())).await, Err(nfsstat3::NFS3ERR_NOENT)),
            "{bad} is not a search_k dir"
        );
    }

    // Root readdir advertises only the fixed magic dirs.
    let res = fs.readdir(root, 0, 128).await.unwrap();
    let names: Vec<&[u8]> = res.entries.iter().map(|e| e.name.as_slice()).collect();
    assert!(names.iter().all(|n| !n.starts_with(b"search_k")), "not enumerable");
    // Nor does the k-dir itself list query files as enumerable DB state.
    let inside = fs.readdir(big, 0, 128).await.unwrap();
    assert!(inside.entries.is_empty(), "k dirs carry no enumerable DB rows");
}

/// `/search_k<N>/<q>` and `/search/<q>` resolve to distinct fileids and both
/// stay readable — no cross-contamination of the registry entries.
#[tokio::test]
async fn search_k_fileids_do_not_collide_with_search() {
    let fs = make().await;
    index_doc(&fs, "greet", b"hello anyhome").await;
    let root = fs.root_dir();

    let search_dir = fs.lookup(root, &nm(b"search")).await.unwrap();
    let plain = fs.lookup(search_dir, &nm(b"hello anyhome")).await.unwrap();
    let k8_dir = fs.lookup(root, &nm(b"search_k8")).await.unwrap();
    let salted = fs.lookup(k8_dir, &nm(b"hello anyhome")).await.unwrap();
    assert_ne!(plain, salted, "k-salted fileid differs from /search fileid");

    assert!(read_all(&fs, plain).await.contains("greet"));
    assert!(read_all(&fs, salted).await.contains("greet"));
    // ...including across k values.
    let k4_dir = fs.lookup(root, &nm(b"search_k4")).await.unwrap();
    let other = fs.lookup(k4_dir, &nm(b"hello anyhome")).await.unwrap();
    assert_ne!(other, salted);
}

/// Multi-segment queries (query text containing '/') still parse inside a
/// search_k dir exactly like inside /search — only the leading `search_k<N>`
/// component is special.
#[tokio::test]
async fn search_k_accepts_slash_containing_queries() {
    let fs = make().await;
    index_doc(&fs, "path", b"note/a/b").await;
    let root = fs.root_dir();

    let k2 = fs.lookup(root, &nm(b"search_k2")).await.unwrap();
    // NFS lookup is per-component: "note" then "a" then "b" would need dirs —
    // so like /search, a slashed query is addressed via percent-encoding.
    let q = fs.lookup(k2, &nm(b"note%2Fa%2Fb")).await.unwrap();
    assert!(read_all(&fs, q).await.contains("path"));
}

// ---- /where error observability (TODO-43, init-pro-7) ----

/// A malformed FTS condition reads back as a `# error:` first line instead
/// of an opaque I/O error — agents can tell "no results" from "bad query".
#[tokio::test]
async fn where_malformed_fts_renders_error_line() {
    let fs = make().await;
    index_doc(&fs, "rust", b"The Rust language focuses on safety").await;
    let root = fs.root_dir();
    let where_dir = fs.lookup(root, &nm(b"where")).await.unwrap();

    let bad = fs.lookup(where_dir, &nm(b"rust OR")).await.unwrap();
    let body = read_all(&fs, bad).await;
    assert!(
        body.starts_with("# error: invalid FTS query: rust OR\n"),
        "got: {body:?}"
    );

    // A valid-but-unmatched query is still plain (empty) rows, not an error.
    let miss = fs.lookup(where_dir, &nm(b"zzz_nomatch")).await.unwrap();
    assert_eq!(read_all(&fs, miss).await, "");

    // And a hit keeps the exact `key<TAB>text` line format.
    let hit = fs.lookup(where_dir, &nm(b"rust")).await.unwrap();
    assert_eq!(read_all(&fs, hit).await, "rust\tThe Rust language focuses on safety\n");
}
