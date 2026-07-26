//! Multi-agent namespace isolation integration tests (TODO-26).
//!
//! Verifies that agents sharing one anyhome instance have independent DB key
//! spaces and retrieval scopes: writes to `/agents/<id>/index/` are invisible
//! to other agents, global paths still work (backward-compatible), and the
//! `/agents` directory listing tracks known agents.

use std::sync::Arc;

use nfsserve::nfs::{filename3, nfsstat3};
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder, MemoryDb};

use anyhome::{AnyHomeFs, RealFs};

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

/// Resolve `/agents/<agent>/<sub>/<name>` through successive lookups from root.
async fn agent_file(fs: &AnyHomeFs, agent: &str, sub: &str, name: &str) -> fileid3 {
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let a_dir = fs.lookup(agents, &nm(agent.as_bytes())).await.unwrap();
    let sub_dir = fs.lookup(a_dir, &nm(sub.as_bytes())).await.unwrap();
    fs.lookup(sub_dir, &nm(name.as_bytes())).await.unwrap()
}

/// Write `text` to `/agents/<agent>/index/<key>`.
async fn index_doc_agent(fs: &AnyHomeFs, agent: &str, key: &str, text: &[u8]) {
    let id = agent_file(fs, agent, "index", key).await;
    fs.write(id, 0, text).await.unwrap();
}

/// Write `text` to the global `/index/<key>`.
async fn index_doc_global(fs: &AnyHomeFs, key: &str, text: &[u8]) {
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(key.as_bytes())).await.unwrap();
    fs.write(id, 0, text).await.unwrap();
}

// ---- Isolation: same key, different agents ----

#[tokio::test]
async fn agent_isolation_key_prefix() {
    let fs = make().await;
    index_doc_agent(&fs, "alpha", "shared", b"alpha content").await;
    index_doc_agent(&fs, "beta", "shared", b"beta content").await;

    // Each agent reads back its own content, not the other's.
    let a = agent_file(&fs, "alpha", "index", "shared").await;
    let b = agent_file(&fs, "beta", "index", "shared").await;

    let (a_data, _) = fs.read(a, 0, 4096).await.unwrap();
    let (b_data, _) = fs.read(b, 0, 4096).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&a_data), "alpha content");
    assert_eq!(String::from_utf8_lossy(&b_data), "beta content");

    // DB-level: both keys coexist.
    assert_eq!(
        fs.db.list_keys("alpha").await.unwrap(),
        vec!["shared"]
    );
    assert_eq!(
        fs.db.list_keys("beta").await.unwrap(),
        vec!["shared"]
    );
}

// ---- Global path maps to default agent ----

#[tokio::test]
async fn global_path_maps_to_default_agent() {
    let fs = make().await;
    index_doc_global(&fs, "g1", b"global memory").await;

    // /agents readdir should NOT list the default agent ("").
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let res = fs.readdir(agents, 0, 64).await.unwrap();
    let names: Vec<String> = res
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.name.0).into_owned())
        .collect();
    assert!(
        !names.iter().any(|n| n.is_empty()),
        "default agent should not be listed: {names:?}"
    );

    // But the data is accessible via the global path.
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    let id = fs.lookup(index_dir, &nm(b"g1")).await.unwrap();
    let (data, _) = fs.read(id, 0, 4096).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&data), "global memory");
}

// ---- Global and agent namespaces are independent ----

#[tokio::test]
async fn global_and_agent_namespace_independent() {
    let fs = make().await;
    index_doc_global(&fs, "foo", b"global foo").await;
    index_doc_agent(&fs, "x", "foo", b"agent x foo").await;

    let global_id = {
        let root = fs.root_dir();
        let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
        fs.lookup(index_dir, &nm(b"foo")).await.unwrap()
    };
    let agent_id = agent_file(&fs, "x", "index", "foo").await;

    // Different fileids.
    assert_ne!(global_id, agent_id);

    let (g_data, _) = fs.read(global_id, 0, 4096).await.unwrap();
    let (a_data, _) = fs.read(agent_id, 0, 4096).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&g_data), "global foo");
    assert_eq!(String::from_utf8_lossy(&a_data), "agent x foo");
}

// ---- Agent appears in readdir after write ----

#[tokio::test]
async fn agent_appears_in_readdir_after_write() {
    let fs = make().await;

    // Before write: /agents is empty.
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let res = fs.readdir(agents, 0, 64).await.unwrap();
    assert!(res.entries.is_empty(), "no agents before write");

    // Write to an agent namespace.
    index_doc_agent(&fs, "newbot", "k1", b"hello").await;

    // After write: /agents lists "newbot".
    let res = fs.readdir(agents, 0, 64).await.unwrap();
    let names: Vec<String> = res
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.name.0).into_owned())
        .collect();
    assert!(
        names.iter().any(|n| n == "newbot"),
        "newbot should appear after write: {names:?}"
    );
}

// ---- Agent data survives restart ----

#[tokio::test]
async fn agent_read_after_restart() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("agent.db");
    let fs_root = dir.path().join("fs");
    std::fs::create_dir_all(&fs_root).unwrap();

    // Session 1: write to an agent namespace.
    {
        let real = RealFs::new(fs_root.clone()).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16)
            .await
            .unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);
        index_doc_agent(&fs, "persist", "key1", b"persisted text").await;
    }

    // Session 2: fresh registry, same DB file -> agent data recovered.
    {
        let real = RealFs::new(fs_root.clone()).unwrap();
        let db = MemoryDb::open(db_path.to_str().unwrap(), 16)
            .await
            .unwrap();
        let emb: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(16));
        let fs = AnyHomeFs::new(real, db, emb);

        // The agent is listed.
        let root = fs.root_dir();
        let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
        let res = fs.readdir(agents, 0, 64).await.unwrap();
        let names: Vec<String> = res
            .entries
            .iter()
            .map(|e| String::from_utf8_lossy(&e.name.0).into_owned())
            .collect();
        assert!(names.iter().any(|n| n == "persist"), "agent listed after restart");

        // The index file is readable.
        let id = agent_file(&fs, "persist", "index", "key1").await;
        let (data, _) = fs.read(id, 0, 4096).await.unwrap();
        assert_eq!(String::from_utf8_lossy(&data), "persisted text");
    }
}

// ---- Agent dir getattr is a directory ----

#[tokio::test]
async fn agent_dir_getattr_is_directory() {
    let fs = make().await;
    let root = fs.root_dir();

    // /agents is a directory.
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let attr = fs.getattr(agents).await.unwrap();
    assert_eq!(attr.mode, 0o755);
    assert_eq!(attr.size, 4096);

    // /agents/<id> is a directory.
    let agent_dir = fs.lookup(agents, &nm(b"myagent")).await.unwrap();
    let attr = fs.getattr(agent_dir).await.unwrap();
    assert_eq!(attr.mode, 0o755);
    assert_eq!(attr.size, 4096);

    // /agents/<id>/index is a directory.
    let index_dir = fs.lookup(agent_dir, &nm(b"index")).await.unwrap();
    let attr = fs.getattr(index_dir).await.unwrap();
    assert_eq!(attr.mode, 0o755);
    assert_eq!(attr.size, 4096);
}

// ---- Writing to /agents itself is rejected ----

#[tokio::test]
async fn write_to_agents_dir_is_rejected() {
    let fs = make().await;
    let root = fs.root_dir();
    let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
    let err = fs.write(agents, 0, b"data").await.unwrap_err();
    assert_eq!(err as u32, nfsstat3::NFS3ERR_ISDIR as u32);
}

// ---- Multi-agent recall: independent KNN ----

#[tokio::test]
async fn multi_agent_recall() {
    let fs = make().await;

    // Agent A corpus: programming topics.
    for (k, t) in [
        ("rust", "The Rust programming language"),
        ("go", "Go is compiled and fast"),
        ("py", "Python is dynamic and flexible"),
    ] {
        index_doc_agent(&fs, "a", k, t.as_bytes()).await;
    }

    // Agent B corpus: cooking topics.
    for (k, t) in [
        ("pasta", "Italian pasta with tomato sauce"),
        ("sushi", "Japanese rice and fish"),
        ("curry", "Indian spicy curry dish"),
    ] {
        index_doc_agent(&fs, "b", k, t.as_bytes()).await;
    }

    // Search "programming" in agent A: top results are A's corpus.
    let a_search = agent_file(&fs, "a", "search", "programming%20language").await;
    let (a_data, _) = fs.read(a_search, 0, 8192).await.unwrap();
    let a_text = String::from_utf8_lossy(&a_data);
    assert!(a_text.contains("rust"), "A search includes rust: {a_text}");
    // No B content leaks into A's search.
    assert!(!a_text.contains("pasta"), "A search should not include pasta");
    assert!(!a_text.contains("sushi"), "A search should not include sushi");

    // Search "food" in agent B: top results are B's corpus.
    let b_search = agent_file(&fs, "b", "search", "food%20cooking").await;
    let (b_data, _) = fs.read(b_search, 0, 8192).await.unwrap();
    let b_text = String::from_utf8_lossy(&b_data);
    assert!(b_text.contains("pasta") || b_text.contains("curry") || b_text.contains("sushi"));
    // No A content leaks into B's search.
    assert!(!b_text.contains("rust"), "B search should not include rust");
    assert!(!b_text.contains("Python"), "B search should not include Python");
}

// ---- /agents/<id>/where works scoped ----

#[tokio::test]
async fn agent_where_isolation() {
    let fs = make().await;
    index_doc_agent(&fs, "a", "doc1", b"unique word waffle").await;
    index_doc_agent(&fs, "b", "doc1", b"different content pancake").await;

    let a_where = agent_file(&fs, "a", "where", "waffle").await;
    let (a_data, _) = fs.read(a_where, 0, 4096).await.unwrap();
    let a_text = String::from_utf8_lossy(&a_data);
    assert!(a_text.contains("doc1"), "A where finds doc1: {a_text}");
    assert!(!a_text.contains("pancake"), "A where should not find B's content");

    let b_where = agent_file(&fs, "b", "where", "pancake").await;
    let (b_data, _) = fs.read(b_where, 0, 4096).await.unwrap();
    let b_text = String::from_utf8_lossy(&b_data);
    assert!(b_text.contains("doc1"), "B where finds doc1: {b_text}");
    assert!(!b_text.contains("waffle"), "B where should not find A's content");
}

// ---- /agents/<id>/index readdir lists only that agent's keys ----

#[tokio::test]
async fn agent_index_readdir_scoped() {
    let fs = make().await;
    index_doc_agent(&fs, "a", "k1", b"a1").await;
    index_doc_agent(&fs, "a", "k2", b"a2").await;
    index_doc_agent(&fs, "b", "k3", b"b1").await;

    let a_index = {
        let root = fs.root_dir();
        let agents = fs.lookup(root, &nm(b"agents")).await.unwrap();
        let a_dir = fs.lookup(agents, &nm(b"a")).await.unwrap();
        fs.lookup(a_dir, &nm(b"index")).await.unwrap()
    };
    let res = fs.readdir(a_index, 0, 64).await.unwrap();
    let keys: Vec<String> = res
        .entries
        .iter()
        .map(|e| String::from_utf8_lossy(&e.name.0).into_owned())
        .collect();
    assert!(keys.contains(&"k1".to_string()), "A index has k1: {keys:?}");
    assert!(keys.contains(&"k2".to_string()), "A index has k2: {keys:?}");
    assert!(
        !keys.contains(&"k3".to_string()),
        "A index should NOT have B's k3: {keys:?}"
    );
}

// ---- fileid non-collision between global and agent ----

#[tokio::test]
async fn fileid_no_collision_global_vs_agent() {
    let fs = make().await;
    let global_id = {
        let root = fs.root_dir();
        let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
        fs.lookup(index_dir, &nm(b"same_key")).await.unwrap()
    };
    let agent_id = agent_file(&fs, "agent1", "index", "same_key").await;
    assert_ne!(
        global_id, agent_id,
        "global and agent fileids must not collide"
    );
}

// ---- Magic dir constants include AGENTS_DIR at root ----

#[tokio::test]
async fn agents_dir_listed_at_root() {
    let fs = make().await;
    let root = fs.root_dir();
    let res = fs.readdir(root, 0, 64).await.unwrap();
    let names: Vec<Vec<u8>> = res.entries.iter().map(|e| e.name.clone().0).collect();
    assert!(
        names.iter().any(|n| n == b"agents"),
        "root readdir should list 'agents': {:?}",
        names.iter().map(|n| String::from_utf8_lossy(n).to_string()).collect::<Vec<_>>()
    );
}

use nfsserve::nfs::fileid3;
