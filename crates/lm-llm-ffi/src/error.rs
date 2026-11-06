//! FFI 越しに返すエラー（メッセージのみ。入力や LLM の出力は文言に入れない）。

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum LlmError {
    /// 入力の不備（パス・スレッド数・文字起こしの形、長すぎる文字起こし）。
    #[error("入力のエラー: {msg}")]
    InvalidInput { msg: String },
    /// モデルファイルの破損・差し替え（固定したハッシュと合わない）。入れ直せば直る。
    #[error("モデルファイルの整合性エラー: {msg}")]
    Integrity { msg: String },
    /// エンジンの失敗（モデルの形式・メモリ不足・出力が読めない）。
    #[error("LLM のエラー: {msg}")]
    Engine { msg: String },
    /// 呼び出し側の指示で中止した。
    #[error("中止されました")]
    Cancelled,
}
