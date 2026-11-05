//! 索引の抽象。インメモリ実装と SQLite 実装を差し替え可能にする。
//!
//! `Hit.score` の意味は段階ごとに異なる（比較・合算してはいけない）:
//! - `LexicalIndex::search` … 実装依存の BM25 スコア（大きいほど良い。他実装と比較不可）
//! - `SemanticIndex::search` … cosine 類似度（0.0〜1.0）
//! - `HybridSearcher` の `Found.hits` … RRF スコア（順位のみに由来。約 0.01〜0.03）

use std::collections::HashSet;

use crate::bm25::Bm25Index;
use crate::tokenize::Tokenizer;
use crate::vector::VectorIndex;
use crate::Hit;

/// クエリとして扱う最大文字数。超えた分は切り捨てる（巨大クエリによる CPU 浪費を防ぐ）。
pub const MAX_QUERY_CHARS: usize = 4096;
/// クエリから使う異なる語の最大数（出現順）。FTS5 に巨大な OR 式を渡さないための上限。
pub const MAX_QUERY_TERMS: usize = 64;
/// 1 文書の最大バイト数。超える文書は `Config` エラーで拒否する（呼び出し側で分割して登録する）。
pub const MAX_DOC_BYTES: usize = 8 * 1024 * 1024;
/// 文書 id の最大バイト数。
pub const MAX_DOC_ID_BYTES: usize = 256;
/// 埋め込みベクトルの最大次元（SQL に埋め込む前の検証用）。
pub const MAX_EMBEDDING_DIM: usize = 4096;
/// 1 回の検索で要求できる最大件数（sqlite-vec の KNN 上限に合わせる）。
pub const MAX_K: usize = 4096;

/// クエリを `MAX_QUERY_CHARS` 文字までに切り詰める（char 境界を保つ）。
/// 語彙検索・ベクトル検索の両方（実モデルの `embed` を含む）の入口で使う。
pub fn limit_query(query: &str) -> &str {
    let end = query
        .char_indices()
        .nth(MAX_QUERY_CHARS)
        .map_or(query.len(), |(i, _)| i);
    &query[..end]
}

/// クエリをトークン化し、上限内の異なる語を出現順で返す（重複は除く）。
pub fn query_terms(tokenizer: &dyn Tokenizer, query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenizer
        .tokenize(limit_query(query))
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .take(MAX_QUERY_TERMS)
        .collect()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SearchError {
    /// 保存先（SQLite 等）の失敗。
    #[error("索引ストレージのエラー: {0}")]
    Storage(String),
    /// 設定・整合性の不一致（埋め込み次元、tokenizer の相違など）。
    #[error("設定エラー: {0}")]
    Config(String),
    /// 埋め込みモデルの読み込み・推論の失敗（ネットワーク失敗を含む）。
    #[error("埋め込みモデルのエラー: {0}")]
    Embedding(String),
    /// モデルファイルが固定したハッシュ・サイズと合わない（破損や差し替え）。
    /// 取得・展開し直せば直る可能性があり、ネットワーク失敗などの `Embedding` とは区別して扱う。
    #[error("モデルファイルの整合性エラー: {0}")]
    Integrity(String),
}

/// クエリと 1 文書の突き合わせ結果。根拠スコアの材料。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocSignals {
    /// クエリ語のうち文書に含まれる割合（0.0〜1.0）。
    pub coverage: f32,
    /// クエリの識別子（数字を含む語）を文書が含まない。
    pub missing_identifier: bool,
}

/// 数字を含む語は型番・条文番号などの識別子として完全一致を要求する。
/// （「第十二条」「三月」などの漢数字は正規化で算用数字に畳まれるため、ASCII 数字だけ見ればよい。
/// 「一度」「十分」のような日常語は畳まれないので識別子扱いにならない。）
pub fn is_identifier(term: &str) -> bool {
    term.chars().any(|c| c.is_ascii_digit())
}

/// クエリ語集合と文書の所持判定から `DocSignals` を計算する。全実装で共通。
pub fn signals(query_terms: &[String], doc_has: impl Fn(&str) -> bool) -> DocSignals {
    if query_terms.is_empty() {
        return DocSignals {
            coverage: 0.0,
            missing_identifier: false,
        };
    }
    let matched = query_terms.iter().filter(|t| doc_has(t)).count();
    DocSignals {
        coverage: matched as f32 / query_terms.len() as f32,
        missing_identifier: query_terms.iter().any(|t| is_identifier(t) && !doc_has(t)),
    }
}

/// 語彙（全文）検索。
pub trait LexicalIndex: Send + Sync {
    /// スコア降順で上位 `k` 件（スコア 0 以下は除外）。
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError>;
    /// 文書 `id` とクエリの突き合わせ。文書が存在しなければ `None`。
    /// 1 回の呼び出しで一貫した状態を読む（検索と判定の間の更新に備え、不在は呼び出し側で扱う）。
    fn doc_signals(&self, id: &str, query: &str) -> Result<Option<DocSignals>, SearchError>;
}

/// 意味（ベクトル）検索。`score` は cosine 類似度（0.0〜1.0）。
pub trait SemanticIndex: Send + Sync {
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError>;
}

impl LexicalIndex for Bm25Index {
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError> {
        Ok(Bm25Index::search(self, query, k))
    }
    fn doc_signals(&self, id: &str, query: &str) -> Result<Option<DocSignals>, SearchError> {
        Ok(Bm25Index::doc_signals(self, id, query))
    }
}

impl SemanticIndex for VectorIndex {
    fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>, SearchError> {
        VectorIndex::search(self, query, k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn signals_compute_coverage_and_missing_identifier() {
        let doc: HashSet<String> = set(&["第", "12", "条", "秘密"]).into_iter().collect();
        let s = signals(&set(&["第", "12", "条"]), |t| doc.contains(t));
        assert!((s.coverage - 1.0).abs() < 1e-6);
        assert!(!s.missing_identifier);

        let s = signals(&set(&["第", "99", "条"]), |t| doc.contains(t));
        assert!((s.coverage - 2.0 / 3.0).abs() < 1e-6);
        assert!(s.missing_identifier);
    }

    #[test]
    fn signals_for_empty_query_are_zero() {
        let s = signals(&[], |_| true);
        assert_eq!(s.coverage, 0.0);
        assert!(!s.missing_identifier);
    }

    #[test]
    fn query_terms_are_deduplicated_in_order_and_capped() {
        use crate::tokenize::NgramTokenizer;
        let t = query_terms(&NgramTokenizer, "abc def abc ghi");
        assert_eq!(t, vec!["abc", "def", "ghi"]);

        let many: String = (0..1000).map(|i| format!("w{i} ")).collect();
        let t = query_terms(&NgramTokenizer, &many);
        assert_eq!(t.len(), MAX_QUERY_TERMS);
        assert_eq!(t[0], "w0"); // 先頭（出現順）が残る
    }

    #[test]
    fn limit_query_truncates_on_char_boundaries() {
        let long = "あ".repeat(MAX_QUERY_CHARS + 10);
        let cut = limit_query(&long);
        assert_eq!(cut.chars().count(), MAX_QUERY_CHARS);
        assert_eq!(limit_query("短い"), "短い");
        assert_eq!(limit_query(""), "");
    }

    #[test]
    fn query_longer_than_the_char_limit_is_truncated_safely() {
        use crate::tokenize::NgramTokenizer;
        // 全角文字の途中で切らない（char 境界）。
        let long = "あ".repeat(MAX_QUERY_CHARS * 3);
        let t = query_terms(&NgramTokenizer, &long);
        assert!(!t.is_empty());
        assert!(t.len() <= MAX_QUERY_TERMS);
    }

    #[test]
    fn identifiers_are_terms_with_ascii_digits() {
        assert!(is_identifier("ab-1234"));
        assert!(is_identifier("12"));
        assert!(!is_identifier("契約"));
    }
}
