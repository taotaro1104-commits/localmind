//! 埋め込みの呼び出し（クエリの切り詰め、文書用・クエリ用の振り分け）のテスト。

use std::sync::{Arc, Mutex};

use super::tests::ngram;
use super::*;
use crate::index::MAX_QUERY_CHARS;
use crate::vector::HashingEmbedder;

/// `embed` に渡った文字数を記録する。
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
fn huge_query_is_truncated_before_the_embedder_on_the_semantic_side() {
    let lens = Arc::new(Mutex::new(Vec::new()));
    let embedder = SpyEmbedder {
        inner: HashingEmbedder::new(64),
        lens: Arc::clone(&lens),
    };
    let s = SqliteStore::in_memory(ngram(), Box::new(embedder)).unwrap();
    s.upsert("a", "契約の更新日").unwrap();
    lens.lock().unwrap().clear();
    let huge = "契約 ".repeat(MAX_QUERY_CHARS);
    assert!(SemanticIndex::search(&s, &huge, 3).is_ok());
    let seen = lens.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "embed が 1 回呼ばれるはず: {seen:?}");
    assert!(seen[0] <= MAX_QUERY_CHARS, "切り詰められていない: {seen:?}");
}

/// どのメソッドが呼ばれたかを記録する（クエリ用・文書用の振り分けの確認）。
struct RoutingEmbedder {
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl Embedder for RoutingEmbedder {
    fn embed(&self, _: &str) -> Result<Vec<f32>, SearchError> {
        self.calls.lock().unwrap().push("plain");
        Ok(vec![1.0, 0.0, 0.0, 0.0])
    }
    fn embed_query(&self, _: &str) -> Result<Vec<f32>, SearchError> {
        self.calls.lock().unwrap().push("query");
        Ok(vec![1.0, 0.0, 0.0, 0.0])
    }
    /// 契約の文書はクエリと同じ向き、天気の文書は少し離れた向き（cosine 0.6）にして、順位を検証できるようにする。
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.calls.lock().unwrap().push("document");
        Ok(if text.contains("契約") {
            vec![1.0, 0.0, 0.0, 0.0]
        } else {
            vec![0.6, 0.8, 0.0, 0.0]
        })
    }
    fn dim(&self) -> usize {
        4
    }
    fn id(&self) -> String {
        "routing".into()
    }
}

#[test]
fn sqlite_embeds_documents_as_documents_and_queries_as_queries() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let embedder = RoutingEmbedder {
        calls: Arc::clone(&calls),
    };
    let s = SqliteStore::in_memory(ngram(), Box::new(embedder)).unwrap();
    s.upsert("a", "契約の更新日").unwrap();
    s.upsert("b", "天気予報").unwrap();
    assert_eq!(*calls.lock().unwrap(), vec!["document", "document"]);

    calls.lock().unwrap().clear();
    let hits = SemanticIndex::search(&s, "契約", 3).unwrap();
    assert_eq!(hits.len(), 2);
    // クエリ用の向きと文書用の向きが実際に比較されている（契約の文書が先頭、天気は遠い）。
    assert_eq!(hits[0].id, "a");
    assert!(hits[0].score > hits[1].score, "{hits:?}");
    assert_eq!(*calls.lock().unwrap(), vec!["query"]);

    // 語彙検索は埋め込みを使わない。
    calls.lock().unwrap().clear();
    LexicalIndex::search(&s, "契約", 3).unwrap();
    assert!(calls.lock().unwrap().is_empty());
}
