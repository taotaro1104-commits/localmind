//! CSV と iCalendar の書き出しのテスト。

use super::tests::{item, meta, sample};
use super::*;

// ---- CSV

/// RFC 4180 の簡易な読み取り（検証用）。
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let (mut rows, mut row, mut cell, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    rows
}

#[test]
fn csv_has_a_bom_crlf_and_one_row_per_item_with_six_columns() {
    let csv = export_minutes(&sample(), &meta(), ExportFormat::Csv)
        .unwrap()
        .text;
    assert!(csv.starts_with('\u{FEFF}'));
    assert!(
        !csv.replace("\r\n", "").contains(['\r', '\n']),
        "改行はすべて CRLF"
    );
    let rows = parse_csv(&csv);
    assert_eq!(rows.len(), 1 + 1 + 1 + 2 + 1);
    assert!(rows.iter().all(|r| r.len() == 6), "{rows:?}");
    assert_eq!(rows[0][0], "区分");
    assert_eq!(
        rows[3],
        [
            "アクションアイテム",
            "見積書を送る",
            "田中",
            "2026-11-12",
            "",
            "00:12 / 01:02"
        ]
    );
    assert_eq!(rows[4][4], "済");
}

#[test]
fn csv_cells_cannot_become_spreadsheet_formulas_or_break_the_row() {
    let mut m = sample();
    m.summary = vec![
        item("=HYPERLINK(\"http://evil\",\"x\")", &[0]),
        item("+1+1", &[0]),
        item("-2+3", &[0]),
        item("@SUM(A1)", &[0]),
        item("say \"hi\", then\nleave", &[0]),
    ];
    let rows = parse_csv(&export_minutes(&m, &meta(), ExportFormat::Csv).unwrap().text);
    let texts: Vec<&str> = rows[1..6].iter().map(|r| r[1].as_str()).collect();
    assert_eq!(texts[0], "'=HYPERLINK(\"http://evil\",\"x\")");
    assert_eq!(&texts[1..4], ["'+1+1", "'-2+3", "'@SUM(A1)"]);
    assert_eq!(
        texts[4], "say \"hi\", then leave",
        "引用符は保たれ、改行は空白になる"
    );
    assert!(rows.iter().all(|r| r.len() == 6));
}

// ---- iCalendar

/// 折り返しを戻した論理行。
fn unfold(ics: &str) -> Vec<String> {
    ics.replace("\r\n ", "")
        .split("\r\n")
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn ics_makes_one_todo_per_action_item_with_status_due_and_stable_uids() {
    let ics = export_minutes(&sample(), &meta(), ExportFormat::Ics)
        .unwrap()
        .text;
    let lines = unfold(&ics);
    assert_eq!(lines.first().map(String::as_str), Some("BEGIN:VCALENDAR"));
    assert_eq!(lines.last().map(String::as_str), Some("END:VCALENDAR"));
    assert!(lines.contains(&"VERSION:2.0".to_string()));
    assert_eq!(lines.iter().filter(|l| *l == "BEGIN:VTODO").count(), 2);
    let uids: Vec<&String> = lines.iter().filter(|l| l.starts_with("UID:")).collect();
    assert_eq!(uids.len(), 2);
    assert_ne!(uids[0], uids[1]);
    for uid in uids {
        assert!(
            uid.starts_with("UID:rec-20261105-135507-") && uid.ends_with("@localmind.invalid"),
            "{uid}"
        );
    }
    assert!(
        lines.contains(&"DTSTAMP:20261105T144640Z".to_string()),
        "{lines:?}"
    );
    assert!(lines.contains(&"SUMMARY:見積書を送る".to_string()));
    assert!(lines.contains(&"DUE;VALUE=DATE:20261112".to_string()));
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("DUE")).count(),
        1,
        "読めない期限は DUE にしない"
    );
    assert!(lines.contains(&"STATUS:NEEDS-ACTION".to_string()));
    assert!(lines.contains(&"STATUS:COMPLETED".to_string()));
    assert!(lines.contains(&"PERCENT-COMPLETE:100".to_string()));
    let desc: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("DESCRIPTION:"))
        .collect();
    assert!(
        desc[1].contains("期限: 来週金曜"),
        "相対表現は説明に残す: {desc:?}"
    );
}

#[test]
fn ics_uses_crlf_and_folds_long_lines_without_splitting_characters() {
    let mut m = sample();
    m.action_items[0].text = "長い".repeat(120);
    let ics = export_minutes(&m, &meta(), ExportFormat::Ics).unwrap().text;
    assert!(
        !ics.replace("\r\n", "").contains(['\r', '\n']),
        "改行はすべて CRLF"
    );
    for line in ics.split("\r\n") {
        assert!(line.len() <= 75, "{} バイト: {line}", line.len());
    }
    assert!(
        unfold(&ics).contains(&format!("SUMMARY:{}", "長い".repeat(120))),
        "折り返しを戻すと元の文字列"
    );
}

#[test]
fn ics_text_is_escaped_and_cannot_inject_lines_or_components() {
    let mut m = sample();
    m.action_items[0].text = "a,b;c\\d\r\nEND:VTODO\r\nBEGIN:VEVENT\r\nSUMMARY:evil".into();
    m.action_items[0].assignee = Some("x\nATTENDEE:mailto:a@b".into());
    let mut meta = meta();
    meta.title = "t\r\nBEGIN:VEVENT".into();
    let ics = export_minutes(&m, &meta, ExportFormat::Ics).unwrap().text;
    let lines = unfold(&ics);
    assert!(
        !lines
            .iter()
            .any(|l| l.starts_with("BEGIN:VEVENT") || l.starts_with("ATTENDEE")),
        "{lines:?}"
    );
    assert_eq!(lines.iter().filter(|l| *l == "END:VTODO").count(), 2);
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("SUMMARY:a\\,b\\;c\\\\d")),
        "{lines:?}"
    );
}

#[test]
fn ics_without_action_items_is_refused() {
    let m = Minutes {
        action_items: vec![],
        ..sample()
    };
    assert!(matches!(
        export_minutes(&m, &meta(), ExportFormat::Ics),
        Err(IngestError::InvalidFormat(_))
    ));
    assert!(
        export_minutes(&m, &meta(), ExportFormat::Markdown).is_ok(),
        "ほかの形式は書き出せる"
    );
}
