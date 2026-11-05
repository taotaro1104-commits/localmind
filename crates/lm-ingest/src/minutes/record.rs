//! 議事録の保存形式（テキスト、1 行 1 項目、タブ区切り）。
//!
//! ```text
//! LMMIN 1
//! rejected\t<件数>
//! S\t<引用 ms をカンマ区切り>\t<本文>          （要約）
//! D\t<引用>\t<本文>                          （決定事項）
//! O\t<引用>\t<本文>                          （未解決の論点）
//! A\t<done: 0|1>\t<引用>\t<担当者>\t<期限>\t<本文>   （アクションアイテム）
//! ```
//!
//! 本文・担当者・期限は登録時に制御文字（タブ・改行を含む）が空白に置き換わっているので、
//! エスケープは要らない。読み込み時も同じ上限で検証し直す（保存先のファイルは信用しない）。

use super::{
    ActionItem, Minutes, MinutesItem, MAX_CITATIONS, MAX_ITEMS_PER_SECTION, MAX_ITEM_CHARS,
};
use crate::error::IngestError;
use crate::transcript::{is_invisible_format, MAX_SPEAKER_CHARS};

const HEADER: &str = "LMMIN 1";

fn corrupt(msg: impl Into<String>) -> IngestError {
    IngestError::Corrupt(format!("議事録: {}", msg.into()))
}

fn cites(c: &[u64]) -> String {
    c.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

/// 保存用のテキストにする。
pub(crate) fn encode(m: &Minutes) -> String {
    let mut out = format!("{HEADER}\nrejected\t{}\n", m.rejected);
    let mut items = |tag: &str, v: &[MinutesItem]| {
        for i in v {
            out.push_str(&format!("{tag}\t{}\t{}\n", cites(&i.citations), i.text));
        }
    };
    items("S", &m.summary);
    items("D", &m.decisions);
    items("O", &m.open_issues);
    for a in &m.action_items {
        out.push_str(&format!(
            "A\t{}\t{}\t{}\t{}\t{}\n",
            u8::from(a.done),
            cites(&a.citations),
            a.assignee.as_deref().unwrap_or(""),
            a.due.as_deref().unwrap_or(""),
            a.text
        ));
    }
    out
}

fn parse_cites(s: &str) -> Result<Vec<u64>, IngestError> {
    let v: Result<Vec<u64>, _> = s.split(',').map(str::parse).collect();
    let v = v.map_err(|_| corrupt("引用の形式が不正です"))?;
    if v.is_empty() || v.len() > MAX_CITATIONS || v.windows(2).any(|w| w[0] >= w[1]) {
        return Err(corrupt("引用が 0 件・多すぎる・昇順でない"));
    }
    Ok(v)
}

/// 制御文字・行区切り・表示を偽装できる書式文字（登録時に取り除いているもの）を含むか。
fn has_unsafe_char(s: &str) -> bool {
    s.chars()
        .any(|c| c.is_control() || is_invisible_format(c) || matches!(c, '\u{2028}' | '\u{2029}'))
}

fn parse_text(s: &str) -> Result<String, IngestError> {
    if s.trim().is_empty() || s.chars().count() > MAX_ITEM_CHARS || has_unsafe_char(s) {
        return Err(corrupt("本文が空・長すぎる・制御文字を含む"));
    }
    Ok(s.to_string())
}

fn parse_label(s: &str) -> Result<Option<String>, IngestError> {
    if s.chars().count() > MAX_SPEAKER_CHARS || has_unsafe_char(s) {
        return Err(corrupt("担当者・期限が長すぎる・制御文字を含む"));
    }
    Ok(Some(s.to_string()).filter(|v| !v.trim().is_empty()))
}

/// 保存されたテキストを読む。形式・件数・長さを検証し、不正なら `Corrupt`。
pub(crate) fn decode(text: &str) -> Result<Minutes, IngestError> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(corrupt("形式の版が未対応です"));
    }
    let rejected = lines
        .next()
        .and_then(|l| l.strip_prefix("rejected\t"))
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| corrupt("rejected の行が不正です"))?;
    let mut m = Minutes {
        rejected,
        ..Minutes::default()
    };
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        let item = |f: &[&str]| -> Result<MinutesItem, IngestError> {
            match f {
                [_, c, t] => Ok(MinutesItem {
                    text: parse_text(t)?,
                    citations: parse_cites(c)?,
                }),
                _ => Err(corrupt("項目の列の数が不正です")),
            }
        };
        let section = match f.first().copied() {
            Some("S") => &mut m.summary,
            Some("D") => &mut m.decisions,
            Some("O") => &mut m.open_issues,
            Some("A") => {
                if m.action_items.len() >= MAX_ITEMS_PER_SECTION {
                    return Err(corrupt("アクションアイテムが多すぎます"));
                }
                let [_, done, c, who, due, t] = f.as_slice() else {
                    return Err(corrupt("アクションアイテムの列の数が不正です"));
                };
                m.action_items.push(ActionItem {
                    text: parse_text(t)?,
                    assignee: parse_label(who)?,
                    due: parse_label(due)?,
                    citations: parse_cites(c)?,
                    done: match *done {
                        "0" => false,
                        "1" => true,
                        _ => return Err(corrupt("done は 0 か 1 です")),
                    },
                });
                continue;
            }
            _ => return Err(corrupt("未知の行です")),
        };
        if section.len() >= MAX_ITEMS_PER_SECTION {
            return Err(corrupt("項目が多すぎます"));
        }
        section.push(item(&f)?);
    }
    Ok(m)
}
