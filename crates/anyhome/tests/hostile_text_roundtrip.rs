//! Hostile-text roundtrip guarantees through the public `MemoryDb` API
//! (TODO-64 / init-pro-11). Contract pinned here: `upsert` binds
//! `(agent_id, key, text)` as parameters (crud.rs `params!`) — never string-
//! interpolated into SQL — so quotes, backslashes, LIKE metacharacters and
//! full SQL snippets are inert data. Every test writes a hostile value and
//! asserts `get_text` returns EXACTLY the same bytes back (`assert_eq` on
//! the whole string), plus key survival in `list_keys` and FTS visibility of
//! a token where applicable.
//!
//! Probe results pinned as tests (spiked first, never assumed):
//! - NUL byte: the binding truncates at the first NUL (TEXT travels through
//!   NUL-terminated C strings). The write succeeds and `get_text` returns
//!   only the prefix before the NUL — no error, no bytes after the NUL.
//! - 1 MiB and 8 MiB texts: exact roundtrip, no limit observed.
//! - 64 KiB key: exact roundtrip (a 1 MiB key also worked in the spike).

use anyhome::db::{dummy_embed, MemoryDb, WhereOutcome};

const DIM: usize = 8;

async fn mem_db() -> MemoryDb {
    MemoryDb::open(":memory:", DIM).await.unwrap()
}

/// Write `text` under `key` and assert the exact-bytes roundtrip.
async fn put(db: &MemoryDb, key: &str, text: &str) {
    db.upsert("", key, text, &dummy_embed(text, DIM))
        .await
        .unwrap();
    assert_eq!(
        db.get_text("", key).await.unwrap(),
        Some(text.to_string()),
        "key {key:?} must roundtrip byte-exactly"
    );
}

/// The row's FTS tokens must be findable via where_search.
async fn fts_finds(db: &MemoryDb, cond: &str, want_key: &str) {
    match db.where_search("", cond, 10).await.unwrap() {
        WhereOutcome::Rows(rows) => {
            assert_eq!(
                rows.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
                [want_key],
                "MATCH {cond:?}"
            );
        }
        WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
    }
}

/// SQL metacharacters in BOTH key and text are inert data: single/double
/// quotes and a full injection attempt roundtrip untouched, and the table
/// outlives the DROP attempt (verified by a live FTS lookup afterwards).
#[tokio::test]
async fn sql_quote_hostiles_roundtrip_in_key_and_text() {
    let db = mem_db().await;
    put(&db, "it's", "it's a quote o'clock").await;
    put(&db, "O\"Brien", "she said \"hi\" twice").await;
    put(&db, "'; DROP TABLE memories;--", "harmless payload").await;

    let mut keys = db.list_keys("").await.unwrap();
    keys.sort();
    assert_eq!(keys, ["'; DROP TABLE memories;--", "O\"Brien", "it's"]);
    fts_finds(&db, "harmless", "'; DROP TABLE memories;--").await;
    fts_finds(&db, "quote", "it's").await;
}

/// Backslashes are plain characters here (no C-style or SQL escaping layer):
/// a Windows-style key and a text full of backslash runs roundtrip exactly.
#[tokio::test]
async fn backslash_heavy_key_and_text_roundtrip() {
    let db = mem_db().await;
    put(&db, "C:\\path\\to\\file", "deep \\ nested \\\\ fin").await;
    let keys = db.list_keys("").await.unwrap();
    assert_eq!(keys, ["C:\\path\\to\\file"]);
    fts_finds(&db, "nested", "C:\\path\\to\\file").await;
}

/// Line breaks are data: `\n` and `\r\n` both survive byte-for-byte (no
/// normalization to a single newline flavor), and the lines stay tokenized.
#[tokio::test]
async fn newline_variants_roundtrip_exactly() {
    let db = mem_db().await;
    put(&db, "nl", "line1\nline2\r\nline3").await;
    assert_eq!(
        db.get_text("", "nl").await.unwrap().as_deref(),
        Some("line1\nline2\r\nline3"),
        "\\r\\n must not be collapsed to \\n"
    );
    fts_finds(&db, "line3", "nl").await;
}

/// A literal TAB in the text is preserved and still separates tokens.
#[tokio::test]
async fn literal_tab_roundtrips_and_separates_tokens() {
    let db = mem_db().await;
    put(&db, "tab", "col1\tcol2").await;
    fts_finds(&db, "col2", "tab").await;
}

/// `%`, `=`, `_` are LIKE/equality metacharacters in SQL but pure data in
/// bound parameters: the hostile key and text roundtrip exactly, and the
/// unicode61 tokenizer splits around them (`100` and `done` stay findable).
#[tokio::test]
async fn like_metacharacters_are_data_not_patterns() {
    let db = mem_db().await;
    put(&db, "100%key", "100% = _done_").await;
    let keys = db.list_keys("").await.unwrap();
    assert_eq!(keys, ["100%key"]);
    fts_finds(&db, "done", "100%key").await;
    fts_finds(&db, "100", "100%key").await;
}

/// Unicode roundtrips byte-exactly: emoji, RTL Hebrew, and a combining
/// sequence (`e` + U+0301). No normalization happens either way — the
/// combining spelling and the precomposed `é` are stored and returned as
/// distinct byte sequences, each under its own key.
#[tokio::test]
async fn unicode_emoji_rtl_and_combining_marks_roundtrip_byte_exact() {
    let db = mem_db().await;
    let combining = "emoji \u{1F980} \u{05E9}\u{05DC}\u{05D5}\u{05DD} e\u{0301} tail";
    let precomposed = "emoji \u{1F980} \u{05E9}\u{05DC}\u{05D5}\u{05DD} \u{00E9} tail";
    assert_ne!(combining, precomposed, "sanity: spellings differ in bytes");
    put(&db, "comb", combining).await;
    put(&db, "pre", precomposed).await;
    // Each spelling comes back as itself, never normalized into the other.
    assert_eq!(db.get_text("", "comb").await.unwrap().as_deref(), Some(combining));
    assert_eq!(db.get_text("", "pre").await.unwrap().as_deref(), Some(precomposed));
    // RTL and post-unicode-6.1-emoji tokens are MATCH-visible; both spellings
    // share those tokens, so both rows come back (measured, key-ordered).
    for cond in ["\u{05E9}\u{05DC}\u{05D5}\u{05DD}", "\u{1F980}"] {
        match db.where_search("", cond, 10).await.unwrap() {
            WhereOutcome::Rows(rows) => {
                let mut got: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
                got.sort_unstable();
                assert_eq!(got, ["comb", "pre"], "MATCH {cond:?}");
            }
            WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
        }
    }
}

/// Measured NUL behavior: SQLite TEXT travels through NUL-terminated C
/// strings, so the first NUL byte truncates the value — the write succeeds,
/// `get_text` returns only the prefix, and a NUL-bearing key is stored under
/// its truncated form (the lookup key is truncated identically, so the row
/// is still found). Pinned because a future driver change could instead
/// error or store the NUL; either shift must be caught here.
#[tokio::test]
async fn nul_byte_truncates_at_the_first_nul() {
    let db = mem_db().await;
    // Text: everything after the NUL is dropped, the prefix survives exactly.
    db.upsert("", "nul-text", "before\0after", &dummy_embed("x", DIM))
        .await
        .unwrap();
    assert_eq!(
        db.get_text("", "nul-text").await.unwrap().as_deref(),
        Some("before"),
        "measured: text truncates at the first NUL, no error"
    );
    // Key: stored as the truncated "nul"; a NUL-bearing lookup truncates the
    // same way and therefore still finds the row.
    db.upsert("", "nul\0key", "plain", &dummy_embed("x", DIM))
        .await
        .unwrap();
    assert_eq!(db.get_text("", "nul\0key").await.unwrap().as_deref(), Some("plain"));
    let mut keys = db.list_keys("").await.unwrap();
    keys.sort();
    assert_eq!(keys, ["nul", "nul-text"]);
}

/// Boundary probe: a full 1 MiB text roundtrips byte-exactly (measured),
/// and the 8 MiB probe shows no hidden limit kicking in at that scale
/// either, so any future size cap will be flagged as a regression.
#[tokio::test]
async fn one_mib_text_roundtrips_exactly_and_8_mib_still_fits() {
    let db = mem_db().await;
    let big = "a".repeat(1024 * 1024);
    put(&db, "big1m", &big).await;
    assert_eq!(db.get_text("", "big1m").await.unwrap().unwrap().len(), 1024 * 1024);

    let big8 = "b".repeat(8 * 1024 * 1024);
    put(&db, "big8m", &big8).await;
    assert_eq!(
        db.get_text("", "big8m").await.unwrap().unwrap().len(),
        8 * 1024 * 1024,
        "measured probe: 8 MiB survives with no observable limit"
    );
}

/// Long keys are not length-capped at any practical size: a 64 KiB key
/// roundtrips exactly and survives in `list_keys` (the spike even measured
/// a 1 MiB key working; 64 KiB is pinned as the standing guarantee).
#[tokio::test]
async fn sixty_four_kib_key_roundtrips() {
    let db = mem_db().await;
    let key = "k".repeat(64 * 1024);
    db.upsert("", &key, "val", &dummy_embed("x", DIM))
        .await
        .unwrap();
    assert_eq!(db.get_text("", &key).await.unwrap().as_deref(), Some("val"));
    assert_eq!(db.list_keys("").await.unwrap(), [key]);
}
