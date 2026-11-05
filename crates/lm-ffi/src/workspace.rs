//! ワークスペース（文書の取り込み・分割・検索・位置つきの引用）の公開面。
//!
//! 文書はテキストか Markdown。見出しと段落で分け、検索結果には出典（文書・見出し・行番号・抜粋）がつく。

use std::sync::Arc;

use crate::error::FfiError;
use crate::search::{check_abs_path, Bridge, ForeignEmbedder, SearchConfig};
use lm_search::{
    Embedder, HashingEmbedder, HybridConfig, HybridSearcher, LinderaTokenizer, SqliteStore,
};
use lm_workspace::{DocStore, Format, Workspace};

/// 文書の形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DocFormat {
    Text,
    Markdown,
}

impl From<DocFormat> for Format {
    fn from(f: DocFormat) -> Self {
        match f {
            DocFormat::Text => Format::Text,
            DocFormat::Markdown => Format::Markdown,
        }
    }
}

impl From<Format> for DocFormat {
    fn from(f: Format) -> Self {
        match f {
            Format::Text => DocFormat::Text,
            Format::Markdown => DocFormat::Markdown,
        }
    }
}

/// 登録済みの文書。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DocumentInfo {
    pub id: String,
    pub title: String,
    pub format: DocFormat,
    pub chunks: u32,
}

/// 検索結果 1 件の出典。行番号は 1 始まりで、`end_line` を含む。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Citation {
    pub doc_id: String,
    pub title: String,
    pub heading_path: Vec<String>,
    pub start_line: u32,
    pub end_line: u32,
    pub excerpt: String,
    pub score: f32,
}

/// 検索の結果。`found == false` なら「資料には見つかりません」（`citations` は空）。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WorkspaceReport {
    pub found: bool,
    pub grounding: f32,
    pub citations: Vec<Citation>,
}

#[derive(uniffi::Object)]
pub struct WorkspaceEngine {
    inner: Workspace,
}

fn build(
    store: SqliteStore,
    config: SearchConfig,
    docs_dir: String,
    margin: f32,
) -> Result<Arc<WorkspaceEngine>, FfiError> {
    let store = Arc::new(store);
    let searcher = HybridSearcher::new(store.clone(), store.clone(), config.to_hybrid());
    let inner =
        Workspace::new(store, searcher, DocStore::new(docs_dir)).with_relevance_margin(margin);
    Ok(Arc::new(WorkspaceEngine { inner }))
}

/// 索引を開く共通部分。呼び出し側の入力（パス・設定）を検査してから、埋め込みを組み合わせる。
pub(crate) fn open_with(
    path: String,
    docs_dir: String,
    config: SearchConfig,
    embedder: Box<dyn Embedder>,
    margin: f32,
) -> Result<Arc<WorkspaceEngine>, FfiError> {
    check_abs_path(&path, "索引のパス")?;
    check_abs_path(&docs_dir, "文書の保存先")?;
    config.validate()?;
    let store = SqliteStore::open(path, Box::new(LinderaTokenizer::new()?), embedder)?;
    build(store, config, docs_dir, margin)
}

#[uniffi::export]
impl WorkspaceEngine {
    /// 埋め込みモデルを Kotlin から注入して開く。`path` は索引、`docs_dir` は文書の原本の保存先。
    #[uniffi::constructor]
    pub fn open(
        path: String,
        docs_dir: String,
        embedder: Arc<dyn ForeignEmbedder>,
        config: SearchConfig,
    ) -> Result<Arc<Self>, FfiError> {
        // 注入された埋め込みの cosine の分布は分からないので、資料ごとの足切りはしない。
        open_with(
            path,
            docs_dir,
            config,
            Box::new(Bridge::new(embedder)?),
            f32::INFINITY,
        )
    }

    /// モデル不要のハッシュ埋め込みで開く（動作確認・モデル未導入時の字面検索用）。
    #[uniffi::constructor]
    pub fn open_lexical(path: String, docs_dir: String) -> Result<Arc<Self>, FfiError> {
        open_with(
            path,
            docs_dir,
            SearchConfig::from(HybridConfig::default()),
            Box::new(HashingEmbedder::new(256)),
            f32::INFINITY,
        )
    }

    /// 文書を追加する（同じ id があれば置き換える）。作った断片の数を返す。
    /// `id` は小文字の英数字・ハイフン・アンダースコアの 1〜64 文字（OS の予約名は不可）、本文は 256 KiB まで。
    /// 資料は 200 件・合計 16 MiB まで。保存したが検索の準備に失敗したときは、エラーの文言でそう分かる
    /// （`reindex` で直せる）。
    pub fn add_document(
        &self,
        id: String,
        title: String,
        format: DocFormat,
        text: String,
    ) -> Result<u32, FfiError> {
        Ok(self.inner.add(&id, &title, format.into(), &text)? as u32)
    }

    /// 文書を消す。無ければ `false`。
    pub fn remove_document(&self, id: String) -> Result<bool, FfiError> {
        Ok(self.inner.remove(&id)?)
    }

    /// 登録済みの文書（id 順）。
    pub fn documents(&self) -> Result<Vec<DocumentInfo>, FfiError> {
        Ok(self
            .inner
            .list()?
            .into_iter()
            .map(|d| DocumentInfo {
                id: d.id,
                title: d.title,
                format: d.format.into(),
                chunks: d.chunks as u32,
            })
            .collect())
    }

    /// 開いた直後に 1 度呼ぶ。分割方式が変わっていたら、原本から索引を作り直す（作り直したら `true`）。
    /// 時間がかかるので UI スレッドから呼ばないこと。失敗しても開いたエンジンは使える（`reindex` で直せる）。
    pub fn ensure_current(&self) -> Result<bool, FfiError> {
        Ok(self.inner.ensure_current()?)
    }

    /// 原本から索引を作り直す。読めなかった文書の id を返す。
    pub fn reindex(&self) -> Result<Vec<String>, FfiError> {
        Ok(self.inner.reindex()?)
    }

    /// 質問で検索する。結果は近い順で、各件に出典がつく。`k` は 50 件までに丸める。
    pub fn search(&self, query: String, k: u32) -> Result<WorkspaceReport, FfiError> {
        let r = self.inner.search(&query, k as usize)?;
        Ok(WorkspaceReport {
            found: r.found,
            grounding: r.grounding,
            citations: r
                .citations
                .into_iter()
                .map(|c| Citation {
                    doc_id: c.doc_id,
                    title: c.title,
                    heading_path: c.heading_path,
                    start_line: c.start_line as u32,
                    end_line: c.end_line as u32,
                    excerpt: c.excerpt,
                    score: c.score,
                })
                .collect(),
        })
    }
}
