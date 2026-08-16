//! unicode61 tokenizer semantics ported from the libsql TCL suites onto the
//! public `MemoryDb` API (TODO-63 / init-pro-11): fts5unicode.test (§1.x
//! case folding + separator tokenization expectations, §2.x proof that the
//! default tokenizer is unicode61) and fts5umlaut.test (§1.x diacritic
//! matrix — the default `remove_diacritics` = 1 half of it). Every
//! assertion is the *measured* behavior of the bundled libsql FTS5 through
//! `where_search`; the fixed schema (single-column `text` table, no
//! tokenizer options) only ever exercises the default unicode61 config.
//!
//! Excluded from these suites (no surface against the fixed anyhome schema):
//! - fts5unicode.test §1.x ascii-tokenizer rows and §2.x t3 comparison:
//!   `tokenize = ascii` is not reachable through the public API.
//! - fts5unicode.test §3.x: `separators` / `tokenchars` tokenizer options.
//! - fts5umlaut.test t2 side: `remove_diacritics 2` table configuration.

use anyhome::db::{dummy_embed, MemoryDb, WhereOutcome};

const DIM: usize = 8;

/// Unwrap a where_search into rows (panics on InvalidFts/Err).
async fn rows(db: &MemoryDb, agent: &str, cond: &str) -> Vec<(String, String)> {
    match db.where_search(agent, cond, 50).await.unwrap() {
        WhereOutcome::Rows(r) => r,
        WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
    }
}

/// Sorted keys: membership, not rank order, is what these tests pin.
fn keyset(rows: &[(String, String)]) -> Vec<&str> {
    let mut ks: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    ks.sort_unstable();
    ks
}

async fn mem_db() -> MemoryDb {
    MemoryDb::open(":memory:", DIM).await.unwrap()
}

/// Expect the condition to be classified as InvalidFts (malformed MATCH).
async fn invalid(db: &MemoryDb, cond: &str) {
    match db.where_search("", cond, 50).await.unwrap() {
        WhereOutcome::InvalidFts(_) => {}
        other => panic!("expected InvalidFts for {cond:?}, got {other:?}"),
    }
}

/// Fixture covering case, diacritics, punctuation, digits and Cyrillic.
async fn fixture() -> MemoryDb {
    let db = mem_db().await;
    for (k, t) in [
        ("case-lower", "the lazy dog sleeps"),
        ("case-upper", "RUST IS FAST"),
        ("um-plain", "uber cool stuff"),
        ("um-umlaut", "\u{00fc}ber alles"), // über alles
        ("cafe-plain", "le cafe noir"),
        ("cafe-acute", "un caf\u{e9} au lait"), // un café au lait
        ("punct", "foo-bar baz.qux 'quoted' words"),
        ("under", "foo_bar baz"),
        ("digit", "x86 arch abc123 token"),
        ("cyr", "\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442} \u{43c}\u{438}\u{440}"), // привет мир
        ("cyr-upper", "\u{411}\u{41e}\u{41b}\u{42c}\u{428}\u{41e}\u{419} \u{41c}\u{418}\u{420}"), // БОЛЬШОЙ МИР
        ("hanoi", "Ha Noi"),
    ] {
        db.upsert("", k, t, &dummy_embed(t, DIM)).await.unwrap();
    }
    db
}

/// fts5unicode.test §1.$tn.0-1 ('A B C D' -> a b c d, 'May you share
/// freely,' lowercased): case folding applies to both query and document
/// text, in both directions.
#[tokio::test]
async fn case_folding_is_applied_to_query_and_doc() {
    let db = fixture().await;
    for q in ["LAZY", "Lazy", "lazy"] {
        assert_eq!(keyset(&rows(&db, "", q).await), ["case-lower"], "query {q:?}");
    }
    // Uppercase doc text is found via lowercase/mixed-case queries.
    for q in ["rust", "RuSt"] {
        assert_eq!(keyset(&rows(&db, "", q).await), ["case-upper"], "query {q:?}");
    }
}

/// fts5umlaut.test (über/uber pair, t1 side): with the default
/// remove_diacritics=1 the umlaut folds to its ASCII base in both
/// directions, so plain and umlauted spellings are interchangeable.
#[tokio::test]
async fn umlaut_folds_to_ascii_base_both_ways() {
    let db = fixture().await;
    for q in ["uber", "\u{00fc}ber", "UBER"] {
        assert_eq!(
            keyset(&rows(&db, "", q).await),
            ["um-plain", "um-umlaut"],
            "query {q:?}"
        );
    }
}

/// fts5umlaut.test pattern applied to é (fts5unicode case folding + default
/// diacritic removal): cafe/café/CAFÉ all hit both spellings.
#[tokio::test]
async fn acute_e_folds_to_ascii_e() {
    let db = fixture().await;
    for q in ["cafe", "caf\u{e9}", "CAF\u{c9}"] {
        assert_eq!(
            keyset(&rows(&db, "", q).await),
            ["cafe-acute", "cafe-plain"],
            "query {q:?}"
        );
    }
}

/// fts5umlaut.test §1.1-1.7 (res1 column, default tokenizer): doc 'Ha Noi'
/// vs. the query matrix. Measured rule: combining marks are always skipped
/// by unicode61, so decomposed spellings (o + U+0302 / U+0323) collapse to
/// "noi"; but the *precomposed* ộ (U+1ED9) is only folded to 'o' under
/// remove_diacritics=2, so with the default config it does not match —
/// exactly upstream's res1 values (0,1,1,0,1,1).
#[tokio::test]
async fn hanoi_query_matrix_matches_upstream_res1() {
    let db = fixture().await;
    for (q, expect_hit) in [
        ("H\u{e0} N\u{1ed9}i", false), // Hà Nội — precomposed ộ not folded
        ("H\u{e0} Noi", true),         // Hà Noi
        ("Ha Noi", true),
        ("Ha N\u{1ed9}i", false), // Ha Nội — precomposed ộ not folded
        ("Ha No\u{302}i", true), // decomposed circumflex: mark skipped
        ("Ha No\u{323}\u{302}i", true), // dot-below + circumflex skipped
    ] {
        let hit = rows(&db, "", q).await;
        let got = keyset(&hit);
        assert_eq!(got.contains(&"hanoi"), expect_hit, "query {q:?}");
    }
}

/// fts5umlaut.test §1.$tn.2 (reverse direction): the variant is the DOC and
/// the query is 'Ha Noi'. Same remove_diacritics=1 asymmetry: precomposed-ộ
/// docs are invisible to the plain query, decomposed-mark docs are visible.
#[tokio::test]
async fn hanoi_reverse_direction_doc_variants() {
    let db = mem_db().await;
    for (k, t) in [
        ("v1", "H\u{e0} N\u{1ed9}i"),       // invisible
        ("v2", "H\u{e0} Noi"),               // visible
        ("v3", "Ha Noi"),                    // visible
        ("v4", "Ha N\u{1ed9}i"),             // invisible
        ("v5", "Ha No\u{302}i"),             // visible
        ("v6", "Ha No\u{323}\u{302}i"),      // visible
    ] {
        db.upsert("", k, t, &dummy_embed(t, DIM)).await.unwrap();
    }
    assert_eq!(
        keyset(&rows(&db, "", "Ha Noi").await),
        ["v2", "v3", "v5", "v6"]
    );
}

/// fts5unicode.test §1.$tn.2 ('..May...you.shAre.freely' splits on
/// punctuation): dashes, dots and quotes are separators, so every embedded
/// word is queryable. Measured: the *query* "baz.qux" is rejected — '.' is
/// not an expression bareword — as InvalidFts.
#[tokio::test]
async fn punctuation_and_quotes_split_tokens() {
    let db = fixture().await;
    for q in ["foo", "bar", "baz", "qux", "quoted", "words"] {
        assert!(keyset(&rows(&db, "", q).await).contains(&"punct"), "query {q:?}");
    }
    assert!(rows(&db, "", "foobar").await.is_empty());
    invalid(&db, "baz.qux").await;
}

/// Measured expression-grammar quirk (fts5parse.y): an UNQUOTED hyphenated
/// query is not a phrase — `foo-bar` parses as `foo` implicitly ANDed with
/// the negated colset `-bar`, failing at parse time with "no such column:
/// bar". That message is not in anyhome's InvalidFts whitelist, so it
/// surfaces as a hard Err. The quoted form is the working spelling.
#[tokio::test]
async fn unquoted_hyphen_query_is_a_negated_colset_error() {
    let db = fixture().await;
    let err = db.where_search("", "foo-bar", 50).await.unwrap_err();
    assert!(format!("{err:#}").contains("no such column: bar"), "{err:#}");
    // The quoted form is a foo+bar phrase: both hyphen and underscore docs
    // tokenize to adjacent foo,bar, so both match.
    assert_eq!(keyset(&rows(&db, "", "\"foo-bar\"").await), ["punct", "under"]);
    assert_eq!(keyset(&rows(&db, "", "foo bar").await), ["punct", "under"]);
}

/// Measured unicode61 behavior: underscore is a separator (not a token
/// char), so "foo_bar" indexes as foo+bar and a "foo_bar" query —
/// itself an expression bareword — tokenizes to the same adjacent pair.
#[tokio::test]
async fn underscore_is_a_unicode61_separator() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "foo_bar").await), ["punct", "under"]);
    assert!(rows(&db, "", "foobar").await.is_empty());
}

/// Measured unicode61 behavior (fts5unicode tokenizer runs): letters and
/// digits fuse into single tokens — "x86" and "abc123" are ONE token each;
/// their letter/digit halves are not indexed separately.
#[tokio::test]
async fn letters_and_digits_form_single_tokens() {
    let db = fixture().await;
    for q in ["x86", "X86", "abc123", "ABC123"] {
        assert_eq!(keyset(&rows(&db, "", q).await), ["digit"], "query {q:?}");
    }
    assert!(rows(&db, "", "abc").await.is_empty());
    assert!(rows(&db, "", "123").await.is_empty());
}

/// fts5unicode.test §1.x case-folding expectation extended to a non-Latin
/// script: Cyrillic folds case in both directions (measured deterministic).
#[tokio::test]
async fn cyrillic_case_folding() {
    let db = fixture().await;
    for q in ["\u{43f}\u{440}\u{438}\u{432}\u{435}\u{442}", "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}", "\u{41f}\u{420}\u{418}\u{412}\u{415}\u{422}"] {
        assert_eq!(keyset(&rows(&db, "", q).await), ["cyr"], "query {q:?}");
    }
    assert_eq!(
        keyset(&rows(&db, "", "\u{431}\u{43e}\u{43b}\u{44c}\u{448}\u{43e}\u{439}").await),
        ["cyr-upper"]
    ); // большой -> БОЛЬШОЙ
    assert_eq!(
        keyset(&rows(&db, "", "\u{43c}\u{438}\u{440}").await),
        ["cyr", "cyr-upper"]
    ); // мир in both docs
}
