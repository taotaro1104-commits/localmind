//! 議事録の Markdown 書き出し。

use super::time::{clock, format_local, format_offset};
use super::{sanitize, ExportMeta};
use crate::minutes::{Minutes, MinutesItem};

/// 構造に関わる ASCII の記号を、バックスラッシュで逃がす。議事録の本文は信頼できない文字列（LLM の出力・文字起こし由来）
/// なので、見出し・リンク・画像・HTML・表・強調・コードに化けて、書き出した文書の見た目や動作を変えないようにする。
///
/// - 常に逃がす: `\` `` ` `` `*` `_` `[` `]` `<` `>` `#` `|` `&` `!` `~`。`[` `]` を逃がすので、リンクと画像は作れない
///   （`(` `)` は逃がさなくてよい）。`<` と `&` を逃がすので、HTML と文字参照も作れない。
/// - 行頭の `-` `+` と、行頭の「数字 + `.` / `)`」は、箇条書きの入れ子を作るので逃がす。
///   （それ以外の `.` `,` `-` `(` などは逃がさない。`v1.2` や `2026-11-12` が読みにくくならない。）
/// - 不可視の文字・制御文字は取り除く（[`sanitize`]）。先頭と末尾の空白は落とす。
pub(super) fn escape(text: &str) -> String {
    let clean = sanitize(text);
    let clean = clean.trim();
    let mut out = String::with_capacity(clean.len() + 8);
    let digits = clean.chars().take_while(char::is_ascii_digit).count();
    let numbered = digits > 0 && clean[digits..].starts_with(['.', ')']);
    for (i, c) in clean.chars().enumerate() {
        let special = "\\`*_[]<>#|&!~".contains(c)
            || (i == 0 && matches!(c, '-' | '+'))
            || (numbered && i == digits);
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 根拠の時刻（録音の開始からの経過時間）。`` `00:12` `` のように並べる。
fn citations(times: &[u64]) -> String {
    times
        .iter()
        .map(|&ms| format!("`{}`", clock(ms)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn section(out: &mut String, heading: &str, items: &[MinutesItem]) {
    out.push_str(&format!("## {heading}\n\n"));
    if items.is_empty() {
        out.push_str("- （なし）\n\n");
        return;
    }
    for item in items {
        out.push_str(&format!(
            "- {}  {}\n",
            escape(&item.text),
            citations(&item.citations)
        ));
    }
    out.push('\n');
}

/// 議事録を Markdown にする。根拠の時刻は、録音の開始からの経過時間（`mm:ss`）。
pub(super) fn render(minutes: &Minutes, meta: &ExportMeta) -> String {
    let mut out = format!(
        "# {}\n\n録音: {}（UTC{}）\n\n",
        escape(&meta.title),
        format_local(meta.started_at_ms, meta.utc_offset_minutes),
        format_offset(meta.utc_offset_minutes),
    );
    section(&mut out, "要約", &minutes.summary);
    section(&mut out, "決定事項", &minutes.decisions);

    out.push_str("## アクションアイテム\n\n");
    if minutes.action_items.is_empty() {
        out.push_str("- （なし）\n\n");
    }
    for a in &minutes.action_items {
        let mark = if a.done { "x" } else { " " };
        let mut notes = Vec::new();
        if let Some(who) = &a.assignee {
            notes.push(format!("担当: {}", escape(who)));
        }
        if let Some(due) = &a.due {
            notes.push(format!("期限: {}", escape(due)));
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("（{}）", notes.join(" ・ "))
        };
        out.push_str(&format!(
            "- [{mark}] {}{notes}  {}\n",
            escape(&a.text),
            citations(&a.citations)
        ));
    }
    if !minutes.action_items.is_empty() {
        out.push('\n');
    }

    section(&mut out, "未解決の論点", &minutes.open_issues);
    out.push_str("---\n\n根拠の時刻は、録音の開始からの経過時間です。");
    if minutes.rejected > 0 {
        out.push_str(&format!(
            "\n\n根拠を確認できず、載せなかった項目: {} 件",
            minutes.rejected
        ));
    }
    out.push('\n');
    out
}
