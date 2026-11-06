//! 生成エンジン（[`Generator`]）を `MinutesExtractor` にする。エンジンは差し込み式で、llama.cpp の実装は
//! `llama` feature の [`crate::llama::LlamaGenerator`]。テストでは偽のエンジンを使う。

use lm_ingest::{IngestError, MinutesExtractor, RawMinutes};

use crate::parse::parse_minutes;
use crate::prompt::{build_prompt, MINUTES_GRAMMAR};

/// 1 回の抽出で生成する最大トークン数。
pub const MAX_NEW_TOKENS: usize = 1500;
/// 抽出に渡せる文字起こしの最大文字数（`extract_minutes` の `max_input_chars` に使う）。
/// 文脈長 8192 トークンのうち、日本語で 1 文字 ≒ 1 トークンとして、指示と出力の分を残した値。
pub const MAX_INPUT_CHARS: usize = 5000;

/// `IngestError::Extract` に入る、中止と「長すぎる」の文言（呼び出し側が種類を見分けるのに使う）。
pub const CANCELLED_MESSAGE: &str = "中止されました";
pub const TOO_LONG_MESSAGE: &str = "文字起こしが長すぎて、モデルが扱えません";

/// 生成が失敗した理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerateError {
    /// プロンプト + 生成の上限が、エンジンの文脈長を超える。
    TooLong,
    /// 中止された。
    Cancelled,
    /// そのほか（メモリ不足・エンジンの失敗）。文言にモデルの出力や入力は入れない。
    Failed(String),
}

impl From<GenerateError> for IngestError {
    fn from(e: GenerateError) -> Self {
        IngestError::Extract(match e {
            GenerateError::TooLong => TOO_LONG_MESSAGE.into(),
            GenerateError::Cancelled => CANCELLED_MESSAGE.into(),
            GenerateError::Failed(m) => m,
        })
    }
}

/// 文法で形を縛って文章を生成するエンジン。
pub trait Generator: Send + Sync {
    /// `prompt` に続けて、`grammar`（GBNF）に合う文字列を最大 `max_new_tokens` トークンまで生成する。
    /// `grammar` が空なら、形を縛らない自由な文章を生成する。
    fn generate(
        &self,
        prompt: &str,
        grammar: &str,
        max_new_tokens: usize,
    ) -> Result<String, GenerateError>;
    /// 中止を求める（実行中の生成も止める）。
    fn cancel(&self);
    /// 求めた中止を取り消す（生成が走っていないときに届いた中止が、次の生成を止めないように）。
    fn reset_cancel(&self) {}
    fn id(&self) -> String;
}

/// 議事録を LLM で抽出する。
pub struct LlmMinutesExtractor {
    generator: Box<dyn Generator>,
}

impl LlmMinutesExtractor {
    pub fn new(generator: Box<dyn Generator>) -> Self {
        Self { generator }
    }

    /// 実行中の抽出を止める。
    pub fn cancel(&self) {
        self.generator.cancel();
    }

    /// 残っている中止の要求を取り消す。
    pub fn reset_cancel(&self) {
        self.generator.reset_cancel();
    }

    /// 同じモデルで、ほかの生成（質問応答）にも使う。
    pub fn generator(&self) -> &dyn Generator {
        self.generator.as_ref()
    }
}

impl MinutesExtractor for LlmMinutesExtractor {
    fn extract(&self, transcript: &str) -> Result<RawMinutes, IngestError> {
        let prompt = build_prompt(transcript);
        let output = self
            .generator
            .generate(&prompt, MINUTES_GRAMMAR, MAX_NEW_TOKENS)?;
        parse_minutes(&output)
    }

    fn id(&self) -> String {
        self.generator.id()
    }
}

#[cfg(test)]
mod tests;
