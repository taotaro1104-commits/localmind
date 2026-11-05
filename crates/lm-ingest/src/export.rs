//! 議事録の書き出し（F-3）: Markdown・CSV・iCalendar（ToDo）。
//!
//! 書き出す文字列は、議事録の本文（LLM の出力・文字起こし由来で、信頼できない）をそのまま含む。そのため
//! 形式ごとに、記号の逃がしと、制御文字・不可視文字の除去を行い、書き出した文書の構造や動作を本文が変えないようにする
//! （Markdown は見出し・リンク・HTML、CSV は表計算の数式、iCalendar は行・部品の偽造）。
//! 純粋な文字列変換で、ファイルへの書き込みや共有は呼び出し側が行う。

mod csv;
mod ics;
mod markdown;
mod time;

use crate::error::IngestError;
use crate::minutes::{Minutes, MinutesItem, MAX_ITEMS_PER_SECTION, MAX_ITEM_CHARS};
use crate::store::validate_recording_id;
use crate::transcript::{is_invisible_format, one_line};

/// 題名の最大文字数。
const MAX_TITLE_CHARS: usize = 100;
/// 既定の題名。
const DEFAULT_TITLE: &str = "議事録";
/// 時刻として受け付ける範囲（2024-01-01 から 2200-01-01 まで）。下限は、秒の値をミリ秒と取り違えて渡すのを
/// 防ぐためでもある（秒なら 1970 年ごろになり、この範囲に入らない）。
const MIN_MS: u64 = 1_704_067_200_000;
const MAX_MS: u64 = 7_258_118_400_000;
/// UTC からの差の範囲（分）。実在するのは -12:00 〜 +14:00。
const OFFSET_RANGE: std::ops::RangeInclusive<i32> = -14 * 60..=14 * 60;

/// 書き出しの形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Csv,
    /// アクションアイテムを ToDo にした iCalendar。
    Ics,
}

/// 書き出しに添える情報。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportMeta {
    /// 録音 id（ファイル名と、ToDo の一意な識別子に使う）。
    pub recording_id: String,
    /// 議事録の題名（1 行にして使う）。空なら「議事録」。
    pub title: String,
    /// 録音の開始時刻（UNIX ミリ秒）。
    pub started_at_ms: u64,
    /// 書き出す時刻（UNIX ミリ秒）。
    pub exported_at_ms: u64,
    /// 現地の UTC からの差（分。日本は 540）。録音の日時の表示に使う。
    pub utc_offset_minutes: i32,
}

/// 書き出した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// 保存・共有するときのファイル名（`minutes-<録音id>.<拡張子>`）。
    pub file_name: String,
    pub mime_type: &'static str,
    pub text: String,
}

fn invalid(msg: &str) -> IngestError {
    IngestError::InvalidFormat(msg.into())
}

/// 書き出す文字列から、不可視の書式文字（ゼロ幅・双方向制御・BOM など）を取り除き、制御文字と行区切り
/// （U+2028/2029）を空白にする。保存時にも除いているが、公開関数として単独でも守る（多層防御）。
pub(super) fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|&c| !is_invisible_format(c))
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// 議事録の大きさが、保存時の上限（区分ごとの件数と 1 項目の文字数）以内か。
fn check_size(minutes: &Minutes) -> Result<(), IngestError> {
    let long = |items: &[MinutesItem]| {
        items
            .iter()
            .any(|i| i.text.chars().count() > MAX_ITEM_CHARS)
    };
    let counts = [
        minutes.summary.len(),
        minutes.decisions.len(),
        minutes.action_items.len(),
        minutes.open_issues.len(),
    ];
    let too_long = long(&minutes.summary)
        || long(&minutes.decisions)
        || long(&minutes.open_issues)
        || minutes
            .action_items
            .iter()
            .any(|a| a.text.chars().count() > MAX_ITEM_CHARS);
    if too_long || counts.iter().any(|&n| n > MAX_ITEMS_PER_SECTION) {
        return Err(invalid(
            "議事録が大きすぎます（件数か 1 項目の長さが上限を超えています）",
        ));
    }
    Ok(())
}

/// 入力を検査し、題名を整える。
fn checked(meta: &ExportMeta) -> Result<ExportMeta, IngestError> {
    validate_recording_id(&meta.recording_id)?;
    if !(MIN_MS..=MAX_MS).contains(&meta.started_at_ms) {
        return Err(invalid(
            "録音の開始時刻が不正です（ミリ秒の UNIX 時刻が要ります）",
        ));
    }
    if !(MIN_MS..=MAX_MS).contains(&meta.exported_at_ms) {
        return Err(invalid(
            "書き出す時刻が不正です（ミリ秒の UNIX 時刻を渡してください）",
        ));
    }
    if !OFFSET_RANGE.contains(&meta.utc_offset_minutes) {
        return Err(invalid("UTC からの差が不正です"));
    }
    let cut: String = sanitize(&one_line(&meta.title))
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let title = cut.trim();
    Ok(ExportMeta {
        title: if title.is_empty() {
            DEFAULT_TITLE.into()
        } else {
            title.into()
        },
        ..meta.clone()
    })
}

/// 根拠の時刻を `00:12 / 01:02` の形にする。
fn clock_list(citations: &[u64]) -> String {
    citations
        .iter()
        .map(|&ms| time::clock(ms))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 議事録を `format` の形式で書き出す。`Ics` は、アクションアイテムが無いとエラー。
/// 議事録が保存時の上限（区分ごとの件数・1 項目の長さ）を超えているときもエラー。
pub fn export_minutes(
    minutes: &Minutes,
    meta: &ExportMeta,
    format: ExportFormat,
) -> Result<Exported, IngestError> {
    let meta = checked(meta)?;
    check_size(minutes)?;
    let (ext, mime_type, text) = match format {
        ExportFormat::Markdown => (
            "md",
            "text/markdown; charset=utf-8",
            markdown::render(minutes, &meta),
        ),
        ExportFormat::Csv => ("csv", "text/csv; charset=utf-8", csv::render(minutes)),
        ExportFormat::Ics => (
            "ics",
            "text/calendar; charset=utf-8",
            ics::render(minutes, &meta)?,
        ),
    };
    Ok(Exported {
        file_name: format!("minutes-{}.{ext}", meta.recording_id),
        mime_type,
        text,
    })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_formats;
#[cfg(test)]
mod tests_hardening;
