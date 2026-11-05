use super::*;
use crate::bm25::Bm25Index;
use crate::index::MAX_QUERY_CHARS;
use crate::tokenize::NgramTokenizer;
use crate::vector::{Embedder, HashingEmbedder, VectorIndex};
use std::sync::Mutex;

const DOCS: [(&str, &str); 4] = [
    ("a", "契約の更新日は来年三月です。"),
    ("b", "型番 AB-1234 の取扱説明書"),
    ("c", "第12条 秘密保持義務について"),
    ("d", "今日の天気は晴れでした"),
];

fn searcher_with(config: HybridConfig) -> HybridSearcher {
    HybridSearcher::new(
        Arc::new(Bm25Index::build(Box::new(NgramTokenizer), DOCS)),
        Arc::new(VectorIndex::build(Box::new(HashingEmbedder::new(512)), DOCS).unwrap()),
        config,
    )
}

pub(super) fn searcher() -> HybridSearcher {
    searcher_with(HybridConfig::default())
}

fn run(q: &str) -> SearchOutcome {
    searcher().search(q, 3).unwrap()
}

fn top(outcome: SearchOutcome) -> String {
    match outcome {
        SearchOutcome::Found { hits, .. } => hits[0].id.clone(),
        SearchOutcome::NotFound { grounding } => panic!("not found (grounding={grounding})"),
    }
}

pub(super) fn is_not_found(o: &SearchOutcome) -> bool {
    matches!(o, SearchOutcome::NotFound { .. })
}

#[test]
fn natural_question_finds_the_contract_doc() {
    assert_eq!(top(run("契約の更新日はいつですか")), "a");
}

#[test]
fn exact_identifiers_are_found() {
    assert_eq!(top(run("AB-1234")), "b");
    assert_eq!(top(run("第12条")), "c");
}

#[test]
fn kanji_article_numbers_match_arabic_ones() {
    assert_eq!(top(run("第十二条")), "c");
}

#[test]
fn unrelated_query_is_not_found() {
    assert!(is_not_found(&run("量子コンピュータの冷却方式")));
}

#[test]
fn nonexistent_article_number_is_not_found_in_either_notation() {
    assert!(is_not_found(&run("第99条")));
    assert!(is_not_found(&run("第十三条")));
}

#[test]
fn gibberish_is_not_found() {
    assert!(is_not_found(&run("zzzzqqqq")));
}

#[test]
fn high_threshold_rejects_weak_matches() {
    let s = searcher().with_threshold(1.1);
    assert!(is_not_found(&s.search("AB-1234", 3).unwrap()));
}

#[test]
fn with_threshold_does_not_change_the_original() {
    let s = searcher();
    let _strict = s.with_threshold(1.1);
    assert_eq!(top(s.search("AB-1234", 3).unwrap()), "b");
}

#[test]
fn nan_threshold_is_rejected_instead_of_accepting_everything() {
    let s = searcher().with_threshold(f32::NAN);
    assert!(matches!(s.search("zzzz", 3), Err(SearchError::Config(_))));
    assert!(matches!(
        s.search("量子コンピュータの冷却", 3),
        Err(SearchError::Config(_))
    ));
}

#[test]
fn invalid_rrf_k_is_rejected() {
    for bad in [f32::NAN, f32::INFINITY, -1.0] {
        let s = searcher_with(HybridConfig {
            rrf_k: bad,
            ..HybridConfig::default()
        });
        assert!(
            matches!(s.search("契約", 3), Err(SearchError::Config(_))),
            "{bad}"
        );
    }
}

#[test]
fn k_zero_is_an_error_not_an_empty_found() {
    assert!(matches!(
        searcher().search("契約の更新日", 0),
        Err(SearchError::Config(_))
    ));
}

#[test]
fn result_count_is_limited_by_k() {
    match searcher().search("契約の更新日", 1).unwrap() {
        SearchOutcome::Found { hits, .. } => assert_eq!(hits.len(), 1),
        other => panic!("{other:?}"),
    }
}

#[test]
fn pool_smaller_than_k_still_returns_k_results() {
    let docs = [
        ("x", "契約書の原本"),
        ("y", "契約の更新"),
        ("z", "契約の終了"),
        ("w", "天気予報"),
    ];
    let s = HybridSearcher::new(
        Arc::new(Bm25Index::build(Box::new(NgramTokenizer), docs)),
        Arc::new(VectorIndex::build(Box::new(HashingEmbedder::new(512)), docs).unwrap()),
        HybridConfig {
            pool: 0,
            grounding_threshold: 0.0,
            ..HybridConfig::default()
        },
    );
    match s.search("契約", 3).unwrap() {
        SearchOutcome::Found { hits, .. } => assert!(hits.len() >= 3, "{hits:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn huge_k_is_clamped_and_still_returns_the_right_documents() {
    match searcher().search("契約の更新日", usize::MAX).unwrap() {
        SearchOutcome::Found { hits, .. } => {
            assert_eq!(hits[0].id, "a");
            assert!(hits.len() <= DOCS.len(), "{hits:?}");
        }
        other => panic!("{other:?}"),
    }
}

struct VanishingDocs;
impl LexicalIndex for VanishingDocs {
    fn search(&self, _: &str, _: usize) -> Result<Vec<Hit>, SearchError> {
        Ok(vec![Hit {
            id: "gone".into(),
            score: 1.0,
        }])
    }
    fn doc_signals(&self, _: &str, _: &str) -> Result<Option<DocSignals>, SearchError> {
        Ok(None) // 検索後に文書が削除された状況
    }
}

#[test]
fn document_deleted_between_search_and_grounding_is_not_found() {
    let s = HybridSearcher::new(
        Arc::new(VanishingDocs),
        Arc::new(VectorIndex::build(Box::new(HashingEmbedder::new(8)), DOCS).unwrap()),
        HybridConfig::default(),
    );
    assert!(is_not_found(&s.search("契約", 3).unwrap()));
}

/// 呼ばれた `embed` の入力長（文字数）を記録する。
struct SpyEmbedder {
    inner: HashingEmbedder,
    lens: Arc<Mutex<Vec<usize>>>,
}
impl Embedder for SpyEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.lens.lock().unwrap().push(text.chars().count());
        self.inner.embed(text)
    }
    fn dim(&self) -> usize {
        self.inner.dim()
    }
    fn id(&self) -> String {
        "spy".into()
    }
}

#[test]
fn huge_query_is_truncated_before_reaching_the_embedder() {
    let lens = Arc::new(Mutex::new(Vec::new()));
    let embedder = SpyEmbedder {
        inner: HashingEmbedder::new(64),
        lens: Arc::clone(&lens),
    };
    let vector = VectorIndex::build(Box::new(embedder), DOCS).unwrap();
    lens.lock().unwrap().clear(); // 索引構築時の呼び出しは除く
    let s = HybridSearcher::new(
        Arc::new(Bm25Index::build(Box::new(NgramTokenizer), DOCS)),
        Arc::new(vector),
        HybridConfig::default(),
    );
    let huge = "契約 ".repeat(MAX_QUERY_CHARS);
    s.search(&huge, 3).unwrap();
    let seen = lens.lock().unwrap().clone();
    assert!(!seen.is_empty(), "embedder が呼ばれていない");
    assert!(
        seen.iter().all(|&n| n <= MAX_QUERY_CHARS),
        "切り詰められずに渡された: {seen:?}"
    );
}

// ---- 語彙・意味の最上位が異なる／同じ場合のシグナル共有 ----------------------------

/// 最上位を固定で返し、`doc_signals` の呼び出しと検索クエリの長さを記録するスタブ。
pub(super) struct StubLex {
    top: &'static str,
    coverage: f32,
    missing: &'static [&'static str],
    signal_calls: Mutex<Vec<String>>,
    query_chars: Mutex<Vec<usize>>,
}

impl StubLex {
    pub(super) fn new(
        top: &'static str,
        coverage: f32,
        missing: &'static [&'static str],
    ) -> Arc<Self> {
        Arc::new(Self {
            top,
            coverage,
            missing,
            signal_calls: Mutex::new(Vec::new()),
            query_chars: Mutex::new(Vec::new()),
        })
    }
}

impl LexicalIndex for StubLex {
    fn search(&self, query: &str, _: usize) -> Result<Vec<Hit>, SearchError> {
        self.query_chars.lock().unwrap().push(query.chars().count());
        Ok(vec![Hit {
            id: self.top.into(),
            score: 1.0,
        }])
    }
    fn doc_signals(&self, id: &str, _: &str) -> Result<Option<DocSignals>, SearchError> {
        self.signal_calls.lock().unwrap().push(id.to_string());
        Ok(Some(DocSignals {
            coverage: self.coverage,
            missing_identifier: self.missing.contains(&id),
        }))
    }
}

pub(super) struct StubSem(pub(super) &'static str);
impl SemanticIndex for StubSem {
    fn search(&self, _: &str, _: usize) -> Result<Vec<Hit>, SearchError> {
        Ok(vec![Hit {
            id: self.0.into(),
            score: 0.9,
        }])
    }
}

pub(super) fn stub_searcher(lex: Arc<StubLex>, sem_top: &'static str) -> HybridSearcher {
    HybridSearcher::new(lex, Arc::new(StubSem(sem_top)), HybridConfig::default())
}

#[test]
fn semantic_top_uses_its_own_signals_when_it_differs_from_the_lexical_top() {
    // 語彙側の最上位 x は識別子が欠けているが、意味側の最上位 y は問題ない → y の根拠 0.9 が通る。
    let lex = StubLex::new("x", 0.05, &["x"]);
    let s = stub_searcher(lex.clone(), "y");
    match s.search("q", 3).unwrap() {
        SearchOutcome::Found { grounding, .. } => assert!((grounding - 0.9).abs() < 1e-6),
        other => panic!("{other:?}"),
    }
    assert_eq!(*lex.signal_calls.lock().unwrap(), vec!["x", "y"]);
}

#[test]
fn a_missing_identifier_on_the_semantic_top_is_not_hidden_by_the_lexical_top() {
    // 語彙側の最上位 x は問題ないが、意味側の最上位 y は識別子が欠けている → 0.9 は 0.1 に抑える。
    let lex = StubLex::new("x", 0.05, &["y"]);
    let s = stub_searcher(lex, "y");
    assert!(is_not_found(&s.search("q", 3).unwrap()));
}

#[test]
fn signals_are_fetched_once_when_both_tops_are_the_same_document() {
    let lex = StubLex::new("x", 0.8, &[]);
    let s = stub_searcher(lex.clone(), "x");
    assert!(matches!(
        s.search("q", 3).unwrap(),
        SearchOutcome::Found { .. }
    ));
    assert_eq!(*lex.signal_calls.lock().unwrap(), vec!["x"]);
}

#[test]
fn huge_query_is_truncated_at_the_hybrid_entry_point() {
    let lex = StubLex::new("x", 0.8, &[]);
    let s = stub_searcher(lex.clone(), "x");
    s.search(&"あ".repeat(MAX_QUERY_CHARS * 3), 3).unwrap();
    let seen = lex.query_chars.lock().unwrap().clone();
    assert_eq!(seen, vec![MAX_QUERY_CHARS]);
}
