//! FFI 越しに返すエラー（メッセージのみ。内部の型は漏らさない）。

use lm_router::RouteError;
use lm_search::SearchError;
use lm_workspace::WorkspaceError;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("経路エラー: {msg}")]
    Route { msg: String },
    #[error("検索エラー: {msg}")]
    Search { msg: String },
    /// 埋め込みモデルの失敗（Kotlin 側の例外を含む）。索引の失敗（`Search`）と区別できる。
    #[error("埋め込みモデルのエラー: {msg}")]
    Embedding { msg: String },
    /// モデルファイルの破損・差し替え（固定したハッシュと合わない）。展開し直せば直る可能性がある。
    #[error("モデルファイルの整合性エラー: {msg}")]
    Integrity { msg: String },
    /// 文書の取り込み・保存・出典まわりの失敗（入力の不備を含む）。
    #[error("ワークスペースのエラー: {msg}")]
    Workspace { msg: String },
    /// Kotlin 側のコールバックが例外を投げた。
    #[error("コールバックの例外: {msg}")]
    Callback { msg: String },
}

impl From<RouteError> for FfiError {
    fn from(e: RouteError) -> Self {
        Self::Route { msg: e.to_string() }
    }
}

impl From<SearchError> for FfiError {
    fn from(e: SearchError) -> Self {
        match e {
            SearchError::Embedding(msg) => Self::Embedding { msg },
            SearchError::Integrity(msg) => Self::Integrity { msg },
            other => Self::Search {
                msg: other.to_string(),
            },
        }
    }
}

impl From<WorkspaceError> for FfiError {
    fn from(e: WorkspaceError) -> Self {
        match e {
            WorkspaceError::Search(inner) => inner.into(),
            other => Self::Workspace {
                msg: other.to_string(),
            },
        }
    }
}

impl From<uniffi::UnexpectedUniFFICallbackError> for FfiError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Callback { msg: e.reason }
    }
}
