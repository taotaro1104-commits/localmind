//! 書き出しの境界・防御のテスト（レビューで指摘された抜けを固定する）。

use super::ics::fold;
use super::tests::{item, meta, sample};
use super::time::{parse_due, Date};
use super::*;
use crate::minutes::ActionItem;

fn export(m: &Minutes, f: ExportFormat) -> String {
    export_minutes(m, &meta(), f).unwrap().text
}

fn todo(text: &str) -> ActionItem {
    ActionItem {
        text: text.into(),
        assignee: None,
        due: None,
        citations: vec![0],
        done: false,
    }
}

// ---- 期限の読み取り

#[test]
fn due_dates_need_a_four_digit_year_in_range_and_no_signs() {
    let rec = Date {
        year: 2026,
        month: 11,
        day: 5,
    };
    for unreadable in [
        "26-11-12",
        "26/11/12",
        "26年11月12日",
        "12345-01-01",
        "99999999-01-01",
        "-5年1月1日",
        "+2026年+11月+12日",
        "2026-+1-12",
        "2023-12-31",
        "2201-01-01",
        "2026-11-12-01",
        "2026-11/12",
    ] {
        assert_eq!(parse_due(unreadable, rec), None, "{unreadable:?}");
    }
    let d = |y, m, day| {
        Some(Date {
            year: y,
            month: m,
            day,
        })
    };
    assert_eq!(
        parse_due("2026-1-2", rec),
        d(2026, 1, 2),
        "月と日は 1 桁でもよい"
    );
    assert_eq!(
        parse_due("  2026-11-12  ", rec),
        d(2026, 11, 12),
        "前後の空白は読み飛ばす"
    );
    assert_eq!(
        parse_due("２０２６ー１１ー１２", rec),
        d(2026, 11, 12),
        "全角の数字と長音の区切り"
    );
    assert_eq!(parse_due("2024-02-29", rec), d(2024, 2, 29));
    assert_eq!(
        parse_due("2100-02-29", rec),
        None,
        "2100 年はうるう年ではない"
    );
    assert_eq!(parse_due("2400-02-29", rec), None, "範囲外");
}

#[test]
fn a_due_date_without_a_year_rolls_into_next_year_after_the_recording_date() {
    let d = |y, m, day| {
        Some(Date {
            year: y,
            month: m,
            day,
        })
    };
    let dec31 = Date {
        year: 2026,
        month: 12,
        day: 31,
    };
    assert_eq!(parse_due("1月1日", dec31), d(2027, 1, 1));
    assert_eq!(parse_due("12月31日", dec31), d(2026, 12, 31));
    let leap_day = Date {
        year: 2028,
        month: 2,
        day: 29,
    };
    assert_eq!(parse_due("2月29日", leap_day), d(2028, 2, 29));
    assert_eq!(
        parse_due("2月28日", leap_day),
        d(2029, 2, 28),
        "録音日より前なので翌年"
    );
    let after_leap_day = Date {
        year: 2028,
        month: 3,
        day: 1,
    };
    assert_eq!(
        parse_due("2月29日", after_leap_day),
        None,
        "翌年（2029）に 2/29 は無いので読まない"
    );
}

// ---- iCalendar

#[test]
fn folding_keeps_every_line_within_75_bytes_and_never_splits_a_character() {
    assert_eq!(
        fold(&"a".repeat(75)),
        "a".repeat(75),
        "ちょうど 75 バイトは折り返さない"
    );
    assert_eq!(fold(&"a".repeat(76)), format!("{}\r\n a", "a".repeat(75)));
    // 74 バイトのあとの 3 バイト文字は、入りきらないので次の行へ送る。
    assert_eq!(
        fold(&format!("{}長", "a".repeat(74))),
        format!("{}\r\n 長", "a".repeat(74))
    );
    for sample in [
        "長".repeat(100),
        "😀".repeat(60),
        format!("{}{}", "a".repeat(73), "😀".repeat(30)),
    ] {
        let folded = fold(&sample);
        for (i, line) in folded.split("\r\n").enumerate() {
            assert!(line.len() <= 75, "{} バイト: {line}", line.len());
            assert!(i == 0 || line.starts_with(' '), "続きの行は空白で始まる");
        }
        assert_eq!(folded.replace("\r\n ", ""), sample, "戻すと元の文字列");
    }
}

#[test]
fn completed_todos_carry_a_completed_time_and_open_ones_do_not() {
    let ics = export(&sample(), ExportFormat::Ics);
    let lines: Vec<&str> = ics.split("\r\n").collect();
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("COMPLETED:")).count(),
        1
    );
    assert!(
        lines.contains(&"COMPLETED:20261105T144640Z"),
        "書き出した時刻で代用する"
    );
    let todo_block = |n: usize| -> Vec<&str> {
        let start = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| **l == "BEGIN:VTODO")
            .nth(n)
            .unwrap()
            .0;
        let end = lines[start..]
            .iter()
            .position(|l| *l == "END:VTODO")
            .unwrap()
            + start;
        lines[start..end].to_vec()
    };
    assert!(
        todo_block(0).contains(&"STATUS:NEEDS-ACTION")
            && !todo_block(0).iter().any(|l| l.starts_with("COMPLETED"))
    );
    assert!(
        todo_block(1).contains(&"STATUS:COMPLETED")
            && todo_block(1).contains(&"PERCENT-COMPLETE:100")
    );
}

fn uid_of(ics: &str, summary: &str) -> String {
    let unfolded = ics.replace("\r\n ", "");
    let lines: Vec<&str> = unfolded.split("\r\n").collect();
    let at = lines
        .iter()
        .position(|l| *l == format!("SUMMARY:{summary}"))
        .expect("SUMMARY");
    lines[..at]
        .iter()
        .rev()
        .find(|l| l.starts_with("UID:"))
        .unwrap()
        .to_string()
}

#[test]
fn uids_follow_the_content_not_the_position_or_the_done_flag() {
    let mut m = Minutes {
        action_items: vec![todo("見積書を送る"), todo("資料を直す")],
        ..Minutes::default()
    };
    let before = export(&m, ExportFormat::Ics);
    let (a, b) = (
        uid_of(&before, "見積書を送る"),
        uid_of(&before, "資料を直す"),
    );
    assert_ne!(a, b);
    m.action_items.reverse(); // 並びが変わっても
    m.action_items[0].done = true; // 完了にしても
    let after = export(&m, ExportFormat::Ics);
    assert_eq!(uid_of(&after, "見積書を送る"), a, "同じ ToDo は同じ UID");
    assert_eq!(uid_of(&after, "資料を直す"), b);
    m.action_items[0].text = "資料を直す（改）".into(); // 内容が変われば別の ToDo
    assert_ne!(
        uid_of(&export(&m, ExportFormat::Ics), "資料を直す（改）"),
        b
    );
}

#[test]
fn identical_todos_still_get_distinct_uids() {
    let m = Minutes {
        action_items: vec![todo("同じ"), todo("同じ"), todo("同じ")],
        ..Minutes::default()
    };
    let ics = export(&m, ExportFormat::Ics);
    let mut uids: Vec<&str> = ics
        .split("\r\n")
        .filter(|l| l.starts_with("UID:"))
        .collect();
    assert_eq!(uids.len(), 3);
    uids.sort_unstable();
    uids.dedup();
    assert_eq!(uids.len(), 3, "{uids:?}");
}

#[test]
fn a_crlf_in_text_becomes_one_escaped_newline() {
    let m = Minutes {
        action_items: vec![todo("一行目\r\n二行目\u{2028}三行目")],
        ..Minutes::default()
    };
    let ics = export(&m, ExportFormat::Ics);
    assert!(
        ics.contains("SUMMARY:一行目\\n二行目\\n三行目\r\n"),
        "{ics}"
    );
    assert!(!ics.contains('\u{2028}'));
}

// ---- CSV

#[test]
fn csv_guards_formulas_even_after_spaces_invisible_characters_and_in_full_width() {
    let cases = [
        "=1+1",
        " =1+1",
        "\t=1+1",
        "\u{200B}=1+1",
        "　＝1+1",
        "＋1",
        "－1",
        "＠SUM(A1)",
        "\r+1",
    ];
    for text in cases {
        let cell = super::csv::cell(text);
        assert!(cell.starts_with("\"'"), "{text:?} -> {cell}");
    }
    for text in ["普通の文章", "a=b", "100%", "x-y", "(カッコ)"] {
        assert!(!super::csv::cell(text).starts_with("\"'"), "{text:?}");
    }
}

// ---- 入力の検査

#[test]
fn oversized_minutes_are_refused_by_the_public_function() {
    let many = Minutes {
        summary: (0..201).map(|_| item("a", &[0])).collect(),
        ..Minutes::default()
    };
    assert!(export_minutes(&many, &meta(), ExportFormat::Markdown).is_err());
    let at_limit = Minutes {
        summary: (0..200).map(|_| item("a", &[0])).collect(),
        ..Minutes::default()
    };
    assert!(export_minutes(&at_limit, &meta(), ExportFormat::Markdown).is_ok());
    let long = Minutes {
        summary: vec![item(&"あ".repeat(1001), &[0])],
        ..Minutes::default()
    };
    assert!(export_minutes(&long, &meta(), ExportFormat::Csv).is_err());
    let long_todo = Minutes {
        action_items: vec![todo(&"あ".repeat(1001))],
        ..Minutes::default()
    };
    assert!(export_minutes(&long_todo, &meta(), ExportFormat::Ics).is_err());
}

#[test]
fn the_title_is_trimmed_after_cutting_and_the_time_errors_say_which_time() {
    let md = |title: &str| {
        let mut m = meta();
        m.title = title.into();
        export_minutes(&sample(), &m, ExportFormat::Markdown)
            .unwrap()
            .text
    };
    let cut = format!("{} {}", "あ".repeat(99), "い".repeat(10));
    assert_eq!(
        md(&cut).lines().next().unwrap(),
        format!("# {}", "あ".repeat(99)),
        "切ったあとの末尾の空白は落とす"
    );
    assert_eq!(
        md(&"あ".repeat(100))
            .lines()
            .next()
            .unwrap()
            .chars()
            .count(),
        2 + 100,
        "100 文字ちょうどはそのまま"
    );

    let msg = |f: fn(&mut ExportMeta)| {
        let mut m = meta();
        f(&mut m);
        export_minutes(&sample(), &m, ExportFormat::Csv)
            .unwrap_err()
            .to_string()
    };
    assert!(msg(|m| m.started_at_ms = 0).contains("録音の開始時刻"));
    assert!(msg(|m| m.exported_at_ms = 0).contains("書き出す時刻"));
    let mut earlier_export = meta();
    earlier_export.exported_at_ms = earlier_export.started_at_ms - 1;
    assert!(
        export_minutes(&sample(), &earlier_export, ExportFormat::Csv).is_ok(),
        "開始より前の書き出し時刻も許す"
    );
}

#[test]
fn markdown_drops_leading_spaces_so_an_item_cannot_become_a_code_block() {
    let mut m = sample();
    m.summary = vec![item("      インデントされた行", &[0])];
    assert!(export(&m, ExportFormat::Markdown).contains("- インデントされた行  `00:00`\n"));
}
