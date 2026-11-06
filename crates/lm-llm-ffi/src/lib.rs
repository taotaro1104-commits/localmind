//! 議事録の LLM 抽出の FFI 層（UniFFI）。Kotlin から呼ぶ入口。
//!
//! `lm-ffi`（whisper.cpp）とは別のライブラリ（理由は Cargo.toml）。録音は `lm-ffi` の `RecordingStore` を
//! 渡せないので、文字起こしの結果（`SpokenLine`）と録音の置き場所（絶対パス）を受け取る。保存した議事録は、
//! `lm-ffi` の `RecordingStore.export_minutes` でそのまま書き出せる。
//! LLM の出力は信用せず、実在する発話を引用しない項目は議事録に入れない（`lm-ingest` の検証）。

mod engine;
mod error;

pub use engine::{MinutesEngine, MinutesEntry, MinutesReport, SpokenLine};
pub use error::LlmError;

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests;
