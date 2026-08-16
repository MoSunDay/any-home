//! End-to-end vector store behavior. `smoke_vector_top_k` is the hard gate for
//! the project: it proves the brute-force KNN search actually orders results
//! by cosine similarity (not just that queries don't crash). Search has been a
//! scoped cosine scan over the agent's partition since init-pro-12 (TODO-68);
//! the ANN index was retired in init-pro-13.

use anyhome::db::{dummy_embed, DummyEmbedder, Embedder, MemoryDb};
use anyhome::db::vec_to_json;

/// Hard gate: index 3 distinct unit-ish vectors and confirm KNN returns them
/// ordered by ascending cosine distance, with the exact-match vector on top.
#[tokio::test]
async fn smoke_vector_top_k() {
    let db = MemoryDb::open(":memory:", 3).await.expect("open db");

    // 'a' and 'c' both point near +x; 'b' points along +y.
    db.upsert("", "a", "alpha", &[1.0, 0.0, 0.0]).await.unwrap();
    db.upsert("", "b", "beta", &[0.0, 1.0, 0.0]).await.unwrap();
    db.upsert("", "c", "gamma", &[0.9, 0.1, 0.0]).await.unwrap();

    let res = db.search("", &[1.0, 0.0, 0.0], 2).await.expect("search");
    assert_eq!(res.len(), 2, "k=2 returns two rows");
    // Exact match is top-1.
    assert_eq!(res[0].1, "alpha", "exact match ranks first");
    // 'c' (cos ~0.994) beats 'b' (cos = 0).
    assert_eq!(res[1].1, "gamma", "nearest non-exact match ranks second");
}

/// DummyEmbedder is deterministic and exact-match yields the same vector.
#[tokio::test]
async fn dummy_embedder_is_deterministic_and_matches() {
    let emb = DummyEmbedder::new(64);
    let v1 = emb.embed("hello world").await.unwrap();
    let v2 = emb.embed("hello world").await.unwrap();
    assert_eq!(v1, v2, "same text -> same vector");
    assert_eq!(v1.len(), 64);

    // Trailing whitespace (echo-style) must not change the embedding.
    let v3 = emb.embed("hello world\n").await.unwrap();
    assert_eq!(v1, v3, "trailing newline is trimmed");
}

/// Indexing via the embedder then searching must round-trip an exact match.
#[tokio::test]
async fn index_then_search_exact_match_top1() {
    let dim = 32;
    let db = MemoryDb::open(":memory:", dim).await.unwrap();
    let emb = DummyEmbedder::new(dim);

    for text in ["the quick brown fox", "lazy dogs", "nfs memory"] {
        let v = emb.embed(text).await.unwrap();
        db.upsert("", text, text, &v).await.unwrap();
    }

    let query = "lazy dogs";
    let qv = emb.embed(query).await.unwrap();
    let res = db.search("", &qv, 1).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].1, query, "exact-match query is top-1");
}

#[tokio::test]
async fn delete_removes_a_memory() {
    let dim = 8;
    let db = MemoryDb::open(":memory:", dim).await.unwrap();
    let v = dummy_embed("hello", dim);
    db.upsert("", "k", "hello", &v).await.unwrap();
    assert_eq!(db.search("", &v, 10).await.unwrap().len(), 1);
    db.delete("", "k").await.unwrap();
    assert!(db.search("", &v, 10).await.unwrap().is_empty());
}

#[test]
fn vec_to_json_is_valid_for_libsql() {
    let s = vec_to_json(&[1.0, 2.5, -3.0]);
    assert!(s.starts_with('[') && s.ends_with(']'));
    // Every component must carry a decimal point (libsql vector() requirement).
    assert!(s.contains("1.0"));
    assert!(s.contains("2.5"));
}

/// Agent namespace isolation at the vector-search layer: a KNN search scoped to
/// one agent must never surface rows that belong to a different agent, even
/// when the key and the embedding are nearly identical.
#[tokio::test]
async fn agent_isolation_search() {
    let db = MemoryDb::open(":memory:", 3).await.expect("open db");

    // Same key "x", near-identical vectors, but different agents. The stored
    // text differs so we can tell which row the search returned.
    db.upsert("a", "x", "a-text", &[1.0, 0.0, 0.0]).await.unwrap();
    db.upsert("b", "x", "b-text", &[0.9, 0.1, 0.0]).await.unwrap();

    // Searching agent "a" must return only agent a's row.
    let res_a = db.search("a", &[1.0, 0.0, 0.0], 10).await.expect("search a");
    assert_eq!(res_a.len(), 1, "agent a sees only its own row: {res_a:?}");
    assert_eq!(res_a[0].0, "x");
    assert_eq!(res_a[0].1, "a-text");

    // Searching agent "b" must return only agent b's row.
    let res_b = db.search("b", &[1.0, 0.0, 0.0], 10).await.expect("search b");
    assert_eq!(res_b.len(), 1, "agent b sees only its own row: {res_b:?}");
    assert_eq!(res_b[0].0, "x");
    assert_eq!(res_b[0].1, "b-text");
}

/// Defect E (init-pro-12 TODO-68): agent-scoped KNN must not be truncated by
/// other agents' nearer rows. The old shape ran `vector_top_k(..., k)` over
/// the GLOBAL index and filtered by agent afterwards, so a competitor holding
/// distance-0 rows consumed all k slots and a scoped query returned empty
/// even though the agent had memories (e2e: `/agents/botA/search_k1` came
/// back empty once botB existed).
#[tokio::test]
async fn scoped_search_survives_other_agents_nearer_rows() {
    let db = MemoryDb::open(":memory:", 3).await.unwrap();
    // A rival agent owns the exact query vector (distance 0), twice.
    db.upsert("rival", "q", "rival text", &[1.0, 0.0, 0.0])
        .await
        .unwrap();
    db.upsert("rival", "q2", "rival text 2", &[1.0, 0.0, 0.0])
        .await
        .unwrap();
    // The requesting agent's only row points elsewhere.
    db.upsert("me", "mine", "my text", &[0.9, 0.1, 0.0])
        .await
        .unwrap();

    // k=1 and k=2 used to come back empty for "me".
    let res = db.search("me", &[1.0, 0.0, 0.0], 1).await.unwrap();
    assert_eq!(res.len(), 1, "scoped k=1 must return the agent's own row");
    assert_eq!(res[0].0, "mine");
    let res = db.search("me", &[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(res.len(), 1, "agent has only one row");

    // The default agent is scoped the same way.
    db.upsert("", "glob", "global text", &[0.8, 0.2, 0.0])
        .await
        .unwrap();
    let res = db.search("", &[1.0, 0.0, 0.0], 1).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].0, "glob");

    // Rival's own scoped search: exact ties break deterministically by id.
    let res = db.search("rival", &[1.0, 0.0, 0.0], 2).await.unwrap();
    assert_eq!(res.len(), 2);
    assert_eq!(res[0].0, "q", "distance ties order by ascending id");
    assert_eq!(res[1].0, "q2");
}
