//! FTS5 semantics ported from the libsql-dependency TCL suites onto the public
//! `MemoryDb` API (TODO-59 / init-pro-10): fts5content.test (§2.6 UPDATE
//! visibility, §9 DELETE index sync), fts5rank.test (6.x term-frequency
//! ordering), fts5rebuild.test (1.5-1.7 shadow-table wipe + rebuild recovery)
//! and fts5ea.test (§3 malformed-expression classification). Every assertion
//! is the *measured* behavior of the bundled libsql FTS5, not assumed.

use anyhome::db::{dummy_embed, MemoryDb, WhereOutcome};
use libsql::{params, Builder};

const DIM: usize = 8;

/// Unwrap a where_search into rows (panics on InvalidFts/Err).
async fn rows(db: &MemoryDb, agent: &str, cond: &str) -> Vec<(String, String)> {
    match db.where_search(agent, cond, 50).await.unwrap() {
        WhereOutcome::Rows(r) => r,
        WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
    }
}

fn keys(rows: &[(String, String)]) -> Vec<&str> {
    rows.iter().map(|(k, _)| k.as_str()).collect()
}

async fn mem_db() -> MemoryDb {
    MemoryDb::open(":memory:", DIM).await.unwrap()
}

/// fts5content.test §2.5-2.6: after an UPDATE, MATCH sees the new content and
/// the old tokens stop matching. anyhome path: re-upsert on the same
/// `(agent, key)` fires the `memories_au` trigger.
#[tokio::test]
async fn upsert_on_same_key_switches_fts_visibility_to_new_text() {
    let db = mem_db().await;
    db.upsert("", "note", "one two three", &dummy_embed("x", DIM)).await.unwrap();
    assert_eq!(keys(&rows(&db, "", "one AND two").await), ["note"]);
    db.upsert("", "note", "three x y z", &dummy_embed("x", DIM)).await.unwrap();
    assert_eq!(keys(&rows(&db, "", "three AND x").await), ["note"]);
    assert!(rows(&db, "", "one").await.is_empty(), "old token must be gone");
    assert!(rows(&db, "", "two").await.is_empty());
}

/// fts5content.test §9: deleting a content row keeps the FTS index in sync.
/// In anyhome this is load-bearing on the `memories_ad` trigger (the TCL
/// suite's §9 shows the stale-index failure mode when the trigger is absent).
#[tokio::test]
async fn delete_removes_key_from_fts_matches() {
    let db = mem_db().await;
    db.upsert("", "a", "one two three", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "b", "one two three", &dummy_embed("x", DIM)).await.unwrap();
    assert_eq!(rows(&db, "", "two").await.len(), 2);
    db.delete("", "b").await.unwrap();
    assert_eq!(keys(&rows(&db, "", "two").await), ["a"]);
}

/// fts5rank.test 6.0-6.1: ORDER BY rank puts the higher term frequency first.
#[tokio::test]
async fn rank_orders_higher_term_frequency_first() {
    let db = mem_db().await;
    db.upsert("", "hi", "table table table", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo1", "the table names", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo2", "rank on an fts5 table", &dummy_embed("x", DIM)).await.unwrap();
    let res = rows(&db, "", "table").await;
    assert_eq!(keys(&res), ["hi", "lo1", "lo2"], "three 'table's outrank one: {res:?}");
}

/// Equal bm25 scores keep a stable order: repeating the identical query must
/// return the identical ordering every time.
#[tokio::test]
async fn rank_tie_order_is_stable_across_repeated_queries() {
    let db = mem_db().await;
    db.upsert("", "hi", "table table table", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo1", "the table names", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo2", "rank on an fts5 table", &dummy_embed("x", DIM)).await.unwrap();
    let first = rows(&db, "", "table").await;
    for _ in 0..3 {
        assert_eq!(rows(&db, "", "table").await, first);
    }
}

/// fts5content.test §3.1 spirit: an OR arm that matches nothing must not
/// change the result set or the ordering of the arm that does match.
#[tokio::test]
async fn or_with_zero_hit_token_behaves_like_the_real_term_alone() {
    let db = mem_db().await;
    db.upsert("", "hi", "table table table", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo1", "the table names", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "lo2", "rank on an fts5 table", &dummy_embed("x", DIM)).await.unwrap();
    let plain = rows(&db, "", "table").await;
    assert_eq!(rows(&db, "", "table OR zzznomatch").await, plain);
}

/// fts5rebuild.test 1.5-1.7: with the shadow tables emptied the index is
/// broken (MATCH fails with "database disk image is malformed"), and the
/// 'rebuild' command restores full search from the content table.
///
/// Measured anyhome specifics: `memories_fts` is an external-content table
/// (content='memories'), so `SELECT count(*) FROM memories_fts` mirrors the
/// content table and never reads 0 — the open-time `ensure_fts_populated`
/// guard therefore does NOT auto-rebuild after a shadow wipe. Recovery uses
/// the upstream mechanism itself: the 'rebuild' command run through a raw
/// libsql connection (no src/ changes).
#[tokio::test]
async fn cleared_shadow_tables_are_recovered_by_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fts.db");
    let p = path.to_str().unwrap();

    let db = MemoryDb::open(p, DIM).await.unwrap();
    db.upsert("", "k1", "alpha beta", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "k2", "gamma delta", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "k3", "alpha omega", &dummy_embed("x", DIM)).await.unwrap();
    drop(db);

    // fts5rebuild 1.5: wipe the fts5 shadow tables behind the DB's back
    // (defensive mode is not enabled for libsql connections, so this works).
    let raw = Builder::new_local(p).build().await.unwrap();
    let conn = raw.connect().unwrap();
    for t in [
        "DELETE FROM memories_fts_data",
        "DELETE FROM memories_fts_idx",
        "DELETE FROM memories_fts_docsize",
    ] {
        conn.execute(t, ()).await.unwrap();
    }
    // fts5rebuild 1.6: the index is now unusable — corruption, not a user
    // query error (exactly what classify_fts_error must NOT swallow). Like
    // all FTS5 expression errors, it surfaces at row-fetch time, not at
    // statement prepare.
    let mut corrupt = conn
        .query("SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?", params!["alpha"])
        .await
        .expect("prepare succeeds; corruption surfaces on fetch");
    let err = corrupt.next().await.expect_err("cleared shadow tables must break MATCH");
    let msg = err.to_string();
    assert!(msg.contains("malformed"), "expected corruption message, got: {msg}");

    // fts5rebuild 1.7: 'rebuild' re-derives the index from content='memories'.
    conn.execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", ())
        .await
        .unwrap();
    drop(conn);
    drop(raw);

    let db = MemoryDb::open(p, DIM).await.unwrap();
    assert_eq!(keys(&rows(&db, "", "alpha").await), ["k1", "k3"]);
    assert_eq!(keys(&rows(&db, "", "delta").await), ["k2"]);
    assert_eq!(db.list_keys("").await.unwrap(), vec!["k1", "k2", "k3"]);
}

/// fts5ea.test §3.1,3.2,3.10,3.4,3.11,§5.0,3.5: malformed MATCH expressions
/// are classified as InvalidFts carrying the offending condition back.
#[tokio::test]
async fn malformed_match_expressions_become_invalid_fts() {
    let db = mem_db().await;
    db.upsert("", "d", "abc def", &dummy_embed("x", DIM)).await.unwrap();
    let bad = [
        "AND",
        "abc def AND",
        "NOT",
        "(a OR b) abc",
        "a AND \"abc",
        "abc | def",
        "NEaR (a b)",
    ];
    for bad in bad {
        match db.where_search("", bad, 10).await.unwrap() {
            WhereOutcome::InvalidFts(returned) => assert_eq!(returned, bad),
            WhereOutcome::Rows(r) => panic!("{bad:?} must be InvalidFts, got rows {r:?}"),
        }
    }
}

/// fts5ea.test §3.12 (`NEAR(a b, xyz)` -> `expected integer, got "xyz"`).
/// Measured deviation: that message starts with none of the classify_fts_error
/// whitelisted prefixes ("fts5:", "unterminated string", "unknown special
/// query"), so anyhome propagates it as a hard Err instead of InvalidFts.
#[tokio::test]
async fn near_with_non_integer_distance_propagates_as_err() {
    let db = mem_db().await;
    let err = db.where_search("", "NEAR(a b, xyz)", 10).await.unwrap_err();
    assert!(format!("{err:#}").contains("expected integer"), "{err:#}");
}

/// fts5ea.test §3.8-3.9 (`nosuch: a ...` -> `no such column: nosuch`).
/// Measured: "no such column:" matches no classify_fts_error prefix, so it is
/// deliberately propagated as Err — a missing column is not part of the MATCH
/// syntax-error classification. Contrast: a filter on the real `text` column
/// is a valid expression and returns rows.
#[tokio::test]
async fn unknown_column_filter_is_err_not_invalid_fts() {
    let db = mem_db().await;
    db.upsert("", "d", "hello world", &dummy_embed("x", DIM)).await.unwrap();
    let err = db.where_search("", "nosuchcol: hello", 10).await.unwrap_err();
    assert!(format!("{err:#}").contains("no such column"), "{err:#}");
    assert_eq!(keys(&rows(&db, "", "text: hello").await), ["d"]);
}

/// Empty / whitespace-only conditions short-circuit to empty Rows — never
/// InvalidFts, never a DB error.
#[tokio::test]
async fn blank_condition_is_empty_rows_not_invalid() {
    let db = mem_db().await;
    db.upsert("", "d", "anything", &dummy_embed("x", DIM)).await.unwrap();
    for cond in ["", "   "] {
        assert_eq!(db.where_search("", cond, 10).await.unwrap(), WhereOutcome::Rows(vec![]));
    }
}

/// A thousand-token document roundtrips intact and stays searchable,
/// including for a token at the very end.
#[tokio::test]
async fn large_text_roundtrip_stays_searchable() {
    let db = mem_db().await;
    let mut text = String::new();
    for i in 0..1000 {
        use std::fmt::Write as _;
        write!(text, "tok{i} ").unwrap(); // String write cannot fail
    }
    db.upsert("", "big", &text, &dummy_embed(&text, DIM)).await.unwrap();
    assert_eq!(keys(&rows(&db, "", "tok999").await), ["big"]);
    assert_eq!(keys(&rows(&db, "", "tok0 AND tok999").await), ["big"]);
    assert_eq!(db.get_text("", "big").await.unwrap().unwrap(), text);
}

/// Unicode roundtrip, measured unicode61 tokenizer behavior: CJK runs are
/// single tokens (no word segmentation); symbols known to Unicode 6.1 (🔥)
/// are separators — invisible to MATCH but intact behind get_text; emoji
/// added after Unicode 6.1 (🦀) fall outside the tokenizer's tables and are
/// treated as token characters.
#[tokio::test]
async fn unicode_text_roundtrips_per_unicode61_tokenizer_rules() {
    let db = mem_db().await;
    db.upsert("", "cn-run", "数据库测试环境", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "cn-sep", "数据库 测试 环境", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "crab", "crab emoji 🦀 here", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "fire", "纯表情 🔥🔥 火焰", &dummy_embed("x", DIM)).await.unwrap();
    // Unsegmented CJK is one token: the full run matches, substrings do not.
    assert_eq!(keys(&rows(&db, "", "数据库测试环境").await), ["cn-run"]);
    // Space-separated CJK: each word is its own token.
    assert_eq!(keys(&rows(&db, "", "数据库").await), ["cn-sep"]);
    assert_eq!(keys(&rows(&db, "", "测试").await), ["cn-sep"]);
    // Post-6.1 emoji behaves as a token character...
    assert_eq!(keys(&rows(&db, "", "🦀").await), ["crab"]);
    // ...while 🔥 (known symbol) yields zero tokens: MATCH misses, but the
    // text is fully intact behind get_text; sibling CJK tokens still match.
    assert!(rows(&db, "", "🔥").await.is_empty());
    assert_eq!(keys(&rows(&db, "", "火焰").await), ["fire"]);
    assert_eq!(db.get_text("", "fire").await.unwrap().as_deref(), Some("纯表情 🔥🔥 火焰"));
}
