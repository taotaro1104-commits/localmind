//! 議事録の CSV 書き出し（表計算ソフトで開く想定）。
//!
//! - UTF-8 の BOM を付ける（Excel が文字コードを取り違えないように）。改行は CRLF（RFC 4180）。
//! - すべての欄を `"` で囲み、欄の中の `"` は `""` にする。欄の中の改行は空白にする（1 行 1 項目）。
//! - 表計算ソフトが数式として実行する先頭の記号（`=` `+` `-` `@`、全角の `＝` `＋` `－` `＠`）は、`'` を前に付けて
//!   文字として扱わせる（CSV インジェクションの対策）。判定は、先頭の空白と不可視の文字を読み飛ばした最初の文字で行う
//!   （先頭が空白でも数式として実行する表計算ソフトがあるため）。元の先頭がタブや復帰のものも同じ扱い。
//!   `-` で始まる正当な値（`-1 の差` など）にも `'` が付くが、安全を優先する。表計算ソフトによっては `'` が
//!   そのまま見える。

use super::sanitize;
use super::time::clock;
use crate::minutes::{Minutes, MinutesItem};

const BOM: char = '\u{FEFF}';
/// 数式の先頭になりうる文字（半角と全角）。
const FORMULA_STARTS: [char; 8] = ['=', '+', '-', '@', '＝', '＋', '－', '＠'];

/// 1 つの欄を、安全な CSV の欄にする。
pub(super) fn cell(text: &str) -> String {
    let flat = sanitize(text);
    let probe = flat.trim_start_matches(|c: char| c.is_whitespace());
    let formula = probe.starts_with(FORMULA_STARTS) || text.starts_with(['\t', '\r']);
    let guarded = if formula { format!("'{flat}") } else { flat };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

fn row(out: &mut String, cells: &[&str]) {
    out.push_str(&cells.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
}

fn times(citations: &[u64]) -> String {
    citations
        .iter()
        .map(|&ms| clock(ms))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn plain(out: &mut String, kind: &str, items: &[MinutesItem]) {
    for i in items {
        row(out, &[kind, &i.text, "", "", "", &times(&i.citations)]);
    }
}

/// 議事録を CSV にする。列: 区分・本文・担当者・期限・完了・根拠（録音の開始からの時刻）。
pub(super) fn render(minutes: &Minutes) -> String {
    let mut out = String::from(BOM);
    row(
        &mut out,
        &[
            "区分",
            "本文",
            "担当者",
            "期限",
            "完了",
            "根拠（録音の開始からの時刻）",
        ],
    );
    plain(&mut out, "要約", &minutes.summary);
    plain(&mut out, "決定事項", &minutes.decisions);
    for a in &minutes.action_items {
        let done = if a.done { "済" } else { "" };
        row(
            &mut out,
            &[
                "アクションアイテム",
                &a.text,
                a.assignee.as_deref().unwrap_or(""),
                a.due.as_deref().unwrap_or(""),
                done,
                &times(&a.citations),
            ],
        );
    }
    plain(&mut out, "未解決の論点", &minutes.open_issues);
    out
}
