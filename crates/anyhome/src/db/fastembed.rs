//! Real semantic embedder backed by a local ONNX model via `fastembed`.
//!
//! Model: `sentence-transformers/all-MiniLM-L6-v2` (ONNX), 384-dim, mean-pooled.
//! The model is fetched once from HuggingFace and cached; afterwards it runs
//! fully offline with no network and no API key — matching anyhome's
//! "plain files, no SDK" philosophy.
//!
//! Inference is CPU-bound synchronous work, so each [`Embedder::embed`] call is
//! dispatched on the blocking thread pool to avoid stalling the async runtime.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::embedder::Embedder;

/// Embedding dimension of [`FastEmbedder`]'s model (all-MiniLM-L6-v2).
pub const FASTEMBED_DIM: usize = 384;

/// Local ONNX semantic embedder.
///
/// Constructing this loads (and if necessary downloads) the model; build it once
/// and share it (the inner model is behind an `Arc`). Embeddings are
/// deterministic for a given model + input, so the cosine-inequality acceptance
/// test for TODO-10 is stable.
pub struct FastEmbedder {
    model: Arc<TextEmbedding>,
    dim: usize,
}

impl FastEmbedder {
    /// Load the default all-MiniLM-L6-v2 model (downloading it on first use).
    pub fn new() -> Result<Self> {
        Self::with_model(EmbeddingModel::AllMiniLML6V2)
    }

    /// Load an explicit fastembed model (exposed for tests / future CLI options).
    pub fn with_model(model: EmbeddingModel) -> Result<Self> {
        let m = TextEmbedding::try_new(InitOptions::new(model))
            .context("load fastembed model (set HF_ENDPOINT/HF_HOME or pre-cache; see scripts/fetch-model.sh)")?;
        Ok(Self {
            model: Arc::new(m),
            dim: FASTEMBED_DIM,
        })
    }
}

#[async_trait]
impl Embedder for FastEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let model = self.model.clone();
        let owned = text.to_string();
        // ONNX inference is blocking; keep the async runtime free.
        let mut vecs = tokio::task::spawn_blocking(move || {
            model.embed(vec![owned], None)
        })
        .await
        .context("join embed task")??;
        vecs.pop().ok_or_else(|| anyhow!("embedder returned no vector"))
    }

    fn dim(&self) -> usize {
        self.dim
    }
}
