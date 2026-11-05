//! 検索・更新の整合性・入力上限のテスト。永続化・索引の整合・同時 open は `tests_persist`。

use std::sync::Arc;
use std::time::Instant;

use super::*;
use crate::eval::CORPUS;
use crate::grounding::{HybridConfig, HybridSearcher, SearchOutcome};
use crate::index::MAX_DOC_BYTES;
use crate::tokenize::NgramTokenizer;
use crate::vector::HashingEmbedder;

pub(super) fn ngram() -> Box<dyn Tokenizer> {
    Box::new(NgramTokenizer)
}

pub(super) fn hashing(dim: usize) -> Box<dyn Embedder> {
    Box::new(HashingEmbedder::new(dim))
}

pub(super) fn store() -> SqliteStore {
    SqliteStore::in_memory(ngram(), hashing(512)).unwrap()
}

pub(super) fn loaded() -> SqliteStore {
    let s = store();
    for (id, text) in CORPUS {
        s.upsert(id, text).unwrap();
    }
    s
}

pub(super) fn lex(s: &SqliteStore, q: &str, k: usize) -> Vec<Hit> {
    LexicalIndex::search(s, q, k).unwrap()
}

pub(super) fn sem(s: &SqliteStore, q: &str, k: usize) -> Vec<Hit> {
    SemanticIndex::search(s, q, k).unwrap()
}

pub(super) fn signals_of(s: &SqliteStore, id: &str, q: &str) -> Option<DocSignals> {
    LexicalIndex::doc_signals(s, id, q).unwrap()
}

// ---- 基本検索 ------------------------------------------------------------------------

#[test]
fn lexical_finds_identifiers_and_phrases() {
    let s = loaded();
    assert_eq!(lex(&s, "ＡＢ－１２３４", 3)[0].id, "manual-ab1234");
    assert_eq!(lex(&s, "第12条", 3)[0].id, "contract-secrecy");
    assert_eq!(lex(&s, "第十二条", 3)[0].id, "contract-secrecy");
    assert_eq!(lex(&s, "契約の更新日", 3)[0].id, "contract-renewal");
}

#[test]
fn hyphen_variants_find_the_same_model_number() {
    let s = loaded();
    for q in ["AB\u{2010}1234", "AB\u{2212}1234", "ab-1234"] {
        assert_eq!(lex(&s, q, 3)[0].id, "manual-ab1234", "{q:?}");
    }
}

#[test]
fn semantic_returns_identical_text_first_with_similarity_near_one() {
    let s = loaded();
    let (id, text) = CORPUS[0];
    let hits = sem(&s, text, 3);
    assert_eq!(hits[0].id, id);
    assert!((hits[0].score - 1.0).abs() < 1e-3, "{:?}", hits[0]);
}

#[test]
fn doc_signals_report_coverage_and_missing_identifier() {
    let s = loaded();
    let ok = signals_of(&s, "manual-ab1234", "ab-1234").unwrap();
    assert!((ok.coverage - 1.0).abs() < 1e-6);
    assert!(!ok.missing_identifier);

    let half = signals_of(&s, "manual-ab1234", "ab-1234 zzzz").unwrap();
    assert!((half.coverage - 0.5).abs() < 1e-6);

    assert!(
        signals_of(&s, "contract-secrecy", "第99条")
            .unwrap()
            .missing_identifier
    );
    assert!(
        signals_of(&s, "contract-secrecy", "第十三条")
            .unwrap()
            .missing_identifier
    );
    assert!(
        !signals_of(&s, "contract-secrecy", "第十二条")
            .unwrap()
            .missing_identifier
    );
    assert!(signals_of(&s, "nope", "第12条").is_none());
}

/// インメモリ版と SQLite 版で、同じコーパス・クエリの signals が一致すること。
/// 分音記号付きラテン文字（café）も同じ意味で扱う。
#[test]
fn doc_signals_match_the_in_memory_implementation() {
    let mut docs: Vec<(&str, &str)> = CORPUS.to_vec();
    docs.push(("latin", "cafe latte と TEST の résumé"));
    let s = store();
    for (id, text) in &docs {
        s.upsert(id, text).unwrap();
    }
    let mem = crate::Bm25Index::build(ngram(), docs.iter().copied());
    let queries = [
        "契約の更新日はいつですか",
        "第99条",
        "AB-1234",
        "ホテルの時間",
        "zzzz",
        "café",
        "tést",
        "CAFÉ",
        "resume",
        "résumé",
    ];
    for (id, _) in &docs {
        for q in queries {
            let a = signals_of(&s, id, q).unwrap();
            let b = mem.doc_signals(id, q).unwrap();
            assert!(
                (a.coverage - b.coverage).abs() < 1e-6,
                "{id} {q}: {a:?} {b:?}"
            );
            assert_eq!(a.missing_identifier, b.missing_identifier, "{id} {q}");
        }
    }
    // café は cafe に当たらない（分音記号を区別する）。両実装とも。
    assert_eq!(signals_of(&s, "latin", "café").unwrap().coverage, 0.0);
    assert_eq!(mem.doc_signals("latin", "café").unwrap().coverage, 0.0);
    assert!(lex(&s, "café", 3).is_empty());
}

// ---- 更新・削除の整合性 --------------------------------------------------------------

#[test]
fn upsert_replaces_content_and_remove_deletes() {
    let s = store();
    s.upsert("x", "りんごとみかん").unwrap();
    s.upsert("x", "ぶどうとすいか").unwrap();
    assert_eq!(s.len().unwrap(), 1);
    assert!(lex(&s, "りんご", 3).is_empty());
    assert_eq!(lex(&s, "ぶどう", 3)[0].id, "x");
    assert_eq!(sem(&s, "ぶどうとすいか", 1)[0].id, "x");

    assert!(s.remove("x").unwrap());
    assert!(!s.remove("x").unwrap());
    assert!(s.is_empty().unwrap());
    assert!(lex(&s, "ぶどう", 3).is_empty());
    assert!(sem(&s, "ぶどうとすいか", 1).is_empty());
    assert!(signals_of(&s, "x", "ぶどう").is_none());
}

#[test]
fn repeated_upserts_and_interleaved_ids_stay_consistent() {
    let s = store();
    for round in 0..5 {
        for (i, (id, text)) in CORPUS.iter().take(6).enumerate() {
            s.upsert(id, &format!("{text} marker{round}x{i}")).unwrap();
        }
    }
    assert_eq!(s.len().unwrap(), 6);
    // 古い版の語は残らず、最新の版の語だけが引ける。
    assert!(lex(&s, "marker0x0", 10).is_empty());
    assert!(lex(&s, "marker3x2", 10).is_empty());
    assert_eq!(lex(&s, "marker4x2", 10).len(), 1);
    assert_eq!(lex(&s, "marker4x2", 10)[0].id, CORPUS[2].0);
    assert_eq!(lex(&s, "AB-1234", 3)[0].id, "manual-ab1234");
    assert_eq!(sem(&s, CORPUS[0].1, 1)[0].id, CORPUS[0].0);
    // 1 件消しても他の索引行に影響しない。
    assert!(s.remove(CORPUS[0].0).unwrap());
    assert_eq!(s.len().unwrap(), 5);
    assert_eq!(lex(&s, "AB-1234", 3)[0].id, "manual-ab1234");
}

// ---- ゼロベクトル文書 -----------------------------------------------------------------

#[test]
fn documents_without_tokens_do_not_break_semantic_search() {
    let s = store();
    s.upsert("empty", "").unwrap();
    s.upsert("symbols", "。。。").unwrap();
    s.upsert("real", "契約の更新日は来年三月です").unwrap();
    for k in [1, 3, 5, 100] {
        let hits = sem(&s, "契約の更新日", k);
        assert_eq!(hits.len(), 1, "k={k}");
        assert_eq!(hits[0].id, "real");
    }
    assert_eq!(s.len().unwrap(), 3);
}

#[test]
fn hybrid_search_survives_empty_documents() {
    let s = Arc::new(store());
    s.upsert("empty", "").unwrap();
    s.upsert("real", "契約の更新日は来年三月です").unwrap();
    let h = HybridSearcher::new(s.clone(), s, HybridConfig::default());
    match h.search("契約の更新日", 5).unwrap() {
        SearchOutcome::Found { hits, .. } => assert_eq!(hits[0].id, "real"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn updating_a_document_to_empty_removes_it_from_vector_search() {
    let s = store();
    s.upsert("x", "ぶどうとすいか").unwrap();
    s.upsert("x", "").unwrap();
    assert!(sem(&s, "ぶどうとすいか", 3).is_empty());
    assert!(lex(&s, "ぶどう", 3).is_empty());
}

// ---- 入力の上限・敵対的な入力 --------------------------------------------------------

#[test]
fn match_expr_quotes_every_term_and_escapes_quotes() {
    let terms = vec![
        "a\"b".to_string(),
        "NEAR(x y)".to_string(),
        "AND".to_string(),
    ];
    let expr = match_expr(&terms).unwrap();
    assert_eq!(expr, "\"a\"\"b\" OR \"NEAR(x y)\" OR \"AND\"");
    assert!(match_expr(&["。".to_string(), String::new()]).is_none());
    assert!(match_expr(&[]).is_none());
    assert_eq!(quote_term("a\"b"), "\"a\"\"b\"");
}

/// 空白で語を切るだけのトークナイザ。`"` や演算子がそのままトークンになる。
struct RawTokenizer;
impl Tokenizer for RawTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }
    fn id(&self) -> String {
        "raw-test".into()
    }
}

#[test]
fn quote_and_operator_tokens_reach_fts5_safely() {
    let s = SqliteStore::in_memory(Box::new(RawTokenizer), hashing(64)).unwrap();
    s.upsert("d1", "say \"hello\" NEAR(a b) AND OR NOT col:val * ^x")
        .unwrap();
    for q in [
        "\"hello\"",
        "NEAR(a",
        "AND",
        "OR",
        "NOT",
        "col:val",
        "*",
        "^x",
        "\"",
        "\"\"\"",
        "-",
    ] {
        assert!(LexicalIndex::search(&s, q, 3).is_ok(), "search {q:?}");
        assert!(
            LexicalIndex::doc_signals(&s, "d1", q).is_ok(),
            "signals {q:?}"
        );
    }
    assert_eq!(lex(&s, "NEAR(a", 3)[0].id, "d1");
}

#[test]
fn hostile_queries_do_not_error() {
    let s = loaded();
    for q in [
        "\"",
        "AND OR NOT",
        "契約\" OR \"x",
        "*",
        "a* NEAR(b c)",
        "col:val",
        "-",
        "",
    ] {
        assert!(LexicalIndex::search(&s, q, 3).is_ok(), "{q:?}");
        assert!(SemanticIndex::search(&s, q, 3).is_ok(), "{q:?}");
        assert!(
            LexicalIndex::doc_signals(&s, "manual-ab1234", q).is_ok(),
            "{q:?}"
        );
    }
}

#[test]
fn huge_query_is_bounded_in_time_for_the_lexical_side() {
    let s = loaded();
    let query: String = (0..200_000).map(|i| format!("w{i} ")).collect();
    let start = Instant::now();
    assert!(lex(&s, &query, 3).is_empty());
    assert!(signals_of(&s, "manual-ab1234", &query).is_some());
    assert!(
        start.elapsed().as_secs() < 10,
        "20万語のクエリが {:?} かかった",
        start.elapsed()
    );
}

#[test]
fn extreme_k_values_are_clamped_and_results_match_a_normal_k() {
    let s = loaded();
    let normal = lex(&s, "契約", MAX_K);
    assert!(normal.len() >= 2, "契約を含む文書が複数あるはず");
    for k in [
        4097,
        100_000,
        i64::MAX as usize,
        i64::MAX as usize + 1,
        usize::MAX,
    ] {
        assert_eq!(lex(&s, "契約", k), normal, "lexical k={k}");
        let sem_normal = sem(&s, "契約の更新日", MAX_K);
        assert!(!sem_normal.is_empty());
        assert_eq!(sem(&s, "契約の更新日", k), sem_normal, "semantic k={k}");
    }
}

#[test]
fn oversized_documents_are_rejected_without_touching_the_index() {
    let s = store();
    s.upsert("keep", "残るはずの文書").unwrap();
    let big = "あ".repeat(MAX_DOC_BYTES / 3 + 1);
    assert!(big.len() > MAX_DOC_BYTES);
    assert!(matches!(s.upsert("big", &big), Err(SearchError::Config(_))));
    assert!(matches!(
        s.upsert("keep", &big),
        Err(SearchError::Config(_))
    ));
    assert_eq!(s.len().unwrap(), 1);
    assert_eq!(lex(&s, "残る", 3)[0].id, "keep");
}

#[test]
fn k_zero_and_empty_store_are_safe() {
    let s = store();
    assert!(lex(&s, "契約", 3).is_empty());
    assert!(sem(&s, "契約", 3).is_empty());
    let s = loaded();
    assert!(lex(&s, "契約", 0).is_empty());
    assert!(sem(&s, "契約", 0).is_empty());
}

#[test]
fn empty_id_is_rejected() {
    assert!(matches!(
        store().upsert("", "x"),
        Err(SearchError::Config(_))
    ));
}
