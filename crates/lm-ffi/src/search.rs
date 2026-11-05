//! ハイブリッド検索（lm-search）の公開面。埋め込みモデルは Kotlin 側から注入する。

use crate::error::FfiError;
use lm_search::{
    Embedder, HashingEmbedder, HybridConfig, HybridSearcher, LinderaTokenizer, SearchError,
    SearchOutcome, SqliteStore,
};
use std::path::{Component, Path};
use std::sync::Arc;

/// Kotlin 側が実装する埋め込みモデル（ONNX Runtime Android など）。
#[uniffi::export(with_foreign)]
pub trait ForeignEmbedder: Send + Sync {
    /// 文書の埋め込み。長さは `dim()` と一致させる。
    fn embed_document(&self, text: String) -> Result<Vec<f32>, FfiError>;
    /// 検索クエリの埋め込み（e5 の `query: ` 接頭辞など）。
    fn embed_query(&self, text: String) -> Result<Vec<f32>, FfiError>;
    /// 埋め込みの次元数。`open` のときに 1 度だけ読む（途中で変わる実装は想定しない）。
    fn dim(&self) -> Result<u32, FfiError>;
    /// モデルや前処理が変わったら必ず変える（索引との整合確認に使う）。`open` のときに 1 度だけ読む。
    fn id(&self) -> Result<String, FfiError>;
}

/// Kotlin の埋め込みを `lm-search` の `Embedder` に繋ぐ。`dim` と `id` は作成時に 1 度だけ読んで
/// 保持する（毎回呼ぶと、例外や値の変化が Rust 側のパニックや次元検査の迂回になる）。
struct Bridge {
    inner: Arc<dyn ForeignEmbedder>,
    dim: usize,
    id: String,
}

impl Bridge {
    fn new(inner: Arc<dyn ForeignEmbedder>) -> Result<Self, FfiError> {
        let dim = inner.dim()? as usize;
        let id = inner.id()?;
        Ok(Self { inner, dim, id })
    }
}

/// コールバックの失敗は、二重に包まずメッセージだけを `Embedding` として渡す。
fn to_search(e: FfiError) -> SearchError {
    let (FfiError::Route { msg }
    | FfiError::Search { msg }
    | FfiError::Embedding { msg }
    | FfiError::Callback { msg }) = e;
    SearchError::Embedding(msg)
}

impl Embedder for Bridge {
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.inner
            .embed_document(text.to_string())
            .map_err(to_search)
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.inner.embed_query(text.to_string()).map_err(to_search)
    }
    fn dim(&self) -> usize {
        self.dim
    }
    fn id(&self) -> String {
        self.id.clone()
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SearchHit {
    pub id: String,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SearchResult {
    /// false なら「資料には見つかりません」（`hits` は空）。
    pub found: bool,
    pub hits: Vec<SearchHit>,
    pub grounding: f32,
}

/// 検索の設定。実モデルでは評価セットで測った値を渡す（e5 は `e5_search_config()`）。
/// 不正な値（NaN・負の閾値・範囲外の下駄）は `open` が拒否する。
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct SearchConfig {
    pub grounding_threshold: f32,
    pub semantic_floor: f32,
}

impl SearchConfig {
    /// 設定値の検証（不正なら `open` の時点で拒否する）。
    pub(crate) fn validate(self) -> Result<(), FfiError> {
        Ok(self.to_hybrid().validate()?)
    }

    pub(crate) fn to_hybrid(self) -> HybridConfig {
        HybridConfig {
            grounding_threshold: self.grounding_threshold,
            semantic_floor: self.semantic_floor,
            ..HybridConfig::default()
        }
    }
}

/// multilingual-e5-small 向けの推奨設定（評価セットで測定済みの値）。
#[uniffi::export]
pub fn e5_search_config() -> SearchConfig {
    let c = HybridConfig::onnx_e5();
    SearchConfig {
        grounding_threshold: c.grounding_threshold,
        semantic_floor: c.semantic_floor,
    }
}

#[derive(uniffi::Object)]
pub struct SearchEngine {
    store: Arc<SqliteStore>,
    searcher: HybridSearcher,
}

/// 1 文書の最大バイト数。モバイルのヒープを守るため、`lm-search` の上限（8 MiB）より小さくする。
/// 長い文書は呼び出し側でチャンクに分けて登録する。
pub const MAX_NOTE_BYTES: usize = 256 * 1024;

/// Kotlin から受け取るパスは、アプリ専用領域の絶対パスだけを受け付ける（相対パスはプロセスの cwd に
/// 依存し、`..` は領域の外へ出られるため拒否する）。`what` はエラー文に使う名前。
pub(crate) fn check_abs_path(path: &str, what: &str) -> Result<(), FfiError> {
    let p = Path::new(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(FfiError::Search {
            msg: format!("{what}は `..` を含まない絶対パスで指定してください"),
        });
    }
    Ok(())
}

pub(crate) fn build_engine(store: SqliteStore, config: SearchConfig) -> Arc<SearchEngine> {
    let store = Arc::new(store);
    let searcher = HybridSearcher::new(store.clone(), store.clone(), config.to_hybrid());
    Arc::new(SearchEngine { store, searcher })
}

#[uniffi::export]
impl SearchEngine {
    /// 永続索引を開く（無ければ作る）。`path` はアプリ専用ディレクトリ内のファイル。
    #[uniffi::constructor]
    pub fn open(
        path: String,
        embedder: Arc<dyn ForeignEmbedder>,
        config: SearchConfig,
    ) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&path, "索引のパス")?;
        config.validate()?;
        let store = SqliteStore::open(
            path,
            Box::new(LinderaTokenizer::new()?),
            Box::new(Bridge::new(embedder)?),
        )?;
        Ok(build_engine(store, config))
    }

    /// モデル不要のハッシュ埋め込みで開く（動作確認・モデル未導入時の字面検索用）。
    #[uniffi::constructor]
    pub fn open_lexical(path: String) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&path, "索引のパス")?;
        let store = SqliteStore::open(
            path,
            Box::new(LinderaTokenizer::new()?),
            Box::new(HashingEmbedder::new(256)),
        )?;
        Ok(build_engine(
            store,
            SearchConfig::from(HybridConfig::default()),
        ))
    }

    pub fn upsert(&self, id: String, text: String) -> Result<(), FfiError> {
        if text.len() > MAX_NOTE_BYTES {
            return Err(FfiError::Search {
                msg: format!(
                    "文書が大きすぎます（{MAX_NOTE_BYTES} バイトまで。分割して登録してください）"
                ),
            });
        }
        Ok(self.store.upsert(&id, &text)?)
    }

    pub fn remove(&self, id: String) -> Result<bool, FfiError> {
        Ok(self.store.remove(&id)?)
    }

    pub fn len(&self) -> Result<u64, FfiError> {
        Ok(self.store.len()? as u64)
    }

    pub fn is_empty(&self) -> Result<bool, FfiError> {
        Ok(self.store.is_empty()?)
    }

    pub fn search(&self, query: String, k: u32) -> Result<SearchResult, FfiError> {
        Ok(match self.searcher.search(&query, k as usize)? {
            SearchOutcome::Found { hits, grounding } => SearchResult {
                found: true,
                hits: hits
                    .into_iter()
                    .map(|h| SearchHit {
                        id: h.id,
                        score: h.score,
                    })
                    .collect(),
                grounding,
            },
            SearchOutcome::NotFound { grounding } => SearchResult {
                found: false,
                hits: vec![],
                grounding,
            },
        })
    }
}

impl From<HybridConfig> for SearchConfig {
    fn from(c: HybridConfig) -> Self {
        Self {
            grounding_threshold: c.grounding_threshold,
            semantic_floor: c.semantic_floor,
        }
    }
}
