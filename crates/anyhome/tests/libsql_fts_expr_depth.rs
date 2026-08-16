//! FTS5 MATCH-expression depth limits, pinned against the bundled libsql
//! (TODO-64 / init-pro-11). Upstream context:
//! - `link_repos/libsql/libsql-sqlite3/ext/fts5/test/fts5expr.test` §1: a NOT
//!   chain longer than 256 nodes fails with "fts5 expression tree is too
//!   large (maximum depth 256)".
//! - `link_repos/libsql/libsql-sqlite3/ext/fts5/test/fts5limits.test` 1.1-1.6:
//!   260-term AND/OR chains stay flat; a 260-term NOT chain errors.
//!
//! Spike findings (all measured via `where_search`, none assumed):
//! - MATCH strings are parsed by FTS5's own lemon parser (fts5parse.y), not
//!   the SQL parser, so SQLITE_MAX_EXPR_DEPTH does not apply. That parser has
//!   a fixed stack (lemon default YYSTACKDEPTH=100, tool/lempar.c): nested
//!   parens work up to 97 levels and at 98 raise
//!   `fts5: parser stack overflow` -> classified `InvalidFts`.
//! - The built expression tree is capped at SQLITE_FTS5_MAX_EXPR_DEPTH=256
//!   (fts5_expr.c). A NOT chain adds one node per NOT, so 255 NOTs (height
//!   256) work and 256 fail with `fts5 expression tree is too large
//!   (maximum depth 256)`. That message has NO "fts5:" prefix, so
//!   `classify_fts_error` does not see it as user-caused and surfaces it as
//!   `Err` rather than `InvalidFts` — the asymmetry is pinned below.
//! - AND/OR children of the same kind are merged (fts5ExprAddChildren), so
//!   10000-term chains stay at height 2 and match fine (measured; upstream
//!   only went to 260).
//! - NEAR is not nestable at any depth: the grammar takes one flat nearset,
//!   so `NEAR(NEAR(a b) c)` is a syntax error (`fts5: syntax error near "("`)
//!   -> `InvalidFts`; flat `NEAR(a b c)` matches.
//! - No construction measured here aborts the process; every failure is a
//!   clean `InvalidFts`/`Err` outcome, so no subprocess probe is needed
//!   (unlike the vector-index k=0 abort pinned in libsql_vector_semantics.rs).

use anyhome::db::{dummy_embed, MemoryDb, WhereOutcome};

const DIM: usize = 8;

async fn mem_db() -> MemoryDb {
    let db = MemoryDb::open(":memory:", DIM).await.unwrap();
    db.upsert("", "d", "alpha beta", &dummy_embed("x", DIM))
        .await
        .unwrap();
    db
}

/// Unwrap a where_search into rows (panics on InvalidFts/Err).
async fn rows(db: &MemoryDb, cond: &str) -> Vec<(String, String)> {
    match db.where_search("", cond, 10).await.unwrap() {
        WhereOutcome::Rows(r) => r,
        WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
    }
}

fn keys(rows: &[(String, String)]) -> Vec<&str> {
    rows.iter().map(|(k, _)| k.as_str()).collect()
}

fn parens(n: usize) -> String {
    format!("{}alpha{}", "(".repeat(n), ")".repeat(n))
}

fn not_chain(n: usize) -> String {
    format!("alpha {}", vec!["NOT zz"; n].join(" zz "))
}

/// Spike: nested parens add one parser-stack entry per level; the bundled
/// lemon parser's fixed stack holds them up to 97 levels. Every level below
/// the limit still matches the single term inside.
#[tokio::test]
async fn nested_parens_up_to_97_levels_still_match() {
    let db = mem_db().await;
    for n in [1usize, 16, 97] {
        assert_eq!(
            keys(&rows(&db, &parens(n)).await),
            ["d"],
            "{n} nested paren levels must still match"
        );
    }
}

/// Measured threshold: at 98 nested levels the FTS parser stack overflows
/// (`fts5: parser stack overflow`). The message starts with `fts5:`, so the
/// failure is classified as `InvalidFts` carrying the offending condition
/// back to the caller — not an opaque `Err`, and no process crash.
#[tokio::test]
async fn paren_nesting_at_98_levels_is_invalid_fts() {
    let db = mem_db().await;
    for n in [98usize, 99, 200] {
        let cond = parens(n);
        match db.where_search("", &cond, 10).await.unwrap() {
            WhereOutcome::InvalidFts(returned) => {
                assert_eq!(returned, cond, "the offending condition travels back");
            }
            WhereOutcome::Rows(rows) => {
                panic!("{n} nested parens must overflow the parser stack: {rows:?}")
            }
        }
    }
}

/// fts5limits.test 1.4/1.6 (extended by spike): same-kind AND/OR children are
/// merged into one flat node, so chain length never feeds the depth limit.
/// Upstream stopped at 260 terms; the spike measured 10000 AND terms working.
#[tokio::test]
async fn and_or_chains_flatten_so_length_is_not_depth() {
    let db = mem_db().await;
    for (label, cond) in [
        ("AND x260", vec!["alpha"; 260].join(" AND ")),
        ("OR x260", vec!["alpha"; 260].join(" OR ")),
        ("AND x10000", vec!["alpha"; 10000].join(" AND ")),
    ] {
        assert_eq!(keys(&rows(&db, &cond).await), ["d"], "{label} must match");
    }
}

/// fts5expr.test §1 lower side: a NOT chain adds one tree node per NOT, and
/// 255 NOTs build height 256 — exactly the limit — so they still match.
#[tokio::test]
async fn not_chain_of_255_terms_is_the_last_depth_that_matches() {
    let db = mem_db().await;
    assert_eq!(keys(&rows(&db, &not_chain(255)).await), ["d"]);
}

/// Measured threshold and classification asymmetry: the 256-NOT chain exceeds
/// SQLITE_FTS5_MAX_EXPR_DEPTH (fts5_expr.c) and SQLite reports `fts5
/// expression tree is too large (maximum depth 256)`. That message starts
/// with `fts5 ` (no colon), so `classify_fts_error` treats it as a real DB
/// error: callers get `Err`, not `InvalidFts`. The message text is pinned so
/// a future reclassification change is detected.
#[tokio::test]
async fn not_chain_of_256_terms_surfaces_as_err_not_invalid_fts() {
    let db = mem_db().await;
    match db.where_search("", &not_chain(256), 10).await {
        Err(e) => {
            let msg = format!("{e:#}");
            assert!(
                msg.contains("fts5 expression tree is too large (maximum depth 256)"),
                "unexpected error text: {msg}"
            );
        }
        Ok(WhereOutcome::InvalidFts(bad)) => {
            panic!("the depth-limit error is not classified as InvalidFts: {bad:?}")
        }
        Ok(WhereOutcome::Rows(rows)) => panic!("256 NOTs must not match: {rows:?}"),
    }
}

/// NEAR takes one flat nearset — nesting is never accepted at any depth.
/// Flat NEAR (2 and 3 phrases) matches; every nested spelling is a syntax
/// error, and `fts5: syntax error near ...` is user-caused -> `InvalidFts`.
#[tokio::test]
async fn flat_near_matches_nested_near_is_invalid_fts() {
    let db = MemoryDb::open(":memory:", DIM).await.unwrap();
    db.upsert("", "d", "alpha beta gamma delta", &dummy_embed("x", DIM))
        .await
        .unwrap();
    assert_eq!(keys(&rows(&db, "NEAR(alpha beta)").await), ["d"]);
    assert_eq!(keys(&rows(&db, "NEAR(alpha beta gamma)").await), ["d"]);
    for bad in [
        "NEAR(NEAR(alpha beta) gamma)",
        "NEAR(alpha NEAR(beta gamma))",
        "NEAR()",
    ] {
        match db.where_search("", bad, 10).await.unwrap() {
            WhereOutcome::InvalidFts(returned) => assert_eq!(returned, bad),
            WhereOutcome::Rows(rows) => panic!("nested/empty NEAR must not match: {rows:?}"),
        }
    }
}

/// Phrase length has no depth dimension: a 20000-token quoted phrase parses
/// and matches (measured; a 100-token phrase too). All tokens live in the
/// same phrase node, so the whole document is one hit either way.
#[tokio::test]
async fn long_phrases_match_regardless_of_term_count() {
    let db = MemoryDb::open(":memory:", DIM).await.unwrap();
    let text = (0..20000).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ");
    db.upsert("", "d", &text, &dummy_embed("x", DIM))
        .await
        .unwrap();
    for n in [100usize, 20000] {
        let phrase = format!(
            "\"{}\"",
            (0..n).map(|i| format!("t{i}")).collect::<Vec<_>>().join(" ")
        );
        assert_eq!(
            keys(&rows(&db, &phrase).await),
            ["d"],
            "a {n}-token phrase must match"
        );
    }
}
