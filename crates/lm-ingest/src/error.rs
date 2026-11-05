//! ingest クレートのエラー。

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IngestError {
    /// 音声形式・設定値が不正（サンプルレート 0 など）。
    #[error("不正な設定: {0}")]
    InvalidFormat(String),
    /// 録音 id がファイル名として安全でない。
    #[error("不正な録音 id: {0}")]
    InvalidId(String),
    /// ファイル入出力の失敗。
    #[error("入出力エラー: {0}")]
    Io(String),
    /// 保存データが壊れている（ヘッダ不正、サイズ不一致など）。
    #[error("データが壊れています: {0}")]
    Corrupt(String),
    /// 議事録を抽出する LLM の失敗、または入力が不正。
    #[error("議事録の抽出に失敗しました: {0}")]
    Extract(String),
    /// 文字起こしエンジンの失敗。
    #[error("文字起こしに失敗しました: {0}")]
    Transcribe(String),
}

impl From<std::io::Error> for IngestError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
