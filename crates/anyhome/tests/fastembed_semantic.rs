#![cfg(feature = "embed-fastembed")]
//! Real semantic embedder acceptance (TODO-10).
//!
//! Runs only with `--features embed-fastembed` and a loadable model. The model
//! is fetched once (HuggingFace) and cached; if it cannot be loaded (e.g. a
//! sandbox with no network and no prefetched cache) the tests self-skip rather
//! than turn the suite red — mirroring how `scripts/e2e.sh` skips without
//! `mount.nfs`. See `scripts/fetch-model.sh` to prefetch the model offline.

use std::sync::OnceLock;

use anyhome::db::Embedder;
use anyhome::FastEmbedder;

fn cos(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// Load the model once, pointing at the prefetched cache when present so an
/// offline sandbox with the model prefetched (see scripts/fetch-model.sh) works
/// without network.
fn model() -> Option<&'static FastEmbedder> {
    static MODEL: OnceLock<Option<FastEmbedder>> = OnceLock::new();
    MODEL.get_or_init(||{
        // Prefer a prefetched standard HF cache layout if it exists.
        if std::env::var("HF_HOME").is_err() {
            if let Ok(home) = std::env::var("HOME") {
                let hub = format!("{home}/.cache/huggingface/hub");
                let cached = std::path::Path::new(&hub)
                    .join("models--Qdrant--all-MiniLM-L6-v2-onnx");
                if cached.exists() {
                    std::env::set_var("HF_HOME", hub);
                    std::env::set_var("HF_HUB_OFFLINE", "1");
                }
            }
        }
        match FastEmbedder::new() {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!(
                    "[fastembed_semantic] model unavailable; skipping real-embedder tests: {e}"
                );
                None
            }
        }
    }).as_ref()
}

#[tokio::test]
async fn dim_is_384() {
    let m = match model() {
        Some(m) => m,
        None => return,
    };
    assert_eq!(m.dim(), 384);
}

#[tokio::test]
async fn semantic_inequality_holds() {
    // The hard acceptance for TODO-10: a synonym/greeting is closer to "hello"
    // than an unrelated word. DummyEmbedder cannot satisfy this.
    let m = match model() {
        Some(m) => m,
        None => return,
    };
    let hello = m.embed("hello").await.unwrap();
    let hi = m.embed("hi").await.unwrap();
    let greetings = m.embed("greetings").await.unwrap();
    let banana = m.embed("banana").await.unwrap();

    assert_eq!(hello.len(), 384);
    let c_hi = cos(&hello, &hi);
    let c_greet = cos(&hello, &greetings);
    let c_banana = cos(&hello, &banana);
    eprintln!("cos(hello,hi)={c_hi:.3} cos(hello,greetings)={c_greet:.3} cos(hello,banana)={c_banana:.3}");
    assert!(
        c_hi > c_banana,
        "hello~hi ({c_hi:.3}) must beat hello~banana ({c_banana:.3})"
    );
    assert!(
        c_greet > c_banana,
        "hello~greetings ({c_greet:.3}) must beat hello~banana ({c_banana:.3})"
    );
}

#[tokio::test]
async fn embeddings_are_deterministic() {
    let m = match model() {
        Some(m) => m,
        None => return,
    };
    let a = m.embed("determinism check").await.unwrap();
    let b = m.embed("determinism check").await.unwrap();
    assert_eq!(a, b, "same input must yield identical vectors");
}
