//! ベクトル検索（インメモリ cosine）。永続版は `sqlite::SqliteStore`（sqlite-vec）。

use crate::index::{limit_query, SearchError};
use crate::tokenize::{NgramTokenizer, Tokenizer};
use crate::Hit;

/// 文 → 埋め込みベクトル。実モデル（ONNX 等）は呼び出し側が実装する。
pub trait Embedder: Send + Sync {
    /// 埋め込みを返す。モデルの読み込み・推論失敗は `Err`。長さは必ず `dim()` と一致させる。
    ///
    /// 注意: クエリと文書で入力の書式が違うモデル（e5 の `query: ` / `passage: `）では、この関数は
    /// 文書として扱う。検索クエリには必ず [`Embedder::embed_query`] を使うこと（`embed` で
    /// クエリを埋め込むと、接頭辞が違うために黙って精度が落ちる）。`VectorIndex` と `SqliteStore` は
    /// 呼び分け済み。
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError>;
    /// 検索クエリの埋め込み。クエリと文書で入力の書式が違うモデル（e5 の `query: ` / `passage: `）は上書きする。
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.embed(text)
    }
    /// 索引する文書の埋め込み。
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.embed(text)
    }
    /// 埋め込みの次元数（索引の列定義に使う）。
    fn dim(&self) -> usize;
    /// 索引との整合確認に使う識別子。モデルや前処理が変わったら必ず変える。
    fn id(&self) -> String;
}

/// 全要素が有限で、少なくとも 1 つは非ゼロ（cosine が定義できる）か。
pub fn is_usable(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite()) && v.iter().any(|x| *x != 0.0)
}

/// モデル不要のハッシュ埋め込み（トークンの feature hashing）。ベースライン／テスト用。
#[derive(Debug, Clone, Copy)]
pub struct HashingEmbedder {
    dim: usize,
}

impl HashingEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }

    fn hash(token: &str) -> u64 {
        // FNV-1a（外部依存なしで決定的）
        token.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        })
    }
}

impl Embedder for HashingEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn id(&self) -> String {
        format!("hashing-v1-{}", self.dim)
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        let mut v = vec![0.0f32; self.dim];
        for t in NgramTokenizer.tokenize(text) {
            let h = Self::hash(&t);
            let idx = (h % self.dim as u64) as usize;
            let sign = if (h >> 63) & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        Ok(v)
    }
}

/// cosine 類似度。次元が違う、またはゼロベクトルなら 0.0。
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// 埋め込みの長さが宣言次元と一致することを確認する。
pub(crate) fn check_len(embedder: &dyn Embedder, v: &[f32]) -> Result<(), SearchError> {
    if v.len() == embedder.dim() {
        Ok(())
    } else {
        Err(SearchError::Config(format!(
            "埋め込み長 {} が宣言次元 {} と一致しません",
            v.len(),
            embedder.dim()
        )))
    }
}

/// 構築後は不変のベクトル索引。
pub struct VectorIndex {
    embedder: Box<dyn Embedder>,
    items: Vec<(String, Vec<f32>)>,
}

impl VectorIndex {
    /// 同じ id が複数あれば最後のものが残る。使えない埋め込み（ゼロ・非有限）の文書は索引に載せない。
    pub fn build<I, S, T>(embedder: Box<dyn Embedder>, docs: I) -> Result<Self, SearchError>
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: AsRef<str>,
    {
        let mut items: Vec<(String, Vec<f32>)> = Vec::new();
        for (id, text) in docs {
            let id = id.into();
            let v = embedder.embed_document(text.as_ref())?;
            check_len(embedder.as_ref(), &v)?;
            items.retain(|(existing, _)| *existing != id);
            if is_usable(&v) {
                items.push((id, v));
            }
        }
        Ok(Self { embedder, items })
    }

    /// cosine 降順で上位 `k` 件（cosine ≤ 0 は除外）。同点は id 昇順。
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        let q = self.embedder.embed_query(limit_query(query))?;
        check_len(self.embedder.as_ref(), &q)?;
        if !is_usable(&q) {
            return Ok(Vec::new());
        }
        let mut hits: Vec<Hit> = self
            .items
            .iter()
            .filter_map(|(id, v)| {
                let s = cosine(&q, v);
                (s > 0.0).then(|| Hit {
                    id: id.clone(),
                    score: s,
                })
            })
            .collect();
        Hit::sort_desc(&mut hits);
        hits.truncate(k);
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn usable_vectors_are_finite_and_nonzero() {
        assert!(is_usable(&[0.0, 1.0]));
        assert!(!is_usable(&[0.0, 0.0]));
        assert!(!is_usable(&[f32::NAN, 1.0]));
        assert!(!is_usable(&[f32::INFINITY, 1.0]));
        assert!(!is_usable(&[]));
    }

    fn build(docs: &[(&str, &str)]) -> VectorIndex {
        VectorIndex::build(Box::new(HashingEmbedder::new(256)), docs.iter().copied()).unwrap()
    }

    #[test]
    fn identical_text_scores_highest() {
        let idx = build(&[
            ("a", "契約の更新日は来年三月です"),
            ("b", "今日の天気は晴れでした"),
        ]);
        let hits = idx.search("契約の更新日は来年三月です", 2).unwrap();
        assert_eq!(hits[0].id, "a");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn documents_without_tokens_are_not_indexed_and_do_not_break_search() {
        let idx = build(&[("empty", ""), ("sym", "。。。"), ("a", "契約の更新日")]);
        let hits = idx.search("契約の更新日", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a");
    }

    #[test]
    fn duplicate_ids_keep_the_last_text() {
        let idx = build(&[("x", "りんごとみかん"), ("x", "ぶどうとすいか")]);
        // 旧い内容で引いても、もう完全一致（類似度 1.0 付近）にはならない。
        let old = idx.search("りんごとみかん", 3).unwrap();
        assert!(old.iter().all(|h| h.score < 0.5), "{old:?}");
        let new = idx.search("ぶどうとすいか", 3).unwrap();
        assert_eq!(new.len(), 1);
        assert!((new[0].score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn k_zero_empty_index_and_empty_query() {
        let idx = build(&[]);
        assert!(idx.search("x", 3).unwrap().is_empty());
        assert!(idx.search("x", 0).unwrap().is_empty());
        assert!(build(&[("a", "契約")]).search("。", 3).unwrap().is_empty());
    }

    struct FixedEmbedder {
        len: usize,
        fail: bool,
    }
    impl Embedder for FixedEmbedder {
        fn embed(&self, _: &str) -> Result<Vec<f32>, SearchError> {
            if self.fail {
                Err(SearchError::Storage("model not loaded".into()))
            } else {
                Ok(vec![1.0; self.len])
            }
        }
        fn dim(&self) -> usize {
            4
        }
        fn id(&self) -> String {
            "fixed".into()
        }
    }

    #[test]
    fn embedder_with_wrong_length_is_rejected() {
        let e = FixedEmbedder {
            len: 3,
            fail: false,
        };
        let r = VectorIndex::build(Box::new(e), [("a", "x")]);
        assert!(matches!(r, Err(SearchError::Config(_))));
    }

    #[test]
    fn embedder_failure_propagates() {
        let e = FixedEmbedder { len: 4, fail: true };
        let r = VectorIndex::build(Box::new(e), [("a", "x")]);
        assert!(matches!(r, Err(SearchError::Storage(_))));
    }

    use std::sync::{Arc, Mutex};

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
        fn embed_document(&self, _: &str) -> Result<Vec<f32>, SearchError> {
            self.calls.lock().unwrap().push("document");
            Ok(vec![1.0, 0.0, 0.0, 0.0])
        }
        fn dim(&self) -> usize {
            4
        }
        fn id(&self) -> String {
            "routing".into()
        }
    }

    #[test]
    fn documents_use_the_document_embedding_and_queries_use_the_query_embedding() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let embedder = RoutingEmbedder {
            calls: Arc::clone(&calls),
        };
        let idx = VectorIndex::build(Box::new(embedder), [("a", "x"), ("b", "y")]).unwrap();
        idx.search("q", 3).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["document", "document", "query"]
        );
    }

    #[test]
    fn default_query_and_document_embeddings_fall_back_to_plain_embed() {
        // HashingEmbedder は上書きしないので、3 つのメソッドは同じ結果になる。
        let e = HashingEmbedder::new(32);
        assert_eq!(e.embed_query("契約").unwrap(), e.embed("契約").unwrap());
        assert_eq!(e.embed_document("契約").unwrap(), e.embed("契約").unwrap());
    }
}
