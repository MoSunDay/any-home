//! Write-through durability (init-pro-6, TODO-36/38).
//!
//! The `/index` write path upserts to the DB on every `WRITE` RPC (it is
//! write-through, *not* buffered-until-close). A file-backed DB therefore keeps
//! every committed write fsynced and survives a crash, while WAL lets concurrent
//! NFS reads proceed without blocking the writer. A `:memory:` DB is unaffected
//! (WAL does not apply) but must not error.

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
