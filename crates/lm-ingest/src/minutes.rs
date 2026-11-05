//! 議事録の抽出（A-6）とアクションアイテム（A-8）。
//!
//! LLM は [`MinutesExtractor`] の実装として呼び出し側が差し込む（Nano・GGUF など）。このモジュールは
//! 入力の組み立てと、LLM の出力の検証だけを行う。LLM の出力は信用せず、次を満たすものだけを採用する。
//!
//! - **根拠のある項目だけ**: 各項目は、実在するセグメントの開始位置（ms）を 1 つ以上引用していること。
//!   引用が無い・実在しない項目は載せない（捏造を議事録に混ぜない）。引用からタップで該当音声へ戻れる。
//! - 本文・担当者・期限は、制御文字や不可視文字を除去し、長さと件数を制限する。

use crate::error::IngestError;
use crate::transcript::{clean_speaker, one_line, Transcript};

/// 1 項目の本文の最大文字数。
pub const MAX_ITEM_CHARS: usize = 1000;
/// 1 区分あたりの最大件数。超えた分は採用しない。
pub const MAX_ITEMS_PER_SECTION: usize = 200;
/// 1 項目が引用できる根拠の最大数。
pub const MAX_CITATIONS: usize = 10;

/// LLM が返す 1 項目（未検証）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawItem {
    pub text: String,
    /// 担当者（アクションアイテムのみ）。
    pub assignee: Option<String>,
    /// 期限（アクションアイテムのみ。自由記述: 「来週金曜」など）。
    pub due: Option<String>,
    /// 根拠にしたセグメントの開始位置（ms）。入力に `[<ms>]` で示したもの。
    pub cited_start_ms: Vec<u64>,
}

/// LLM が返す議事録（未検証）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawMinutes {
    pub summary: Vec<RawItem>,
    pub decisions: Vec<RawItem>,
    pub action_items: Vec<RawItem>,
    pub open_issues: Vec<RawItem>,
}

/// 議事録を抽出する LLM。入力は [`render_for_llm`] の形式（1 行 1 セグメント）。
pub trait MinutesExtractor: Send + Sync {
    fn extract(&self, transcript: &str) -> Result<RawMinutes, IngestError>;
    /// モデルの識別子（呼び出し側が再抽出の要否判断などに使う。保存形式には含まれない）。
    fn id(&self) -> String;
}

/// 検証済みの項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinutesItem {
    pub text: String,
    /// 根拠のセグメントの開始位置（昇順・重複なし。必ず 1 つ以上）。
    pub citations: Vec<u64>,
}

/// 検証済みのアクションアイテム（ACTION_ITEM）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionItem {
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub citations: Vec<u64>,
    /// 完了済みか（利用者が付ける）。
    pub done: bool,
}

/// 検証済みの議事録。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Minutes {
    pub summary: Vec<MinutesItem>,
    pub decisions: Vec<MinutesItem>,
    pub action_items: Vec<ActionItem>,
    pub open_issues: Vec<MinutesItem>,
    /// 根拠が無い・不正などの理由で採用しなかった項目の数。
    pub rejected: usize,
}

/// 検証をくぐった項目: `(元の項目, 有効な引用, 無害化した本文)`。
type Checked = (RawItem, Vec<u64>, String);

/// LLM への入力: 1 行 1 セグメントで `[<開始ms>] 話者: 本文`。引用はこの開始位置で返させる。
pub fn render_for_llm(transcript: &Transcript) -> String {
    transcript
        .segments()
        .iter()
        .map(|s| match &s.speaker {
            Some(sp) => format!("[{}] {sp}: {}\n", s.start_ms, s.text),
            None => format!("[{}] {}\n", s.start_ms, s.text),
        })
        .collect()
}

/// 文字起こしから議事録を抽出する。入力が `max_input_chars` 文字を超えるときはエラー
/// （呼び出し側で区間に分けて抽出する）。空の文字起こしでは LLM を呼ばない。
pub fn extract_minutes(
    transcript: &Transcript,
    extractor: &dyn MinutesExtractor,
    max_input_chars: usize,
) -> Result<Minutes, IngestError> {
    if transcript.is_empty() {
        return Ok(Minutes::default());
    }
    let input = render_for_llm(transcript);
    if input.chars().count() > max_input_chars {
        return Err(IngestError::Extract(format!(
            "文字起こしが長すぎます（上限 {max_input_chars} 文字）。区間に分けて抽出してください"
        )));
    }
    let raw = extractor.extract(&input)?;
    Ok(validate(transcript, raw))
}

/// LLM の出力を検証する（根拠の実在・本文の無害化・長さと件数の制限）。
pub fn validate(transcript: &Transcript, raw: RawMinutes) -> Minutes {
    let known: Vec<u64> = transcript.segments().iter().map(|s| s.start_ms).collect();
    let mut rejected = 0;
    let mut section = |items: Vec<RawItem>| -> Vec<Checked> {
        let mut out = Vec::new();
        for item in items {
            match checked(&known, item) {
                Some(v) if out.len() < MAX_ITEMS_PER_SECTION => out.push(v),
                _ => rejected += 1,
            }
        }
        out
    };
    let summary = section(raw.summary);
    let decisions = section(raw.decisions);
    let action_items = section(raw.action_items);
    let open_issues = section(raw.open_issues);
    let plain = |v: Vec<Checked>| -> Vec<MinutesItem> {
        v.into_iter()
            .map(|(_, citations, text)| MinutesItem { text, citations })
            .collect()
    };
    Minutes {
        summary: plain(summary),
        decisions: plain(decisions),
        action_items: action_items
            .into_iter()
            .map(|(item, citations, text)| ActionItem {
                text,
                assignee: item.assignee.as_deref().and_then(short_label),
                due: item.due.as_deref().and_then(short_label),
                citations,
                done: false,
            })
            .collect(),
        open_issues: plain(open_issues),
        rejected,
    }
}

/// 項目を検証する。採用できなければ None（本文が空・長すぎる、有効な引用が無い）。
fn checked(known: &[u64], item: RawItem) -> Option<Checked> {
    let text = one_line(&item.text).trim().to_string();
    if text.is_empty() || text.chars().count() > MAX_ITEM_CHARS {
        return None;
    }
    let mut citations: Vec<u64> = item
        .cited_start_ms
        .iter()
        .copied()
        .filter(|ms| known.binary_search(ms).is_ok())
        .collect();
    citations.sort_unstable();
    citations.dedup();
    citations.truncate(MAX_CITATIONS);
    if citations.is_empty() {
        return None;
    }
    Some((item, citations, text))
}

/// 担当者・期限の短いラベル。空になれば None。
fn short_label(s: &str) -> Option<String> {
    Some(clean_speaker(s).trim().to_string()).filter(|v| !v.is_empty())
}

impl Minutes {
    pub fn is_empty(&self) -> bool {
        self.summary.is_empty()
            && self.decisions.is_empty()
            && self.action_items.is_empty()
            && self.open_issues.is_empty()
    }

    /// アクションアイテムの完了を切り替える。範囲外なら false。
    pub fn set_done(&mut self, index: usize, done: bool) -> bool {
        match self.action_items.get_mut(index) {
            Some(item) => {
                item.done = done;
                true
            }
            None => false,
        }
    }

    /// 人が読む形式（共有・書き出し用）。
    pub fn to_text(&self) -> String {
        let items =
            |v: &[MinutesItem]| -> Vec<String> { v.iter().map(|i| i.text.clone()).collect() };
        [
            ("要約", items(&self.summary)),
            ("決定事項", items(&self.decisions)),
            ("ToDo", self.action_items.iter().map(action_line).collect()),
            ("未解決の論点", items(&self.open_issues)),
        ]
        .into_iter()
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(title, lines)| {
            let body: String = lines.iter().map(|l| format!("- {l}\n")).collect();
            format!("## {title}\n{body}\n")
        })
        .collect()
    }
}

fn action_line(a: &ActionItem) -> String {
    let mark = if a.done { "[x]" } else { "[ ]" };
    let who = a
        .assignee
        .as_deref()
        .map(|w| format!(" ({w})"))
        .unwrap_or_default();
    let due = a
        .due
        .as_deref()
        .map(|d| format!(" 期限: {d}"))
        .unwrap_or_default();
    format!("{mark} {}{who}{due}", a.text)
}

pub(crate) mod record;

#[cfg(test)]
mod tests;
