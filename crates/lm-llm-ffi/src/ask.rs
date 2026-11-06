//! 質問応答（資料なし）の入口。端末の LLM だけで答えるか、Kotlin が取ってきた検索結果つきで答える。
//! ここはネットワークに出ない。検索は Kotlin が行い、結果（`WebSource`）をここへ渡す。

use std::panic::{catch_unwind, AssertUnwindSafe};

use lm_llm::qa::{self, QaError};
use lm_llm::GenerateError;

use crate::engine::MinutesEngine;
use crate::error::LlmError;

/// Web 検索で得た出典 1 件。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WebSource {
    pub title: String,
    /// 利用者に見せる URL（https のみ。それ以外の出典は使わない）。
    pub url: String,
    pub text: String,
}

/// 質問に Web 検索が要るか。要るときの `query` は、検索先へ送る語（利用者に見せること）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchPlan {
    pub need_search: bool,
    pub query: String,
}

/// 質問への回答。`cited` は、本文が引用した出典の番号（`sources` の並びで 1 始まり）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnswerReport {
    pub text: String,
    pub cited: Vec<u32>,
    /// 回答に使った出典（整えたあと）。本文の `[n]` は、この並びの番号。
    pub sources: Vec<WebSource>,
    pub model_id: String,
}

fn to_core(s: &WebSource) -> qa::WebSource {
    qa::WebSource {
        title: s.title.clone(),
        url: s.url.clone(),
        text: s.text.clone(),
    }
}

fn from_core(s: &qa::WebSource) -> WebSource {
    WebSource {
        title: s.title.clone(),
        url: s.url.clone(),
        text: s.text.clone(),
    }
}

fn map_error(e: QaError) -> LlmError {
    match e {
        QaError::Invalid(msg) => LlmError::InvalidInput { msg },
        QaError::BadOutput(msg) => LlmError::Engine { msg },
        QaError::Generate(GenerateError::Cancelled) => LlmError::Cancelled,
        QaError::Generate(GenerateError::TooLong) => LlmError::InvalidInput {
            msg: "質問が長すぎます。短くして、もう一度お試しください".into(),
        },
        QaError::Generate(GenerateError::Failed(msg)) => LlmError::Engine { msg },
    }
}

/// ネイティブの推論は、パニックでも FFI の外へ出さない。
fn guarded<T>(f: impl FnOnce() -> Result<T, QaError>) -> Result<T, LlmError> {
    catch_unwind(AssertUnwindSafe(f))
        .map_err(|_| LlmError::Engine {
            msg: "回答の作成の途中で失敗しました".into(),
        })?
        .map_err(map_error)
}

/// 日本語版 Wikipedia の検索 API の応答（JSON）を、出典の一覧にする。読めない応答・不正な URL は空・除外にする。
#[uniffi::export]
pub fn parse_wikipedia_sources(json: String) -> Vec<WebSource> {
    qa::wikipedia::parse_sources(&json)
        .iter()
        .map(from_core)
        .collect()
}

/// 質問の最大文字数。
#[uniffi::export]
pub fn max_question_chars() -> u32 {
    qa::MAX_QUESTION_CHARS as u32
}

#[uniffi::export]
impl MinutesEngine {
    /// 質問に Web 検索が要るかを判断する（保存も通信もしない）。要るなら、検索先へ送る語も返す。
    pub fn plan_search(&self, question: String) -> Result<SearchPlan, LlmError> {
        let plan = guarded(|| qa::plan_search(self.extractor().generator(), &question))?;
        Ok(SearchPlan {
            need_search: plan.need_search,
            query: plan.query,
        })
    }

    /// 質問に答える。`sources` が空なら、端末の LLM の知識だけで答える。出典は、数・長さ・URL（https）を
    /// 整えてから使い、本文の `[n]` は実在する出典の番号だけを残す。
    pub fn answer(
        &self,
        question: String,
        sources: Vec<WebSource>,
    ) -> Result<AnswerReport, LlmError> {
        let core: Vec<qa::WebSource> = sources.iter().map(to_core).collect();
        let cleaned = qa::clean_sources(&core);
        let answer = guarded(|| qa::answer(self.extractor().generator(), &question, &cleaned))?;
        Ok(AnswerReport {
            text: answer.text,
            cited: answer
                .cited
                .iter()
                .filter_map(|n| u32::try_from(*n).ok())
                .collect(),
            sources: cleaned.iter().map(from_core).collect(),
            model_id: self.model_id(),
        })
    }
}
