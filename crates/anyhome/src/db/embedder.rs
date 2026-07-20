//! Embedding abstraction. The real semantic model lives in [`super::fastembed`]
//! (behind the `embed-fastembed` feature); [`DummyEmbedder`] is the
//! dependency-free default so the index/search pipeline is verifiable without a
//! model download.

use anyhow::Result;
use async_trait::async_trait;

/// Turns text into a fixed-dimension vector. Implementations must be deterministic
/// in `dim` (so identical text yields identical vectors).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed `text` into a vector of length [`Embedder::dim`].
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// The (constant) dimensionality of every vector this embedder produces.
    fn dim(&self) -> usize;
}

/// Default embedding dimension used by [`DummyEmbedder`].
pub const DUMMY_DIM: usize = 64;

/// Deterministic, dependency-free embedder.
///
/// Same text always maps to the same unit vector (after trimming trailing
/// whitespace), so an exact-match search (`query == stored text`) yields cosine
/// similarity 1 and ranks top-1. Different texts are spread across `dim` buckets
/// via FNV hashing of characters and whitespace-split words.
#[derive(Clone, Copy)]
pub struct DummyEmbedder {
    dim: usize,
}

impl DummyEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }
}

impl Default for DummyEmbedder {
    fn default() -> Self {
        Self::new(DUMMY_DIM)
    }
}

#[async_trait]
impl Embedder for DummyEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(dummy_embed(text, self.dim))
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// Pure embedding function: deterministic, no allocations beyond the result.
pub fn dummy_embed(text: &str, dim: usize) -> Vec<f32> {
    let dim = dim.max(1);
    let mut v = vec![0.0f32; dim];
    // Trailing whitespace (e.g. the newline added by `echo`) must not change the
    // vector, otherwise exact-match over the shell would drift.
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return v;
    }
    // Character-level signal.
    for (i, &b) in trimmed.as_bytes().iter().enumerate() {
        let h = splitmix(b as u64 ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15));
        v[(h % dim as u64) as usize] += 1.0;
    }
    // Word-level signal (stronger) so lexically distinct texts separate.
    for word in trimmed.split_whitespace() {
        let mut acc: u64 = 0x811C9DC5;
        for &b in word.as_bytes() {
            acc ^= b as u64;
            acc = acc.wrapping_mul(0x100000001B3);
        }
        let h = splitmix(acc);
        v[(h % dim as u64) as usize] += 3.0;
    }
    normalize(&mut v);
    v
}

fn normalize(v: &mut [f32]) {
    let mut sumsq = 0.0f32;
    for &x in v.iter() {
        sumsq += x * x;
    }
    if sumsq <= 0.0 {
        // leave zero vector as-is (avoid NaN); queries on it still return rows
        return;
    }
    let inv = 1.0 / sumsq.sqrt();
    for x in v.iter_mut() {
        *x *= inv;
    }
}

/// 64-bit splitmix64 mixer (deterministic, good avalanche).
fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = z;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}
