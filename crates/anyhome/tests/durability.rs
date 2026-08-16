//! Write-through durability (init-pro-6, TODO-36/38).
//!
//! The `/index` write path upserts to the DB on every `WRITE` RPC (it is
//! write-through, *not* buffered-until-close). A file-backed DB therefore keeps
//! every committed write fsynced and survives a crash, while WAL lets concurrent
//! NFS reads proceed without blocking the writer. A `:memory:` DB is unaffected
//! (WAL does not apply) but must not error. Checkpoint coverage (fold WAL back
//! into the main file, the graceful-shutdown backup contract) was added in
//! init-pro-13 TODO-73.

use anyhome::db::{dummy_embed, MemoryDb};

#[tokio::test]
async fn file_db_uses_wal_and_full_sync() {
    let dir = tempfile::tempdir().unwrap();
    let db = MemoryDb::open(dir.path().join("d.db").to_str().unwrap(), 8)
        .await
        .expect("open");
    let (mode, sync) = db.durability().await.expect("query durability");
    assert_eq!(mode.to_lowercase(), "wal", "file DB must use WAL journal mode");
    assert_eq!(sync, 2, "synchronous must be FULL (2) so each commit fsyncs");
}

#[tokio::test]
async fn memory_db_stays_ephemeral_without_error() {
    // WAL does not apply to a pure in-memory DB; it must stay "memory" and never
    // error (durability for ":memory:" is, by definition, process-scoped).
    let db = MemoryDb::open(":memory:", 8).await.expect("open");
    let (mode, _sync) = db.durability().await.expect("query durability");
    assert_eq!(mode.to_lowercase(), "memory");
}

#[tokio::test]
async fn write_through_survives_reopen() {
    // Every WRITE upserts immediately (write-through), so a file-backed DB
    // retains committed writes across a close/reopen — the crash-safety contract.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("wt.db");
    let db = MemoryDb::open(p.to_str().unwrap(), 8).await.unwrap();
    db.upsert("", "k", "crash me", &dummy_embed("crash me", 8))
        .await
        .unwrap();
    drop(db); // process exit: fsynced WAL + page cache survive
    let db = MemoryDb::open(p.to_str().unwrap(), 8).await.unwrap();
    assert_eq!(
        db.get_text("", "k").await.unwrap().as_deref(),
        Some("crash me")
    );
}

#[tokio::test]
async fn checkpoint_truncates_wal_to_zero_bytes() {
    // TRUNCATE-mode checkpoint folds every WAL frame into the main file and
    // truncates the `-wal` to zero bytes, so a plain copy of the main file is
    // a consistent backup — the exact contract the graceful shutdown relies on.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ck.db");
    let wal = dir.path().join("ck.db-wal");
    let db = MemoryDb::open(p.to_str().unwrap(), 8).await.unwrap();
    db.upsert("", "k", "fold me back", &dummy_embed("fold me back", 8))
        .await
        .unwrap();
    let wal_size = std::fs::metadata(&wal)
        .expect("-wal sibling must exist after a WAL-mode write")
        .len();
    assert!(wal_size > 0, "WAL must hold frames before checkpoint, got 0 bytes");
    let (busy, _wal_frames, _checkpointed) = db.checkpoint().await.expect("checkpoint");
    assert_eq!(busy, 0, "no concurrent reader, so busy must be 0");
    let wal_size = std::fs::metadata(&wal)
        .expect("-wal sibling must still exist after checkpoint")
        .len();
    assert_eq!(wal_size, 0, "TRUNCATE checkpoint must leave the WAL at zero bytes");
}

#[tokio::test]
async fn checkpointed_data_survives_reopen() {
    // After a checkpoint the main file alone must carry the data: drop the DB
    // (no auto-checkpoint on close is guaranteed), reopen, and read it back.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ck2.db");
    let db = MemoryDb::open(p.to_str().unwrap(), 8).await.unwrap();
    db.upsert("", "a", "alpha text", &dummy_embed("alpha text", 8))
        .await
        .unwrap();
    db.upsert("", "b", "beta text", &dummy_embed("beta text", 8))
        .await
        .unwrap();
    db.checkpoint().await.expect("checkpoint");
    drop(db);
    let db = MemoryDb::open(p.to_str().unwrap(), 8).await.unwrap();
    assert_eq!(
        db.get_text("", "a").await.unwrap().as_deref(),
        Some("alpha text"),
        "first row must survive checkpoint + reopen"
    );
    assert_eq!(
        db.get_text("", "b").await.unwrap().as_deref(),
        Some("beta text"),
        "second row must survive checkpoint + reopen"
    );
    let mut keys = db.list_keys("").await.unwrap();
    keys.sort();
    assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
}

#[tokio::test]
async fn memory_db_checkpoint_is_noop() {
    // A `:memory:` DB has no WAL: the pragma must not error (values are
    // intentionally not asserted, only the no-error contract).
    let db = MemoryDb::open(":memory:", 8).await.expect("open");
    db.checkpoint().await.expect("checkpoint on :memory: must be a harmless no-op");
}
