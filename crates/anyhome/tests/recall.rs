//! Recall evaluation scaffold (TODO-13).
//!
//! A tiny hand-labeled (query, expected_key) set measures semantic retrieval
//! quality. The non-gated baseline proves the scaffold is meaningful by showing
//! the DummyEmbedder (lexical/exact-match) cannot answer paraphrased questions;
//! the gated test proves the real FastEmbedder clears the bar.

use anyhome::db::{DummyEmbedder, Embedder, MemoryDb};

/// (key, stored text) — the indexed corpus.
fn corpus() -> Vec<(&'static str, &'static str)> {
    // Terse keyword phrases (no articles/stopwords) so lexical overlap reflects
    // real vocabulary, not filler words.
    vec![
        ("paris", "France capital city Paris"),
        ("tokyo", "Japan capital city Tokyo"),
        ("python", "data science machine learning Python"),
        ("rust", "memory safety systems programming Rust"),
        ("water", "water boiling point Celsius temperature"),
        ("gravity", "gravitational acceleration falling objects downward"),
        ("photosynthesis", "plants sunlight energy photosynthesis"),
        ("dns", "domain name system hostname IP mapping"),
        ("inflation", "rising prices economy inflation"),
        ("everest", "Everest tallest mountain Earth"),
    ]
}

/// (paraphrased query, expected key) — deliberately low surface overlap so a
/// purely lexical embedder struggles.
fn questions() -> Vec<(&'static str, &'static str)> {
    // Paraphrased with deliberately different vocabulary from the target doc,
    // so a purely lexical embedder cannot match them.
    vec![
        ("french government seat", "paris"),
        ("japanese metropolitan hub", "tokyo"),
        ("artificial intelligence scripting", "python"),
        ("null pointer prevention", "rust"),
        ("h2o vaporization heat", "water"),
        ("apples dropping orchards", "gravity"),
        ("botanical light harvesting", "photosynthesis"),
        ("website address resolution", "dns"),
        ("currency purchasing power loss", "inflation"),
        ("planetary summit elevation", "everest"),
    ]
}

/// recall@k = fraction of questions whose expected key appears in the top-k.
async fn recall_at_k<E: Embedder + Sync>(emb: &E, dim: usize, k: usize) -> (usize, usize) {
    let db = MemoryDb::open(":memory:", dim).await.unwrap();
    for (key, text) in corpus() {
        let v = emb.embed(text).await.unwrap();
        db.upsert(key, text, &v).await.unwrap();
    }
    let mut hits = 0;
    for (q, expected) in questions() {
        let qv = emb.embed(q).await.unwrap();
        let res = db.search(&qv, k as i64).await.unwrap();
        if res.iter().any(|(key, _)| key == expected) {
            hits += 1;
        }
    }
    (hits, questions().len())
}

#[tokio::test]
async fn dummy_embedder_baseline_fails_paraphrase_recall() {
    // The scaffold must be able to *fail* a non-semantic embedder: DummyEmbedder
    // matches on token overlap, so paraphrased questions it cannot answer keep
    // recall@3 low. This is the proof the scaffold discriminates.
    let emb = DummyEmbedder::new(64);
    let (hits, total) = recall_at_k(&emb, 64, 3).await;
    let recall = hits as f32 / total as f32;
    eprintln!("[recall/dummy] recall@3 = {hits}/{total} = {recall:.2}");
    assert!(
        recall < 0.5,
        "DummyEmbedder must not pass the semantic bar (got {recall:.2}); \
         otherwise the scaffold cannot prove anything"
    );
}

#[cfg(feature = "embed-fastembed")]
mod fastembed {
    use super::recall_at_k;
    use anyhome::db::Embedder;
    use anyhome::FastEmbedder;
    use std::sync::OnceLock;

    fn model() -> Option<&'static FastEmbedder> {
        static MODEL: OnceLock<Option<FastEmbedder>> = OnceLock::new();
        MODEL.get_or_init(|| {
            if std::env::var("HF_HOME").is_err() {
                if let Ok(home) = std::env::var("HOME") {
                    let hub = format!("{home}/.cache/huggingface/hub");
                    if std::path::Path::new(&hub)
                        .join("models--Qdrant--all-MiniLM-L6-v2-onnx")
                        .exists()
                    {
                        std::env::set_var("HF_HOME", hub);
                        std::env::set_var("HF_HUB_OFFLINE", "1");
                    }
                }
            }
            match FastEmbedder::new() {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("[recall/fastembed] model unavailable; skipping: {e}");
                    None
                }
            }
        }).as_ref()
    }

    #[tokio::test]
    async fn fastembed_clears_recall_bar() {
        let m = match model() {
            Some(m) => m,
            None => return,
        };
        let (hits, total) = recall_at_k(m, m.dim(), 3).await;
        let recall = hits as f32 / total as f32;
        eprintln!("[recall/fastembed] recall@3 = {hits}/{total} = {recall:.2}");
        assert!(
            recall >= 0.7,
            "fastembed recall@3 must clear the bar (got {recall:.2})"
        );
    }
}
