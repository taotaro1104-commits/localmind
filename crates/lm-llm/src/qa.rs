//! 質問への回答（資料なし）。端末の LLM だけで答えるか、必要なら Web 検索の結果（出典つき）を使って答える。
//!
//! このクレートはネットワークに出ない。検索そのものは呼び出し側（Android の Kotlin）が行い、ここでは次の 2 段階だけを担う。
//! 1. [`plan_search`]: 質問に Web 検索が要るかを LLM に判断させ、要るなら短い検索語を作らせる。
//! 2. [`answer`]: 質問（と、あれば検索結果）から回答を作る。回答の `[n]` は、実在する出典の番号だけを残す。
//!
//! 質問と検索結果は信用しないデータとして渡す（`<` を全角にして区切りを破れなくし、指示には従わせない）。

mod prompt;
pub mod wikipedia;

use crate::extractor::{GenerateError, Generator};

pub use prompt::{PLAN_GRAMMAR, PLAN_MAX_TOKENS};

/// 質問の最大文字数。
pub const MAX_QUESTION_CHARS: usize = 500;
/// 渡せる出典の数と、1 件の本文・題名の最大文字数。
pub const MAX_SOURCES: usize = 5;
pub const MAX_SOURCE_TEXT_CHARS: usize = 700;
pub const MAX_SOURCE_TITLE_CHARS: usize = 100;
/// 検索語の最大文字数。
pub const MAX_QUERY_CHARS: usize = 80;
/// 回答で生成する最大トークン数。
pub const ANSWER_MAX_TOKENS: usize = 700;
/// 回答の最大文字数（これを超えた分は切る）。
const MAX_ANSWER_CHARS: usize = 4000;
/// 出典の URL の最大文字数。
const MAX_URL_CHARS: usize = 300;

/// Web 検索で得た 1 件の出典。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSource {
    pub title: String,
    /// 利用者に見せる URL（https のみ）。
    pub url: String,
    pub text: String,
}

/// 検索が要るか。要るなら、利用者に見せたうえで検索に送る語。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPlan {
    pub need_search: bool,
    pub query: String,
}

/// 回答。`cited` は、本文が引用した出典の番号（1 始まり、重複なし、昇順）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub cited: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QaError {
    /// 質問・出典の不備。
    Invalid(String),
    /// 生成の失敗（中止・長すぎる・エンジンの失敗）。
    Generate(GenerateError),
    /// モデルの出力を読めない・空。
    BadOutput(String),
}

impl From<GenerateError> for QaError {
    fn from(e: GenerateError) -> Self {
        Self::Generate(e)
    }
}

/// 制御文字・空白（改行・タブを含む）を 1 つの空白にして、前後を詰める。
pub(crate) fn squash(text: &str) -> String {
    text.split(|c: char| c.is_control() || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 質問を整える。空・長すぎるものは断る。
pub fn check_question(question: &str) -> Result<String, QaError> {
    let q = squash(question);
    if q.is_empty() {
        return Err(QaError::Invalid("質問を入力してください".into()));
    }
    if q.chars().count() > MAX_QUESTION_CHARS {
        return Err(QaError::Invalid(format!(
            "質問が長すぎます（{MAX_QUESTION_CHARS} 文字まで）"
        )));
    }
    Ok(q)
}

/// 出典を整える（数・長さを絞り、制御文字を除く。日本語版 Wikipedia の https でない URL・空の本文の出典は捨てる）。
pub fn clean_sources(sources: &[WebSource]) -> Vec<WebSource> {
    sources
        .iter()
        .filter(|s| {
            s.url.starts_with(wikipedia::URL_PREFIX)
                && !s.url.chars().any(|c| c.is_control() || c.is_whitespace())
                && !squash(&s.text).is_empty()
        })
        .take(MAX_SOURCES)
        .map(|s| WebSource {
            title: truncate(&squash(&s.title), MAX_SOURCE_TITLE_CHARS),
            url: truncate(&squash(&s.url), MAX_URL_CHARS),
            text: truncate(&squash(&s.text), MAX_SOURCE_TEXT_CHARS),
        })
        .collect()
}

/// 時事・最新の情報を求めていそうな質問か（小さなモデルの判断を補う）。
fn looks_time_sensitive(question: &str) -> bool {
    const WORDS: [&str; 12] = [
        "最新",
        "今日",
        "昨日",
        "明日",
        "現在",
        "今年",
        "今週",
        "ニュース",
        "天気",
        "株価",
        "速報",
        "為替",
    ];
    WORDS.iter().any(|w| question.contains(w))
}

/// 検索語を整える。長すぎれば切る。URL・メール・長い数字列（電話番号など）に見えるものは、送らないよう空にする。
fn clean_query(raw: &str) -> String {
    let q = squash(raw);
    let longest_digits = q
        .split(|c: char| !c.is_ascii_digit())
        .map(str::len)
        .max()
        .unwrap_or(0);
    if q.contains("://") || q.contains("www.") || q.contains('@') || longest_digits >= 6 {
        return String::new();
    }
    truncate(&q, MAX_QUERY_CHARS)
}

/// 質問に検索が要るかを判断し、要るなら検索語を作る。
pub fn plan_search(generator: &dyn Generator, question: &str) -> Result<SearchPlan, QaError> {
    let question = check_question(question)?;
    let output = generator.generate(
        &prompt::plan_prompt(&question),
        PLAN_GRAMMAR,
        PLAN_MAX_TOKENS,
    )?;
    let (model_says, model_query) = prompt::parse_plan(&output)?;
    let need = model_says || looks_time_sensitive(&question);
    // 検索語を作れなかった・使えなかったときは、質問を代わりに送らず、端末の中だけで答える。
    let query = clean_query(&model_query);
    Ok(SearchPlan {
        need_search: need && !query.is_empty(),
        query,
    })
}

/// 回答を作る。`sources` が空なら、端末の LLM の知識だけで答える。
pub fn answer(
    generator: &dyn Generator,
    question: &str,
    sources: &[WebSource],
) -> Result<Answer, QaError> {
    let question = check_question(question)?;
    let sources = clean_sources(sources);
    // 文法なし（空）で、自由な文章を生成する。
    let output = generator.generate(
        &prompt::answer_prompt(&question, &sources),
        "",
        ANSWER_MAX_TOKENS,
    )?;
    finish_answer(&output, sources.len())
}

/// 出力を整える。実在しない出典の番号 `[n]` は、本文から消す。
pub fn finish_answer(output: &str, source_count: usize) -> Result<Answer, QaError> {
    let trimmed = truncate(output.trim(), MAX_ANSWER_CHARS);
    let chars: Vec<char> = trimmed.chars().collect();
    let mut text = String::with_capacity(trimmed.len());
    let mut cited = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            let digits: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let end = i + 1 + digits.len();
            if !digits.is_empty() && digits.len() <= 3 && chars.get(end) == Some(&']') {
                if let Ok(n) = digits.parse::<usize>() {
                    if (1..=source_count).contains(&n) {
                        if !cited.contains(&n) {
                            cited.push(n);
                        }
                        text.extend(&chars[i..=end]);
                    }
                }
                i = end + 1;
                continue;
            }
        }
        text.push(chars[i]);
        i += 1;
    }
    cited.sort_unstable();
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(QaError::BadOutput("回答を作れませんでした".into()));
    }
    Ok(Answer { text, cited })
}

#[cfg(test)]
mod tests;
