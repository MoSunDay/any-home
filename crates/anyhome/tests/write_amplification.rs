//! Write-path amplification spike (init-pro-8, TODO-46).
//!
//! The `/index` write contract is fixed and protected:
//! 1. every WRITE RPC upserts to the DB write-through (fsynced on commit), and
//! 2. every WRITE RPC embeds the full composed text once.
//!
//! This spike pins the observable *shape* of that cost with a counting
//! embedder wrapper and a `get_text` counter, so later optimization can only
//! remove semantic-irrelevant overhead — never the contract itself. Baseline
//! numbers are recorded in `plans/init-pro-8/index.md` (decision D5).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use nfsserve::nfs::filename3;
use nfsserve::vfs::NFSFileSystem;
use tempfile::tempdir;

use anyhome::db::{DummyEmbedder, Embedder};
use anyhome::{AnyHomeFs, MemoryDb, RealFs};

/// The chunk size a typical NFS client uses for one large write (32 KiB).
const CHUNK: usize = 32 * 1024;
/// 8 chunks => 256 KiB total: enough to expose per-RPC costs, still instant
/// under the DummyEmbedder.
const CHUNKS: usize = 8;

/// Embedder wrapper counting calls and input bytes. Delegates unchanged to the
/// inner embedder, so behavior is identical.
struct CountingEmbedder {
    inner: Arc<dyn Embedder>,
    calls: AtomicUsize,
    bytes: AtomicUsize,
}

#[async_trait]
impl Embedder for CountingEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.bytes.fetch_add(text.len(), Ordering::SeqCst);
        self.inner.embed(text).await
    }

    fn dim(&self) -> usize {
        self.inner.dim()
    }
}

fn nm(s: &[u8]) -> filename3 {
    s.into()
}

async fn make_fs() -> (AnyHomeFs, Arc<CountingEmbedder>) {
    let root = tempdir().expect("tempdir").keep();
    let real = RealFs::new(root).unwrap();
    let db = MemoryDb::open(":memory:", 16).await.unwrap();
    let counting = Arc::new(CountingEmbedder {
        inner: Arc::new(DummyEmbedder::new(16)),
        calls: AtomicUsize::new(0),
        bytes: AtomicUsize::new(0),
    });
    let emb: Arc<dyn Embedder> = counting.clone();
    (AnyHomeFs::new(real, db, emb), counting)
}

async fn index_handle(fs: &AnyHomeFs, key: &[u8]) -> nfsserve::nfs::fileid3 {
    let root = fs.root_dir();
    let index_dir = fs.lookup(root, &nm(b"index")).await.unwrap();
    fs.lookup(index_dir, &nm(key)).await.unwrap()
}

/// Payload of ASCII letters (valid UTF-8), CHUNK * CHUNKS bytes.
fn payload() -> Vec<u8> {
    (0..CHUNK * CHUNKS)
        .map(|i| b'a' + (i % 26) as u8)
        .collect()
}

#[tokio::test]
async fn every_write_rpc_embeds_full_text_once_and_upserts_write_through() {
    let (fs, emb) = make_fs().await;
    let id = index_handle(&fs, b"big").await;
    let data = payload();

    for i in 0..CHUNKS {
        fs.write(id, (i * CHUNK) as u64, &data[i * CHUNK..(i + 1) * CHUNK])
            .await
            .unwrap();
        // Write-through: exactly one upsert per WRITE, already visible in the
        // authoritative store when the RPC returns (no close-time flush).
        let stored = fs.db.get_text("", "big").await.unwrap().unwrap();
        assert_eq!(
            stored.as_bytes(),
            &data[..(i + 1) * CHUNK],
            "WRITE must be durable (composed with prior chunks) on return"
        );
    }

    // One embed per WRITE RPC (= ceil(N / C)); never per-close, never zero.
    assert_eq!(emb.calls.load(Ordering::SeqCst), CHUNKS);
    // Full-text embed: the embedder input is the whole composed buffer, so
    // total embedded bytes are C + 2C + ... + mC = C*m*(m+1)/2. This is the
    // protected O(N^2/C) shape — only its constant factor is optimizable.
    assert_eq!(
        emb.bytes.load(Ordering::SeqCst),
        CHUNK * CHUNKS * (CHUNKS + 1) / 2
    );
}

#[tokio::test]
async fn hot_write_path_db_read_shape() {
    // Read-amplification baseline (pre TODO-48): after upserting, the write
    // path re-reads the stored text once just to size the fattr
    // (`index_attr`), and the very first WRITE probes the DB once for a seed
    // buffer. => m WRITEs cost m + 1 `get_text` calls. TODO-48 removes the
    // per-WRITE fattr re-read (assertion tightened there to exactly 1).
    let (fs, _emb) = make_fs().await;
    let id = index_handle(&fs, b"big").await;
    let data = payload();

    for i in 0..CHUNKS {
        fs.write(id, (i * CHUNK) as u64, &data[i * CHUNK..(i + 1) * CHUNK])
            .await
            .unwrap();
    }
    assert_eq!(fs.db.text_read_count(), (CHUNKS + 1) as u64);
}
