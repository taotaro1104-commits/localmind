//! GGUF の LLM（llama.cpp）による議事録の抽出（A-6）。`lm_ingest::MinutesExtractor` の実装。
//!
//! 構成:
//! - `prompt`: プロンプトと、出力の形を縛る文法（GBNF）。文字起こしは信用しないデータとして渡す。
//! - `parse`: 出力の JSON を `RawMinutes`（未検証）にする。
//! - `extractor`: 生成エンジン `Generator` を差し込み式にした `LlmMinutesExtractor`。
//! - `model`: GGUF の SHA-256 検証。
//! - `llama`（feature `llama`）: llama.cpp の実装。ネイティブのビルドに cmake が要る。
//!
//! 出力は、文法で形を縛ったうえで、`lm_ingest::minutes::validate` が根拠の実在などを検証する。

pub mod extractor;
#[cfg(feature = "llama")]
pub mod llama;
pub mod model;
pub mod parse;
pub mod prompt;

pub use extractor::{
    GenerateError, Generator, LlmMinutesExtractor, CANCELLED_MESSAGE, MAX_INPUT_CHARS,
    MAX_NEW_TOKENS, TOO_LONG_MESSAGE,
};
