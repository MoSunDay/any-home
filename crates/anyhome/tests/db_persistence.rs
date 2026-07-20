//! Persistence (TODO-08): a file-backed DB survives a close/reopen, and a
//! mismatched embedding dimension is rejected on reopen (never silently applied
//! to a stale vector column). These are the hard gates for "restart survival".

use anyhome::db::{dummy_embed, MemoryDb};

#[tokio::test]
async fn file_db_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem.db");
    let p = path.to_str().unwrap();

    let db = MemoryDb::open(p, 8).await.expect("open");
    db.upsert("alpha", "hello world", &dummy_embed("hello world", 8))
        .await
        .unwrap();
    db.upsert("beta", "nfs memory", &dummy_embed("nfs memory", 8))
        .await
        .unwrap();
    drop(db); // close

    // Reopen with the same dim: data must still be there.
    let db = MemoryDb::open(p, 8).await.expect("reopen");
    assert_eq!(db.list_keys().await.unwrap(), vec!["alpha", "beta"]);
    assert_eq!(
        db.get_text("alpha").await.unwrap().as_deref(),
        Some("hello world")
    );
    // Vector search still works after reopen.
    let res = db.search(&dummy_embed("hello world", 8), 1).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].0, "alpha");
}

#[tokio::test]
async fn reopen_with_mismatched_dim_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("m.db");
    let p = p.to_str().unwrap();
    let db = MemoryDb::open(p, 8).await.unwrap();
    db.upsert("k", "x", &dummy_embed("x", 8)).await.unwrap();
    drop(db);

    // Reopen with a different dim: must hard-error, not silently misalign the
    // embedding column against the active embedder.
    let err = MemoryDb::open(p, 16).await;
    assert!(err.is_err(), "mismatched dim must be rejected");
    let msg = format!("{}", err.err().unwrap());
    assert!(
        msg.contains("mismatch"),
        "error should explain the dimension mismatch: {msg}"
    );
}

#[tokio::test]
async fn reopen_with_same_dim_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ok.db");
    let p = p.to_str().unwrap();
    MemoryDb::open(p, 4).await.unwrap();
    MemoryDb::open(p, 4).await.expect("reopen same dim");
}
