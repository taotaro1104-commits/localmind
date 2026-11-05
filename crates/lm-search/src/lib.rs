//! 日本語に強いハイブリッド検索: 正規化 → BM25 + ベクトル → RRF → 根拠スコア。
//!
//! 既定は依存なしのインメモリ版（bi-gram + BM25 + ハッシュ埋め込み）。
//! feature `lindera` で形態素解析、feature `sqlite` で永続索引（FTS5 + sqlite-vec）が使え、
//! どちらも `Tokenizer` / `LexicalIndex` / `SemanticIndex` の実装として差し替わる。
//! 実際の埋め込みモデルは `Embedder` の実装として差し替える。

pub mod bm25;
pub mod eval;
pub mod grounding;
pub mod index;
#[cfg(feature = "lindera")]
pub mod lindera_tokenizer;
pub mod normalize;
#[cfg(feature = "onnx")]
pub mod onnx;
#[cfg(feature = "onnx")]
mod onnx_pin;
pub mod rrf;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod tokenize;
pub mod vector;

pub use bm25::Bm25Index;
pub use grounding::{HybridConfig, HybridSearcher, SearchOutcome};
pub use index::{DocSignals, LexicalIndex, SearchError, SemanticIndex};
#[cfg(feature = "lindera")]
pub use lindera_tokenizer::LinderaTokenizer;
pub use normalize::normalize;
#[cfg(feature = "onnx")]
pub use onnx::OnnxEmbedder;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteStore;
pub use tokenize::{NgramTokenizer, Tokenizer};
pub use vector::{Embedder, HashingEmbedder, VectorIndex};

/// 検索ヒット（文書 id とスコア）。
///
/// `score` の意味は生成元で異なる（BM25 / cosine / RRF）。段階をまたいで比較しないこと。
/// 詳細は [`index`] のモジュール doc を参照。
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub score: f32,
}

impl Hit {
    /// スコア降順、同点は id 昇順に並べる（結果を決定的にする）。
    pub(crate) fn sort_desc(hits: &mut [Hit]) {
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    }
}
