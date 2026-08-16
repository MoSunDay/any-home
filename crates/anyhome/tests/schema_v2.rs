//! Schema v1 -> v2 migration (init-pro-13 TODO-74): retirement of the ANN
//! index `memories_vec_idx`. Since init-pro-12 (TODO-68), `MemoryDb::search`
//! is a scoped brute-force KNN over the agent's partition; nothing queries the
//! index anymore, so every write paid pure maintenance cost. A plain
//! `DROP INDEX` makes libsql cascade-drop the shadow table, its index, and the
//! `libsql_vector_meta_shadow` metadata row. Decision recorded in
//! plans/init-pro-13.

use anyhome::db::{dummy_embed, vec_to_json, MemoryDb};
use libsql::Builder;

/// All object names in `sqlite_master`, ordered.
async fn sqlite_master_names(path: &str) -> Vec<String> {
    let db = Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query("SELECT name FROM sqlite_master ORDER BY name", ())
        .await
        .unwrap();
    let mut names = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        names.push(row.get::<String>(0).unwrap());
    }
    names
}

/// Value of a `meta` key (`None` if absent). `key` is a fixed internal
/// literal at every call site, never user input.
async fn meta_value(path: &str, key: &str) -> Option<String> {
    let db = Builder::new_local(path).build().await.unwrap();
    let conn = db.connect().unwrap();
    let sql = format!("SELECT value FROM meta WHERE key = '{key}'");
    let mut rows = conn.query(&sql, ()).await.unwrap();
    let row = rows.next().await.unwrap()?;
    Some(row.get::<String>(0).unwrap())
}

/// A fresh DB is created at schema v2 and never gets the retired ANN index.
#[tokio::test]
async fn new_db_is_schema_v2_and_creates_no_vector_index() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("fresh.db");
    let p = p.to_str().unwrap();

    let db = MemoryDb::open(p, 8).await.expect("open");
    db.upsert("", "k", "hello world", &dummy_embed("hello world", 8))
        .await
        .unwrap();
    drop(db);

    assert_eq!(
        meta_value(p, "schema_version").await.as_deref(),
        Some("2"),
        "fresh DBs are stamped at schema v2"
    );
    let names = sqlite_master_names(p).await;
    assert!(
        !names.iter().any(|n| n == "memories_vec_idx"),
        "fresh DBs must not create the retired ANN index: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "memories_vec_idx_shadow"),
        "no shadow table without the index: {names:?}"
    );
    for name in ["memories", "memories_fts", "meta"] {
        assert!(names.iter().any(|n| n == name), "{name} must exist: {names:?}");
    }
}

/// A hand-built v1 DB (with the ANN index) reopens into v2: the index and its
/// shadow table are dropped, data survives, and brute-force KNN semantics are
/// unchanged. Reopening again is a no-op (migration idempotence).
#[tokio::test]
async fn v1_db_reopen_drops_index_and_keeps_data_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.db");
    let p = path.to_str().unwrap();

    // Build the v1 fixture by hand: current (v1) table shape, meta stamped at
    // schema_version '1', FTS + sync triggers (verbatim from db_persistence),
    // and the ANN index v1 used to create. Rows store their own dummy_embed
    // vectors so the exact-match search below has a deterministic top-1.
    {
        let db = Builder::new_local(p).build().await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute(
            "CREATE TABLE memories(id INTEGER PRIMARY KEY, agent_id TEXT NOT NULL DEFAULT '', key TEXT NOT NULL, text TEXT, emb FLOAT32(8), UNIQUE(agent_id, key))",
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
            "INSERT INTO meta(key, value) VALUES ('schema_version', '1')",
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
        conn.execute(
            "CREATE INDEX memories_vec_idx ON memories (libsql_vector_idx(emb))",
            (),
        )
        .await
        .unwrap();
        for (key, text) in [("a", "row a text"), ("b", "row b text")] {
            let sql = format!(
                "INSERT INTO memories(agent_id, key, text, emb) \
                 VALUES ('', '{key}', '{text}', vector('{emb}'))",
                emb = vec_to_json(&dummy_embed(text, 8))
            );
            conn.execute(&sql, ()).await.unwrap();
        }
        drop(conn);
        drop(db);
    }

    // Reopen: the v1 -> v2 migration must retire the index and keep the data.
    let db = MemoryDb::open(p, 8).await.expect("reopen v1 db");
    assert_eq!(db.list_keys("").await.unwrap(), vec!["a", "b"]);
    assert_eq!(
        db.get_text("", "a").await.unwrap().as_deref(),
        Some("row a text")
    );
    assert_eq!(
        db.get_text("", "b").await.unwrap().as_deref(),
        Some("row b text")
    );
    // Search semantics unchanged post-migration: exact match on row a's
    // stored text is top-1 under the brute-force cosine scan.
    let res = db
        .search("", &dummy_embed("row a text", 8), 1)
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].0, "a");
    drop(db);

    assert_eq!(
        meta_value(p, "schema_version").await.as_deref(),
        Some("2"),
        "migration stamps schema v2"
    );
    let names = sqlite_master_names(p).await;
    assert!(
        !names.iter().any(|n| n == "memories_vec_idx"),
        "ANN index retired: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "memories_vec_idx_shadow"),
        "shadow table cascade-dropped: {names:?}"
    );

    // Reopen again: idempotent no-op — still v2, still index-less, data intact.
    let db = MemoryDb::open(p, 8).await.expect("second reopen");
    assert_eq!(db.list_keys("").await.unwrap(), vec!["a", "b"]);
    drop(db);
    assert_eq!(meta_value(p, "schema_version").await.as_deref(), Some("2"));
    let names = sqlite_master_names(p).await;
    assert!(
        !names.iter().any(|n| n == "memories_vec_idx"),
        "index stays retired across reopens: {names:?}"
    );
}
