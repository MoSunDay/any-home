//! Persistence (TODO-08): a file-backed DB survives a close/reopen, and a
//! mismatched embedding dimension is rejected on reopen (never silently applied
//! to a stale vector column). These are the hard gates for "restart survival".

use anyhome::db::{dummy_embed, MemoryDb};
use libsql::Builder;

#[tokio::test]
async fn file_db_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem.db");
    let p = path.to_str().unwrap();

    let db = MemoryDb::open(p, 8).await.expect("open");
    db.upsert("", "alpha", "hello world", &dummy_embed("hello world", 8))
        .await
        .unwrap();
    db.upsert("", "beta", "nfs memory", &dummy_embed("nfs memory", 8))
        .await
        .unwrap();
    drop(db); // close

    // Reopen with the same dim: data must still be there.
    let db = MemoryDb::open(p, 8).await.expect("reopen");
    assert_eq!(db.list_keys("").await.unwrap(), vec!["alpha", "beta"]);
    assert_eq!(
        db.get_text("", "alpha").await.unwrap().as_deref(),
        Some("hello world")
    );
    // Vector search still works after reopen.
    let res = db.search("", &dummy_embed("hello world", 8), 1).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].0, "alpha");
}

#[tokio::test]
async fn reopen_with_mismatched_dim_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("m.db");
    let p = p.to_str().unwrap();
    let db = MemoryDb::open(p, 8).await.unwrap();
    db.upsert("", "k", "x", &dummy_embed("x", 8)).await.unwrap();
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

/// v0 -> v1 migration: a DB created under the *old* schema (no `agent_id`
/// column, `key TEXT UNIQUE`) must still be readable after reopening with the
/// new code. Existing rows are preserved and back-filled with the default
/// agent_id `""`. This is the hard gate for "restart survival across versions".
#[tokio::test]
async fn agent_id_migration_from_v0() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let p = path.to_str().unwrap();

    // Build an old-schema DB directly through libsql: no `agent_id` column,
    // `key TEXT UNIQUE`, plus the FTS5 shadow table + triggers and the vector
    // index, exactly as v0 would have created them.
    {
        let db = Builder::new_local(p).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute(
            "CREATE TABLE memories(id INTEGER PRIMARY KEY, key TEXT UNIQUE, text TEXT, emb FLOAT32(8))",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            (),
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO meta(key, value) VALUES ('dim', '8')", ())
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO memories(key, text, emb) VALUES ('old1', 'legacy data', vector('[1.0,0.0,0.0,0.0,0.0,0.0,0.0,0.0]'))",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE INDEX memories_vec_idx ON memories (libsql_vector_idx(emb))",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE VIRTUAL TABLE memories_fts USING fts5(text, content='memories', content_rowid='id')",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text); END",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, text) VALUES('delete', old.id, old.text); END",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, text) VALUES('delete', old.id, old.text); INSERT INTO memories_fts(rowid, text) VALUES (new.id, new.text); END",
            (),
        )
        .await
        .unwrap();
        drop(conn);
        drop(db);
    }

    // Reopen with the new code: the migration must run automatically and
    // preserve the row under the default agent_id "".
    let db = MemoryDb::open(p, 8).await.expect("reopen migrated db");
    assert_eq!(db.list_keys("").await.unwrap(), vec!["old1"]);
    assert_eq!(
        db.get_text("", "old1").await.unwrap().as_deref(),
        Some("legacy data")
    );
}

/// The unique constraint is `(agent_id, key)`, so the same key may exist under
/// two different agents without colliding.
#[tokio::test]
async fn composite_unique_allows_same_key_different_agent() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("agents.db");
    let p = p.to_str().unwrap();

    let db = MemoryDb::open(p, 8).await.expect("open");
    db.upsert("a", "shared", "text from agent a", &dummy_embed("a", 8))
        .await
        .unwrap();
    db.upsert("b", "shared", "text from agent b", &dummy_embed("b", 8))
        .await
        .unwrap();

    // Both rows coexist under the same key but different agent_ids.
    assert!(db.exists("a", "shared").await.unwrap());
    assert!(db.exists("b", "shared").await.unwrap());
    assert_eq!(
        db.get_text("a", "shared").await.unwrap().as_deref(),
        Some("text from agent a")
    );
    assert_eq!(
        db.get_text("b", "shared").await.unwrap().as_deref(),
        Some("text from agent b")
    );
    // Each agent sees only its own key.
    assert_eq!(db.list_keys("a").await.unwrap(), vec!["shared"]);
    assert_eq!(db.list_keys("b").await.unwrap(), vec!["shared"]);
    // Both agents are now registered.
    let agents = db.list_agents().await.unwrap();
    assert!(agents.contains(&"a".to_string()), "agent a listed: {agents:?}");
    assert!(agents.contains(&"b".to_string()), "agent b listed: {agents:?}");
}
