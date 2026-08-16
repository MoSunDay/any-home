//! FTS5 MATCH *query syntax* ported from the libsql TCL suites onto the
//! public `MemoryDb` API (TODO-63 / init-pro-11): fts5aa.test (§2.3 term
//! queries, §12.2-12.3 `*` special queries, §13 single-char tokens),
//! fts5prefix.test (§1.1 prefix queries, §3.1/§6 column-filtered prefixes)
//! and fts5ac.test's §3-§9 matrix (column filters, `+` phrases, NEAR,
//! operator precedence) as cross-check source. Every assertion is the
//! *measured* behavior of the bundled libsql FTS5 through `where_search`.
//!
//! Excluded from these suites (no surface against the fixed anyhome schema,
//! single-column `text` fts5 table, default tokenizer, default detail):
//! - fts5aa §1-§11, §14-§25: shadow-table DDL, integrity-check/pgsz/merge
//!   internals, reserved-name DDL, nested transactions, shadow corruption,
//!   bm25() internals, rowid extremes, NULL docs, temp-schema DDL.
//! - fts5prefix §2.x/§4-§7: `prefix=` index options, on-disk checksum
//!   equivalence, highlight()/snippet() output — not configurable here.
//! - detail=none/column tables, custom tokenizers, rank weight values beyond
//!   relevance order (ordering is pinned in libsql_fts_semantics.rs).

use anyhome::db::{dummy_embed, MemoryDb, WhereOutcome};

const DIM: usize = 8;

/// Unwrap a where_search into rows (panics on InvalidFts/Err).
async fn rows(db: &MemoryDb, agent: &str, cond: &str) -> Vec<(String, String)> {
    match db.where_search(agent, cond, 50).await.unwrap() {
        WhereOutcome::Rows(r) => r,
        WhereOutcome::InvalidFts(bad) => panic!("unexpected InvalidFts for {bad:?}"),
    }
}

/// Sorted keys: these tests pin set membership, not rank order (rank order is
/// pinned separately in libsql_fts_semantics.rs).
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

/// Minimal deterministic fixture shared by the syntax scenarios.
async fn fixture() -> MemoryDb {
    let db = mem_db().await;
    for (k, t) in [
        ("d1", "one two three"),
        ("d2", "four five six"),
        ("d3", "seven eight nine ten"),
        ("d4", "rust rules the world"),
        ("d5", "ruby runs fast"),
        ("d6", "the quick brown fox jumps"),
    ] {
        db.upsert("", k, t, &dummy_embed(t, DIM)).await.unwrap();
    }
    db
}

/// fts5aa.test §2.3: every single indexed term of a doc matches that doc.
#[tokio::test]
async fn single_term_matches_each_doc_containing_it() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "two").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "five").await), ["d2"]);
    assert_eq!(keyset(&rows(&db, "", "ten").await), ["d3"]);
    assert_eq!(keyset(&rows(&db, "", "fox").await), ["d6"]);
    assert!(rows(&db, "", "zzznomatch").await.is_empty());
}

/// fts5aa.test §2.3 / fts5ac.test §9.12: whitespace between terms is an
/// implicit AND; terms scattered across docs yield nothing.
#[tokio::test]
async fn implicit_and_between_adjacent_terms() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "two three").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "one two").await), ["d1"]);
    assert!(rows(&db, "", "two six").await.is_empty());
}

/// fts5ac.test §9.12-9.13: explicit AND requires every term in one doc.
#[tokio::test]
async fn explicit_and_requires_all_terms() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "four AND six").await), ["d2"]);
    assert!(rows(&db, "", "one AND six").await.is_empty());
}

/// fts5ac.test §9.14b: OR returns the union of its arms' doc sets.
#[tokio::test]
async fn or_returns_union_of_arms() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "one OR five").await), ["d1", "d2"]);
    assert_eq!(keyset(&rows(&db, "", "one OR five OR ten").await), ["d1", "d2", "d3"]);
}

/// fts5ac.test §9.18: NOT is a binary exclusion (`c NOT (b OR a)` shape).
/// Measured: a dangling NOT (either side) is a malformed expression.
#[tokio::test]
async fn not_is_binary_exclusion() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "the NOT rust").await), ["d6"]); // d4 excluded
    assert_eq!(keyset(&rows(&db, "", "one NOT nine").await), ["d1"]);
    assert!(rows(&db, "", "two NOT three").await.is_empty()); // d1 has both
    invalid(&db, "NOT one").await;
    invalid(&db, "one NOT").await;
}

/// fts5ac.test §9.15-9.17: AND binds tighter than OR; parentheses override.
#[tokio::test]
async fn parentheses_override_and_or_precedence() {
    let db = fixture().await;
    // one OR (five AND six) — AND first.
    assert_eq!(keyset(&rows(&db, "", "one OR five AND six").await), ["d1", "d2"]);
    // (one OR five) AND six — grouping wins.
    assert_eq!(keyset(&rows(&db, "", "(one OR five) AND six").await), ["d2"]);
    assert_eq!(keyset(&rows(&db, "", "(one OR five) AND two").await), ["d1"]);
}

/// fts5ac.test §9.10-9.11 (`x : "r c"`): a quoted phrase matches only when
/// the terms are adjacent and in order.
#[tokio::test]
async fn phrase_matches_adjacent_terms_in_order() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "\"quick brown\"").await), ["d6"]);
    assert!(rows(&db, "", "\"brown quick\"").await.is_empty());
    // Adjacency is required: one@0, three@2 in d1 is not a phrase.
    assert!(rows(&db, "", "\"one three\"").await.is_empty());
}

/// fts5aa.test §12 style no-hit queries stay `Rows(vec![])`, never an error:
/// a phrase matching nothing, and a single-term quoted phrase (= the term).
#[tokio::test]
async fn phrase_without_match_or_adjacency_is_empty() {
    let db = fixture().await;
    assert!(rows(&db, "", "\"zebra crossing\"").await.is_empty());
    assert_eq!(keyset(&rows(&db, "", "\"one\"").await), ["d1"]);
}

/// fts5ac.test §8.1/§8.6: NEAR() with the default distance of 10 matches;
/// terms living in different docs never share a NEAR context.
#[tokio::test]
async fn near_default_distance_is_generous() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "NEAR(one three)").await), ["d1"]);
    assert!(rows(&db, "", "NEAR(one five)").await.is_empty());
}

/// fts5ac.test §8.2-8.5 (`NEAR(r c, k)` sweep): the explicit distance is the
/// maximum number of *intervening* tokens. one@0/three@2 in "one two three"
/// have one intervening token: k=1 matches, k=0 does not; adjacent terms
/// match at k=0. Measured on the bundled libsql.
#[tokio::test]
async fn near_explicit_distance_counts_intervening_tokens() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "NEAR(one three, 1)").await), ["d1"]);
    assert!(rows(&db, "", "NEAR(one three, 0)").await.is_empty());
    assert_eq!(keyset(&rows(&db, "", "NEAR(two three, 0)").await), ["d1"]);
}

/// fts5ac.test §8 shape variants, measured: a single-phrase NEAR behaves like
/// the bare term; NEAR is order-insensitive; an empty NEAR() is malformed.
#[tokio::test]
async fn near_argument_shapes() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "NEAR(one)").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "NEAR(three one)").await), ["d1"]);
    invalid(&db, "NEAR()").await;
}

/// Measured grammar detail (fts5parse.y): NEAR is a keyword only in the
/// `NEAR(...)` form. Infix `one NEAR three` parses as an implicit AND of
/// three literal terms — d1 ("one two three") lacks a "near" token, so no
/// hit; had NEAR been an operator here, d1 would match.
#[tokio::test]
async fn infix_near_is_a_literal_term_not_an_operator() {
    let db = fixture().await;
    assert!(rows(&db, "", "one NEAR three").await.is_empty());
}

/// Measured: NEAR is case-sensitive — lowercase `near(...)` is not the
/// operator and is rejected as a malformed expression.
#[tokio::test]
async fn lowercase_near_is_not_the_operator() {
    let db = fixture().await;
    invalid(&db, "near(one three)").await;
}

/// fts5prefix.test §1.1 (`t*` over the same three-doc corpus): a prefix
/// expands to every indexed term starting with it — no prefix index needed.
#[tokio::test]
async fn prefix_query_expands_across_docs() {
    let db = fixture().await;
    // two/three (d1), ten (d3), the (d4), the (d6).
    assert_eq!(keyset(&rows(&db, "", "t*").await), ["d1", "d3", "d4", "d6"]);
    // rust/rules (d4) and ruby/runs (d5).
    assert_eq!(keyset(&rows(&db, "", "ru*").await), ["d4", "d5"]);
}

/// fts5prefix.test §1.1/§3.1 shapes: a prefix hitting nothing stays an empty
/// `Rows`; a prefix equal to a full token matches that token.
#[tokio::test]
async fn prefix_matching_nothing_or_full_token() {
    let db = fixture().await;
    assert!(rows(&db, "", "zz*").await.is_empty());
    assert_eq!(keyset(&rows(&db, "", "eigh*").await), ["d3"]);
    assert_eq!(keyset(&rows(&db, "", "the*").await), ["d4", "d6"]);
}

/// fts5prefix.test §3.1 (`a : c*`) + fts5ac.test §2.x (`x:a`): `text:`
/// column filters work with terms, prefixes, phrases and grouped exprs
/// (whitespace around the colon is optional — measured).
#[tokio::test]
async fn column_filter_colon_forms() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "text: two").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "text:two").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "text : t*").await), ["d1", "d3", "d4", "d6"]);
    assert_eq!(keyset(&rows(&db, "", "text : \"quick brown\"").await), ["d6"]);
    assert_eq!(keyset(&rows(&db, "", "text:(one OR five)").await), ["d1", "d2"]);
}

/// fts5ac.test §3.1 (`{x}:a`): the braces colset form is equivalent for the
/// single `text` column, incl. with NEAR. Measured: `-text`/`-{text}` invert
/// the colset — with one column the complement is empty; `{-text}` (minus
/// inside the braces) is malformed.
#[tokio::test]
async fn column_filter_braces_forms() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "{text} : two").await), ["d1"]);
    assert_eq!(keyset(&rows(&db, "", "{text} : NEAR(one three, 1)").await), ["d1"]);
    assert!(rows(&db, "", "-text : one").await.is_empty());
    assert!(rows(&db, "", "-{text} : one").await.is_empty());
    invalid(&db, "{-text} : one").await;
}

/// fts5ac.test §6.x-§7.x (`a+b`, `a+b AND c`) + fts5parse.y grammar comment:
/// `+` glues terms into ONE ordered phrase, not an implicit AND.
#[tokio::test]
async fn plus_builds_one_ordered_phrase() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "one+two").await), ["d1"]);
    assert!(rows(&db, "", "two+one").await.is_empty()); // order matters
    assert_eq!(keyset(&rows(&db, "", "quick+brown AND fox").await), ["d6"]);
    assert!(rows(&db, "", "brown+quick").await.is_empty());
}

/// fts5aa.test-style mixed expressions: phrase + AND, phrase + OR, and an OR
/// arm that is a prefix query.
#[tokio::test]
async fn phrase_and_operator_mixes() {
    let db = fixture().await;
    assert_eq!(keyset(&rows(&db, "", "\"quick brown\" AND fox").await), ["d6"]);
    assert!(rows(&db, "", "\"quick brown\" AND ten").await.is_empty());
    assert_eq!(keyset(&rows(&db, "", "\"two three\" OR five").await), ["d1", "d2"]);
    assert_eq!(keyset(&rows(&db, "", "one OR ru*").await), ["d1", "d4", "d5"]);
}

/// fts5aa.test §12.2-12.3: a leading `*` enters the special-query path.
/// `*stuff` and bare `*` are "unknown special query" -> InvalidFts.
/// Measured deviation for `* reads `: upstream returns the index read count
/// as a single introspection row, but anyhome JOINs memories_fts against
/// `memories`, which no introspection rowid satisfies -> valid, zero rows.
#[tokio::test]
async fn star_special_queries() {
    let db = fixture().await;
    invalid(&db, "*stuff").await;
    invalid(&db, "*").await;
    assert!(rows(&db, "", "* reads ").await.is_empty());
}

/// fts5aa.test §13.1-13.2: single-character tokens are indexed like any
/// other term (`MATCH 'o'` hits both 'o n e' and 't w o').
#[tokio::test]
async fn single_char_tokens_match_across_docs() {
    let db = mem_db().await;
    db.upsert("", "k1", "o n e", &dummy_embed("x", DIM)).await.unwrap();
    db.upsert("", "k2", "t w o", &dummy_embed("x", DIM)).await.unwrap();
    assert_eq!(keyset(&rows(&db, "", "o").await), ["k1", "k2"]);
    assert_eq!(keyset(&rows(&db, "", "n").await), ["k1"]);
    assert_eq!(keyset(&rows(&db, "", "w").await), ["k2"]);
    assert_eq!(keyset(&rows(&db, "", "\"o n\"").await), ["k1"]);
}
