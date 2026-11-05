use lm_search::SearchError;

/// ワークスペース（文書の取り込み・分割・検索・引用）のエラー。
/// 文言にはファイルのパスを入れない（利用者の端末内の場所を画面やログに出さないため）。
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("文書が大きすぎます（{size} バイト。上限は {max} バイト）")]
    TooLarge { size: usize, max: usize },
    #[error("文書の区切りが多すぎます（上限は {max} 個）。分けて登録してください")]
    TooManyChunks { max: usize },
    #[error("登録できる資料がいっぱいです（{max} 件、または合計 {max_bytes} バイトまで）。不要な資料を削除してください")]
    Full { max: usize, max_bytes: usize },
    #[error("文書に使えない文字が含まれています: {0}")]
    InvalidText(&'static str),
    #[error("文書の id が不正です（小文字の英数字・ハイフン・アンダースコアで 1〜64 文字。OS の予約名は使えません）")]
    InvalidId,
    #[error("題名が不正です: {0}")]
    InvalidTitle(&'static str),
    #[error("文書が見つかりません")]
    NotFound,
    #[error("保存した文書を読めません（壊れているか、形式が違います）")]
    Corrupt,
    #[error("文書の保存に失敗しました: {0}")]
    Io(&'static str),
    /// 原本は保存したが、検索用の索引の更新に失敗した。`reindex` で直せる。
    #[error("資料は保存しましたが、検索の準備に失敗しました（再構築で直せます）: {0}")]
    IndexOutOfDate(String),
    #[error(transparent)]
    Search(#[from] SearchError),
}
