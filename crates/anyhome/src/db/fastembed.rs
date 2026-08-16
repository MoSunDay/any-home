//! Real semantic embedder backed by a local ONNX model via `fastembed`.
//!
//! Model: `sentence-transformers/all-MiniLM-L6-v2` (ONNX), 384-dim, mean-pooled.
//! The model is fetched once from HuggingFace and cached; afterwards it runs
//! fully offline with no network and no API key — matching anyhome's
//! "plain files, no SDK" philosophy.
//!
//! hf-hub (fastembed's downloader) has neither an offline mode nor a timeout,
//! so a download attempt on a hostile network can hang forever.
//! [`FastEmbedder::new`] therefore probes the exact cache layout first and
//! fails fast when the model is missing while `HF_HUB_OFFLINE=1` is set; see
//! `scripts/fetch-model.sh` for the documented prefetch.
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

/// HuggingFace repo id of the default model (must match
/// `EmbeddingModel::AllMiniLML6V2`'s model_code in fastembed).
const MINILM_REPO: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
/// The exact files `scripts/fetch-model.sh` prefetches — the documented
/// offline cache contract.
const MINILM_FILES: &[&str] = &[
    "config.json",
    "model.onnx",
    "special_tokens_map.json",
    "tokenizer_config.json",
    "vocab.txt",
];

/// Cache root fastembed will resolve, replicating its precedence exactly:
/// `$HF_HOME` verbatim (it does NOT append `/hub`, so `HF_HOME` must point at
/// the hub directory itself), else `$FASTEMBED_CACHE_DIR`, else the relative
/// `.fastembed_cache` dir.
fn hf_cache_root(hf_home: Option<&str>, fastembed_cache_dir: Option<&str>) -> std::path::PathBuf {
    match (hf_home, fastembed_cache_dir) {
        (Some(home), _) => std::path::PathBuf::from(home),
        (None, Some(dir)) => std::path::PathBuf::from(dir),
        (None, None) => std::path::PathBuf::from(".fastembed_cache"),
    }
}

/// Model-dir name hf-hub derives from a repo id: `models--<org>--<repo>`.
fn hf_model_dir_name(repo_id: &str) -> String {
    format!("models--{}", repo_id.replace('/', "--"))
}

/// Probe whether `cache_root` holds a complete snapshot of `repo_id`, mimicking
/// hf-hub's `CacheRepo::get` for every file: the commit hash comes from
/// `refs/main` and each file must exist under `snapshots/<hash>/` inside
/// `models--<org>--<repo>`. Any miss or IO error yields `false`.
///
/// The ref content is used UNTRIMMED on purpose: hf-hub reads it with
/// `std::fs::read_to_string` and never strips whitespace, so a trailing newline
/// in `refs/main` points at a snapshot dir hf-hub cannot see — the probe must
/// agree with what hf-hub will actually do, not with what looks complete.
fn cached_snapshot_complete(cache_root: &std::path::Path, repo_id: &str, files: &[&str]) -> bool {
    let model_dir = cache_root.join(hf_model_dir_name(repo_id));
    let Ok(rev) = std::fs::read_to_string(model_dir.join("refs").join("main")) else {
        return false;
    };
    let snapshot = model_dir.join("snapshots").join(rev);
    files.iter().all(|f| snapshot.join(f).exists())
}

/// Outcome of the model-cache probe, deciding whether to proceed offline, fail
/// fast, or warn before fastembed is allowed to touch the network.
enum ModelCacheState {
    /// Snapshot complete: load fully offline, no network involved.
    Cached,
    /// Snapshot missing and `HF_HUB_OFFLINE=1` forbids downloads: fail fast.
    MissingOffline,
    /// Snapshot missing but downloads are allowed: warn, then let it try.
    MissingOnline,
}

/// Pure decision table for [`ModelCacheState`].
fn classify_model_cache(cache_complete: bool, offline: bool) -> ModelCacheState {
    match (cache_complete, offline) {
        (true, _) => ModelCacheState::Cached,
        (false, true) => ModelCacheState::MissingOffline,
        (false, false) => ModelCacheState::MissingOnline,
    }
}

/// Fail-fast gate run before fastembed (and thus hf-hub) is constructed.
///
/// hf-hub 0.4.3 ignores `HF_HUB_OFFLINE` entirely and has no timeout config,
/// so we honor the flag ourselves: probe the exact cache layout fastembed will
/// use, and refuse to start with an actionable error when the model is missing
/// under `HF_HUB_OFFLINE=1`. Without the offline flag a missing cache only
/// warns — fastembed will then attempt a network download.
fn ensure_model_cache_or_fail_fast() -> Result<()> {
    let hf_home = std::env::var("HF_HOME").ok();
    let fastembed_cache_dir = std::env::var("FASTEMBED_CACHE_DIR").ok();
    let offline = std::env::var("HF_HUB_OFFLINE").is_ok_and(|v| v == "1");
    let root = hf_cache_root(hf_home.as_deref(), fastembed_cache_dir.as_deref());
    let complete = cached_snapshot_complete(&root, MINILM_REPO, MINILM_FILES);
    match classify_model_cache(complete, offline) {
        ModelCacheState::Cached => {
            tracing::info!(cache = %root.display(), "fastembed model cache found; loading offline");
            Ok(())
        }
        ModelCacheState::MissingOffline => {
            let model_dir = root.join(hf_model_dir_name(MINILM_REPO));
            anyhow::bail!(
                "fastembed model cache not found under {} and HF_HUB_OFFLINE=1 refuses network \
                 fetches. Prefetch the model with scripts/fetch-model.sh (or point HF_HOME at an \
                 existing HuggingFace hub cache), then retry.",
                model_dir.display()
            )
        }
        ModelCacheState::MissingOnline => {
            tracing::warn!(
                "fastembed model cache not found under {}; a network download will be attempted. \
                 For offline/air-gapped hosts prefetch with scripts/fetch-model.sh and set \
                 HF_HOME + HF_HUB_OFFLINE=1",
                root.display()
            );
            Ok(())
        }
    }
}

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
    ///
    /// Fails fast before touching fastembed when the model cache is missing and
    /// `HF_HUB_OFFLINE=1` is set: hf-hub itself has no offline mode or timeout,
    /// so an unguarded download attempt could hang forever. Prefetch the cache
    /// with `scripts/fetch-model.sh`.
    pub fn new() -> Result<Self> {
        ensure_model_cache_or_fail_fast()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::io::Write;

    use tempfile::tempdir;

    /// Commit pinned by `scripts/fetch-model.sh`; only used as a path segment.
    const SHA: &str = "5f1b8cd78bc4fb444dd171e59b18f3a3af89a079";

    /// Build a fake hub cache: `files` under `snapshots/<SHA>/` and `refs/main`
    /// written byte-exactly as `ref_content` (no newline appended).
    fn write_fake_cache(root: &std::path::Path, ref_content: &str, files: &[&str]) {
        let model_dir = root.join(hf_model_dir_name(MINILM_REPO));
        let snapshot = model_dir.join("snapshots").join(SHA);
        fs::create_dir_all(&snapshot).unwrap();
        for f in files {
            fs::write(snapshot.join(f), b"stub").unwrap();
        }
        fs::create_dir_all(model_dir.join("refs")).unwrap();
        let mut refs_main = fs::File::create(model_dir.join("refs").join("main")).unwrap();
        refs_main.write_all(ref_content.as_bytes()).unwrap();
    }

    #[test]
    fn cache_root_prefers_hf_home() {
        let root = hf_cache_root(Some("/hf/home"), Some("/fe/dir"));
        assert_eq!(root, std::path::PathBuf::from("/hf/home"));
    }

    #[test]
    fn cache_root_falls_back_to_fastembed_cache_dir() {
        let root = hf_cache_root(None, Some("/fe/dir"));
        assert_eq!(root, std::path::PathBuf::from("/fe/dir"));
    }

    #[test]
    fn cache_root_defaults_to_dot_fastembed_cache() {
        let root = hf_cache_root(None, None);
        assert_eq!(root, std::path::PathBuf::from(".fastembed_cache"));
    }

    #[test]
    fn snapshot_probe_accepts_complete_cache() {
        let dir = tempdir().unwrap();
        write_fake_cache(dir.path(), SHA, MINILM_FILES);
        assert!(cached_snapshot_complete(dir.path(), MINILM_REPO, MINILM_FILES));
    }

    #[test]
    fn snapshot_probe_rejects_missing_file() {
        // Every file but `vocab.txt` — the kind of partial cache an
        // interrupted prefetch leaves behind.
        let dir = tempdir().unwrap();
        write_fake_cache(dir.path(), SHA, &MINILM_FILES[..4]);
        assert!(!cached_snapshot_complete(dir.path(), MINILM_REPO, MINILM_FILES));
    }

    #[test]
    fn snapshot_probe_rejects_missing_refs_main() {
        let dir = tempdir().unwrap();
        write_fake_cache(dir.path(), SHA, MINILM_FILES);
        let refs_main = dir
            .path()
            .join(hf_model_dir_name(MINILM_REPO))
            .join("refs")
            .join("main");
        fs::remove_file(refs_main).unwrap();
        assert!(!cached_snapshot_complete(dir.path(), MINILM_REPO, MINILM_FILES));
    }

    #[test]
    fn snapshot_probe_rejects_trailing_newline_in_refs_main() {
        // hf-hub reads refs/main UNTRIMMED, so "sha\n" resolves to a snapshot
        // dir with a newline in its name that cannot exist. The probe must
        // agree with hf-hub and reject such a poisoned cache.
        let dir = tempdir().unwrap();
        write_fake_cache(dir.path(), &format!("{SHA}\n"), MINILM_FILES);
        assert!(!cached_snapshot_complete(dir.path(), MINILM_REPO, MINILM_FILES));
    }

    #[test]
    fn classify_model_cache_truth_table() {
        assert!(matches!(classify_model_cache(true, false), ModelCacheState::Cached));
        assert!(matches!(classify_model_cache(false, true), ModelCacheState::MissingOffline));
        assert!(matches!(classify_model_cache(false, false), ModelCacheState::MissingOnline));
    }
}
