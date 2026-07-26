//! `/where` full-text + key-prefix filtering (TODO-09), at the DB layer. Backed
//! by the FTS5 shadow table kept in sync by triggers; verified to differ from
//! lexical overlap and to drop a document after deletion.

use anyhome::db::{DummyEmbedder, Embedder, MemoryDb};

async fn fixture(dim: usize) -> MemoryDb {
    let db = MemoryDb::open(":memory:", dim).await.unwrap();
    let emb = DummyEmbedder::new(dim);
    for (k, t) in [
        ("rust", "The Rust programming language focuses on safety and speed."),
        ("py", "Python is a popular language for data science."),
        ("nfs", "NFS lets clients mount a remote filesystem locally."),
    ] {
        let v = emb.embed(t).await.unwrap();
        db.upsert("", k, t, &v).await.unwrap();
    }
    db
}

#[tokio::test]
async fn where_matches_a_single_term() {
    let db = fixture(16).await;
    let res = db.where_search("", "rust", 10).await.unwrap();
    assert_eq!(res.len(), 1, "only the rust doc mentions rust");
    assert_eq!(res[0].0, "rust");
}

#[tokio::test]
async fn where_matches_shared_word_across_docs() {
    let db = fixture(16).await;
    // 'language' occurs in two docs.
    let res = db.where_search("", "language", 10).await.unwrap();
    let keys: Vec<&str> = res.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(res.len(), 2, "two docs contain 'language': {keys:?}");
    assert!(keys.contains(&"rust") && keys.contains(&"py"));
}

#[tokio::test]
async fn where_no_longer_hits_after_delete() {
    let db = fixture(16).await;
    assert_eq!(db.where_search("", "rust", 10).await.unwrap().len(), 1);
    db.delete("", "rust").await.unwrap();
    assert_eq!(
        db.where_search("", "rust", 10).await.unwrap().len(),
        0,
        "deleted doc is no longer hit"
    );
}

#[tokio::test]
async fn where_key_prefix_filter_matches_keys() {
    let db = fixture(16).await;
    let res = db.where_search("", "key:r", 10).await.unwrap();
    assert_eq!(res.len(), 1, "only 'rust' starts with 'r'");
    assert_eq!(res[0].0, "rust");
}

#[tokio::test]
async fn where_empty_condition_returns_nothing() {
    let db = fixture(16).await;
    assert!(db.where_search("", "   ", 10).await.unwrap().is_empty());
}

/// Agent namespace isolation at the FTS layer: `/where` scoped to one agent
/// must never surface a document that belongs to a different agent, even when
/// the same key and a shared search term appear in both namespaces.
#[tokio::test]
async fn agent_isolation_where_fts() {
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let emb = DummyEmbedder::new(16);

    // Same key "k1", both texts mention "rust", but under different agents.
    let va = emb.embed("rust safety speed").await.unwrap();
    let vb = emb.embed("rust data science").await.unwrap();
    db.upsert("a", "k1", "rust safety speed", &va).await.unwrap();
    db.upsert("b", "k1", "rust data science", &vb).await.unwrap();

    // Agent a's /where must return only agent a's k1.
    let res_a = db.where_search("a", "rust", 10).await.unwrap();
    assert_eq!(res_a.len(), 1, "agent a sees only its own doc: {res_a:?}");
    assert_eq!(res_a[0].0, "k1");
    assert_eq!(res_a[0].1, "rust safety speed");

    // Agent b's /where must return only agent b's k1.
    let res_b = db.where_search("b", "rust", 10).await.unwrap();
    assert_eq!(res_b.len(), 1, "agent b sees only its own doc: {res_b:?}");
    assert_eq!(res_b[0].0, "k1");
    assert_eq!(res_b[0].1, "rust data science");
}
