use super::time::{
    clock, days_from_civil, format_local, format_offset, ics_utc, local, parse_due, Date,
};
use super::*;
use crate::minutes::{ActionItem, MinutesItem};

pub(super) const START: u64 = 1_793_886_907_027; // 2026-11-05 13:55:07 UTC
pub(super) const EXPORTED: u64 = 1_793_890_000_000;

pub(super) fn meta() -> ExportMeta {
    ExportMeta {
        recording_id: "rec-20261105-135507".into(),
        title: "定例会議".into(),
        started_at_ms: START,
        exported_at_ms: EXPORTED,
        utc_offset_minutes: 540,
    }
}

pub(super) fn item(text: &str, citations: &[u64]) -> MinutesItem {
    MinutesItem {
        text: text.into(),
        citations: citations.to_vec(),
    }
}

pub(super) fn action(text: &str, who: Option<&str>, due: Option<&str>, done: bool) -> ActionItem {
    ActionItem {
        text: text.into(),
        assignee: who.map(Into::into),
        due: due.map(Into::into),
        citations: vec![12_000, 62_000],
        done,
    }
}

pub(super) fn sample() -> Minutes {
    Minutes {
        summary: vec![item("予算の見直しを議論した", &[12_000])],
        decisions: vec![item("来期は増員する", &[62_000, 3_725_000])],
        action_items: vec![
            action("見積書を送る", Some("田中"), Some("2026-11-12"), false),
            action("資料を直す", None, Some("来週金曜"), true),
        ],
        open_issues: vec![item("予算の上限は未定", &[12_000])],
        rejected: 0,
    }
}

// ---- 暦と時刻

#[test]
fn utc_milliseconds_become_local_time_with_the_given_offset() {
    assert_eq!(format_local(START, 0), "2026-11-05 13:55");
    assert_eq!(format_local(START, 540), "2026-11-05 22:55");
    assert_eq!(format_local(START, -300), "2026-11-05 08:55");
    assert_eq!(format_local(1_800_000_000_000, 0), "2027-01-15 08:00");
    assert_eq!(ics_utc(1_800_000_000_000), "20270115T080000Z");
    assert_eq!(format_offset(540), "+09:00");
    assert_eq!(format_offset(-330), "-05:30");
    assert_eq!(format_offset(0), "+00:00");
}

#[test]
fn the_local_date_follows_the_offset_across_midnight_and_new_year() {
    let end_of_2026 = days_from_civil(Date {
        year: 2026,
        month: 12,
        day: 31,
    }) as u64
        * 86_400_000
        + (23 * 60 + 30) * 60_000;
    assert_eq!(format_local(end_of_2026, 0), "2026-12-31 23:30");
    assert_eq!(format_local(end_of_2026, 540), "2027-01-01 08:30");
    assert_eq!(format_local(end_of_2026, -600), "2026-12-31 13:30");
}

#[test]
fn calendar_conversion_round_trips_including_leap_days() {
    for (y, m, d) in [
        (2024, 2, 29),
        (2026, 11, 5),
        (2028, 2, 29),
        (2100, 3, 1),
        (2100, 2, 28),
        (2199, 12, 31),
    ] {
        let date = Date {
            year: y,
            month: m,
            day: d,
        };
        let ms = days_from_civil(date) as u64 * 86_400_000;
        assert_eq!(local(ms, 0).date, date, "{y}-{m}-{d}");
        assert_eq!(local(ms + 86_399_000, 0).date, date, "{y}-{m}-{d} の終わり");
    }
}

#[test]
fn elapsed_time_is_shown_as_minutes_and_seconds_or_with_hours() {
    assert_eq!(clock(0), "00:00");
    assert_eq!(clock(62_000), "01:02");
    assert_eq!(clock(59_999), "00:59");
    assert_eq!(clock(3_725_000), "1:02:05");
}

#[test]
fn only_dates_that_can_be_read_become_due_dates() {
    let rec = Date {
        year: 2026,
        month: 11,
        day: 5,
    };
    let d = |y, m, day| {
        Some(Date {
            year: y,
            month: m,
            day,
        })
    };
    assert_eq!(parse_due("2026-11-12", rec), d(2026, 11, 12));
    assert_eq!(parse_due("2026/11/12", rec), d(2026, 11, 12));
    assert_eq!(parse_due("2026年11月12日", rec), d(2026, 11, 12));
    assert_eq!(
        parse_due("１１月１２日", rec),
        d(2026, 11, 12),
        "全角の数字も読む"
    );
    assert_eq!(
        parse_due("11月5日", rec),
        d(2026, 11, 5),
        "録音と同じ日は同じ年"
    );
    assert_eq!(
        parse_due("11月3日", rec),
        d(2027, 11, 3),
        "録音より前なら翌年"
    );
    assert_eq!(parse_due("2028-02-29", rec), d(2028, 2, 29));
    for unreadable in [
        "来週金曜",
        "",
        "2027-02-29",
        "2026-13-01",
        "2026-11-31",
        "11月31日",
        "あした",
        "2026-11",
        "2026-11-12-01",
    ] {
        assert_eq!(parse_due(unreadable, rec), None, "{unreadable:?}");
    }
}

// ---- Markdown

#[test]
fn markdown_has_the_title_sections_checkboxes_and_evidence_times() {
    let md = export_minutes(&sample(), &meta(), ExportFormat::Markdown)
        .unwrap()
        .text;
    assert!(
        md.starts_with("# 定例会議\n\n録音: 2026-11-05 22:55（UTC+09:00）\n"),
        "{md}"
    );
    for heading in [
        "## 要約",
        "## 決定事項",
        "## アクションアイテム",
        "## 未解決の論点",
    ] {
        assert!(md.contains(&format!("\n{heading}\n")), "{heading}");
    }
    assert!(md.contains("- 予算の見直しを議論した  `00:12`\n"), "{md}");
    assert!(md.contains("- 来期は増員する  `01:02` `1:02:05`\n"), "{md}");
    assert!(
        md.contains("- [ ] 見積書を送る（担当: 田中 ・ 期限: 2026-11-12）  `00:12` `01:02`\n"),
        "{md}"
    );
    assert!(md.contains("- [x] 資料を直す（期限: 来週金曜）"), "{md}");
}

#[test]
fn empty_sections_say_so_and_rejected_items_are_reported() {
    let md = export_minutes(
        &Minutes {
            rejected: 3,
            ..Minutes::default()
        },
        &meta(),
        ExportFormat::Markdown,
    )
    .unwrap()
    .text;
    assert_eq!(md.matches("- （なし）").count(), 4);
    assert!(md.contains("載せなかった項目: 3 件"));
}

#[test]
fn hostile_text_cannot_turn_into_markdown_structure() {
    let mut m = sample();
    m.summary = vec![item("# 偽の見出し\n[リンク](javascript:alert(1)) <script>x</script> | a | b ![img](http://e/x.png) `code` *強調*", &[0])];
    m.action_items[0].assignee = Some("[x](http://evil)".into());
    let mut meta = meta();
    meta.title = "# タイトル <b>".into();
    let md = export_minutes(&m, &meta, ExportFormat::Markdown)
        .unwrap()
        .text;
    assert_eq!(
        md.lines().filter(|l| l.starts_with('#')).count(),
        5,
        "見出しは題名と 4 つの区分だけ:\n{md}"
    );
    // 構造を作れる記号は、すべて逃がされている（自分が付ける `...` は根拠の時刻だけ）。
    // チェックボックス（自分が付ける `- [ ] ` と `- [x] `）は除いて数える。
    let md = md.replace("- [ ] ", "- ").replace("- [x] ", "- ");
    for c in ['[', ']', '<', '>', '!', '|', '&', '*', '~', '#'] {
        let bare = md
            .char_indices()
            .filter(|&(i, ch)| ch == c && !md[..i].ends_with('\\'))
            // 見出し（行頭の `#`）と、根拠の時刻を囲む `\`` は、こちらが付けたもの。
            .filter(|&(i, _)| {
                let line_start = md[..i].rfind('\n').map_or(0, |p| p + 1);
                !(c == '#' && md[line_start..i].chars().all(|x| x == '#'))
            })
            .count();
        assert_eq!(bare, 0, "{c:?} が逃がされていない:\n{md}");
    }
    assert!(!md.contains("\n# 偽の見出し"));
    assert!(!md.contains('\r'), "{md:?}");
}

// ---- 共通

#[test]
fn file_name_and_mime_type_follow_the_format() {
    for (format, name, mime) in [
        (
            ExportFormat::Markdown,
            "minutes-rec-20261105-135507.md",
            "text/markdown; charset=utf-8",
        ),
        (
            ExportFormat::Csv,
            "minutes-rec-20261105-135507.csv",
            "text/csv; charset=utf-8",
        ),
        (
            ExportFormat::Ics,
            "minutes-rec-20261105-135507.ics",
            "text/calendar; charset=utf-8",
        ),
    ] {
        let e = export_minutes(&sample(), &meta(), format).unwrap();
        assert_eq!((e.file_name.as_str(), e.mime_type), (name, mime));
    }
}

#[test]
fn bad_metadata_is_refused() {
    let bad = |f: fn(&mut ExportMeta)| {
        let mut m = meta();
        f(&mut m);
        export_minutes(&sample(), &m, ExportFormat::Markdown)
    };
    assert!(bad(|m| m.recording_id = "../x".into()).is_err());
    assert!(bad(|m| m.recording_id = "Rec".into()).is_err());
    assert!(bad(|m| m.utc_offset_minutes = 15 * 60).is_err());
    assert!(bad(|m| m.utc_offset_minutes = -15 * 60).is_err());
    assert!(bad(|m| m.started_at_ms = 5).is_err());
    assert!(bad(|m| m.exported_at_ms = u64::MAX).is_err());
    assert!(bad(|m| m.utc_offset_minutes = 14 * 60).is_ok());
}

#[test]
fn the_title_is_cleaned_shortened_and_defaulted() {
    let md = |title: &str| {
        let mut m = meta();
        m.title = title.into();
        export_minutes(&sample(), &m, ExportFormat::Markdown)
            .unwrap()
            .text
    };
    assert!(md("").starts_with("# 議事録\n"));
    assert!(md("   \n ").starts_with("# 議事録\n"));
    assert_eq!(
        md("a\nb\tc").lines().next(),
        Some("# a b c"),
        "改行とタブは空白になる"
    );
    let long = md(&"あ".repeat(500));
    assert_eq!(
        long.lines().next().unwrap().chars().count(),
        "# ".len() + 100
    );
}

#[test]
fn a_saved_and_reloaded_minutes_exports_the_same_text() {
    use crate::store::FsChunkStore;
    let root = std::env::temp_dir().join(format!("lm-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = FsChunkStore::new(&root);
    store
        .begin_recording(
            "rec-20261105-135507",
            &crate::store::RecordingMeta {
                consent_confirmed: true,
                sample_rate: 16_000,
                started_at_ms: START,
            },
        )
        .unwrap();
    store
        .save_minutes("rec-20261105-135507", &sample())
        .unwrap();
    let loaded = store.load_minutes("rec-20261105-135507").unwrap().unwrap();
    for format in [ExportFormat::Markdown, ExportFormat::Csv, ExportFormat::Ics] {
        assert_eq!(
            export_minutes(&loaded, &meta(), format).unwrap(),
            export_minutes(&sample(), &meta(), format).unwrap()
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
