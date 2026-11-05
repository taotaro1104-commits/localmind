//! 議事録の iCalendar（RFC 5545）書き出し。アクションアイテムを ToDo（VTODO）にする。
//!
//! - 改行は CRLF。1 行は 75 バイトまでで、超える行は「CRLF + 空白」で折り返す（UTF-8 の文字の途中では切らない）。
//! - 値は `\` `;` `,` と改行を逃がし、制御文字・不可視の文字は取り除く（本文に改行を混ぜて、別の行や部品を作らせない）。
//! - 期限は、日付として読めるものだけを `DUE` にする。「来週金曜」のような相対表現は、推測せずに説明欄へ書く。
//! - UID は、録音 id と項目の内容（本文・担当者・期限）から決める。書き出し直しても同じ ToDo は同じ UID になり、
//!   項目の並びが変わっても別の ToDo を上書きしない（完了の切り替えでは変わらない）。
//! - 完了した ToDo には `COMPLETED` を付ける。実際の完了時刻は記録していないので、書き出した時刻で代用する。

use std::collections::HashMap;

use super::time::{ics_date, ics_utc, local, parse_due};
use super::{clock_list, sanitize, ExportMeta};
use crate::error::IngestError;
use crate::minutes::{ActionItem, Minutes};

/// 1 行の最大バイト数（折り返しの前）。
const FOLD_AT: usize = 75;

/// 値の逃がし（TEXT 型）。改行（CRLF も 1 つの改行）は `\n` にする。
pub(super) fn escape(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        match c {
            '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}' => out.push_str("\\n"),
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            c => out.push_str(&sanitize(c.encode_utf8(&mut [0; 4]))),
        }
    }
    out
}

/// 1 行を 75 バイトで折り返す（続きの行は先頭に空白 1 つ。UTF-8 の文字の途中では切らない）。
pub(super) fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut used = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if used + n > FOLD_AT {
            out.push_str("\r\n ");
            used = 1; // 先頭の空白の分
        }
        out.push(c);
        used += n;
    }
    out
}

fn property(out: &mut String, name: &str, value: &str) {
    out.push_str(&fold(&format!("{name}:{value}")));
    out.push_str("\r\n");
}

/// 内容の指紋（FNV-1a 64bit）。UID を作るためのもので、暗号の用途ではない。
fn fingerprint(a: &ActionItem) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [Some(&a.text), a.assignee.as_ref(), a.due.as_ref()] {
        let bytes = part.map_or(&[][..], |s| s.as_bytes());
        for &byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

/// 各項目の UID。内容が同じ項目が複数あるときだけ、2 つ目以降に連番を付けて区別する。
fn uids(recording_id: &str, items: &[ActionItem]) -> Vec<String> {
    let mut seen: HashMap<u64, usize> = HashMap::new();
    items
        .iter()
        .map(|a| {
            let h = fingerprint(a);
            let n = seen.entry(h).or_insert(0);
            *n += 1;
            let suffix = if *n > 1 {
                format!("-{n}")
            } else {
                String::new()
            };
            format!("{recording_id}-{h:016x}{suffix}@localmind.invalid")
        })
        .collect()
}

/// アクションアイテムを VTODO にした iCalendar を返す。アクションアイテムが無ければエラー
/// （中身の無い VCALENDAR は規格に合わない）。
pub(super) fn render(minutes: &Minutes, meta: &ExportMeta) -> Result<String, IngestError> {
    if minutes.action_items.is_empty() {
        return Err(IngestError::InvalidFormat(
            "アクションアイテムが無いため、カレンダーに書き出せません".into(),
        ));
    }
    let recorded = local(meta.started_at_ms, meta.utc_offset_minutes).date;
    let stamp = ics_utc(meta.exported_at_ms);
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//LocalMind//Minutes//JA\r\nCALSCALE:GREGORIAN\r\n",
    );
    let uids = uids(&meta.recording_id, &minutes.action_items);
    for (a, uid) in minutes.action_items.iter().zip(&uids) {
        out.push_str("BEGIN:VTODO\r\n");
        property(&mut out, "UID", uid);
        property(&mut out, "DTSTAMP", &stamp);
        property(&mut out, "SUMMARY", &escape(&a.text));
        let mut notes = vec![format!("録音: {}", meta.title)];
        if let Some(who) = &a.assignee {
            notes.push(format!("担当: {who}"));
        }
        if let Some(due) = &a.due {
            notes.push(format!("期限: {due}"));
        }
        notes.push(format!(
            "根拠（録音の開始からの時刻）: {}",
            clock_list(&a.citations)
        ));
        property(&mut out, "DESCRIPTION", &escape(&notes.join("\n")));
        if let Some(date) = a.due.as_deref().and_then(|d| parse_due(d, recorded)) {
            property(&mut out, "DUE;VALUE=DATE", &ics_date(date));
        }
        if a.done {
            property(&mut out, "STATUS", "COMPLETED");
            property(&mut out, "PERCENT-COMPLETE", "100");
            property(&mut out, "COMPLETED", &stamp);
        } else {
            property(&mut out, "STATUS", "NEEDS-ACTION");
        }
        out.push_str("END:VTODO\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    Ok(out)
}
