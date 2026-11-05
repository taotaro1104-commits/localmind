use super::*;
use crate::transcribe::{Stage, TranscriptSegment};

fn seg(start: u64, text: &str, speaker: Option<&str>) -> TranscriptSegment {
    TranscriptSegment {
        chunk_index: 0,
        start_ms: start,
        end_ms: start + 1000,
        speaker: speaker.map(Into::into),
        text: text.into(),
        confidence: 0.9,
        stage: Stage::Draft,
    }
}

fn transcript() -> Transcript {
    let mut t = Transcript::new();
    t.set_chunk(
        0,
        vec![
            seg(0, "予算案を承認します", Some("田中")),
            seg(2000, "来週までに見積もりを出します", Some("鈴木")),
            seg(4000, "納期はまだ決まっていません", None),
        ],
    )
    .unwrap();
    t
}

fn item(text: &str, cites: &[u64]) -> RawItem {
    RawItem {
        text: text.into(),
        cited_start_ms: cites.to_vec(),
        ..RawItem::default()
    }
}

struct Fixed(RawMinutes);
impl MinutesExtractor for Fixed {
    fn extract(&self, _: &str) -> Result<RawMinutes, IngestError> {
        Ok(self.0.clone())
    }
    fn id(&self) -> String {
        "fixed".into()
    }
}

struct Panics;
impl MinutesExtractor for Panics {
    fn extract(&self, _: &str) -> Result<RawMinutes, IngestError> {
        panic!("呼んではいけない")
    }
    fn id(&self) -> String {
        "panics".into()
    }
}

#[test]
fn llm_input_has_one_line_per_segment_with_start_positions() {
    let text = render_for_llm(&transcript());
    assert_eq!(
        text,
        "[0] 田中: 予算案を承認します\n[2000] 鈴木: 来週までに見積もりを出します\n[4000] 納期はまだ決まっていません\n"
    );
}

#[test]
fn items_citing_real_segments_are_accepted() {
    let raw = RawMinutes {
        decisions: vec![item("予算案を承認", &[0])],
        action_items: vec![RawItem {
            text: "見積もりを出す".into(),
            assignee: Some("鈴木".into()),
            due: Some("来週".into()),
            cited_start_ms: vec![2000, 0, 2000],
        }],
        open_issues: vec![item("納期が未定", &[4000])],
        ..RawMinutes::default()
    };
    let m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.decisions[0].citations, [0]);
    assert_eq!(m.action_items[0].citations, [0, 2000], "昇順・重複なし");
    assert_eq!(m.action_items[0].assignee.as_deref(), Some("鈴木"));
    assert!(!m.action_items[0].done);
    assert_eq!(m.open_issues[0].text, "納期が未定");
    assert_eq!(m.rejected, 0);
}

#[test]
fn items_without_real_evidence_are_rejected() {
    let raw = RawMinutes {
        decisions: vec![
            item("根拠なし", &[]),
            item("存在しない時刻", &[999]),
            item("一部だけ実在", &[1, 2000]),
        ],
        ..RawMinutes::default()
    };
    let m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.rejected, 2);
    assert_eq!(m.decisions.len(), 1);
    assert_eq!(m.decisions[0].citations, [2000], "実在する引用だけが残る");
}

#[test]
fn text_is_sanitized_and_empty_or_oversized_text_is_rejected() {
    let raw = RawMinutes {
        summary: vec![
            item("承認\u{202e}済み\x1b[2J\n次行", &[0]),
            item(" \u{200b} ", &[0]),
            item(&"あ".repeat(MAX_ITEM_CHARS + 1), &[0]),
        ],
        ..RawMinutes::default()
    };
    let m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.summary.len(), 1);
    assert_eq!(m.summary[0].text, "承認済み [2J 次行");
    assert_eq!(m.rejected, 2);
}

#[test]
fn assignee_and_due_are_cleaned_and_blank_ones_dropped() {
    let raw = RawMinutes {
        action_items: vec![RawItem {
            text: "対応".into(),
            assignee: Some("  \u{200b} ".into()),
            due: Some("金曜\x1b".repeat(100)),
            cited_start_ms: vec![0],
        }],
        ..RawMinutes::default()
    };
    let m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.action_items[0].assignee, None);
    let due = m.action_items[0].due.as_deref().unwrap();
    assert!(due.chars().count() <= crate::transcript::MAX_SPEAKER_CHARS);
    assert!(!due.chars().any(char::is_control));
}

#[test]
fn sections_are_capped() {
    let many = (0..MAX_ITEMS_PER_SECTION + 5)
        .map(|i| item(&format!("項目{i}"), &[0]))
        .collect();
    let raw = RawMinutes {
        decisions: many,
        ..RawMinutes::default()
    };
    let m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.decisions.len(), MAX_ITEMS_PER_SECTION);
    assert_eq!(m.rejected, 5);
}

#[test]
fn citations_are_capped_per_item() {
    let mut t = Transcript::new();
    let segs = (0..20).map(|i| seg(i * 2000, "x", None)).collect();
    t.set_chunk(0, segs).unwrap();
    let cites: Vec<u64> = (0..20).map(|i| i * 2000).collect();
    let raw = RawMinutes {
        summary: vec![item("多くの根拠", &cites)],
        ..RawMinutes::default()
    };
    let m = extract_minutes(&t, &Fixed(raw), 10_000).unwrap();
    assert_eq!(m.summary[0].citations.len(), MAX_CITATIONS);
}

#[test]
fn an_empty_transcript_never_calls_the_llm() {
    let m = extract_minutes(&Transcript::new(), &Panics, 10).unwrap();
    assert!(m.is_empty());
}

#[test]
fn too_long_input_is_an_error_not_a_truncation() {
    let err = extract_minutes(&transcript(), &Panics, 10).unwrap_err();
    assert!(matches!(err, IngestError::Extract(_)), "{err:?}");
}

#[test]
fn llm_failure_is_returned() {
    struct Failing;
    impl MinutesExtractor for Failing {
        fn extract(&self, _: &str) -> Result<RawMinutes, IngestError> {
            Err(IngestError::Extract("down".into()))
        }
        fn id(&self) -> String {
            "failing".into()
        }
    }
    assert!(extract_minutes(&transcript(), &Failing, 10_000).is_err());
}

#[test]
fn action_items_can_be_marked_done_and_exported_as_text() {
    let raw = RawMinutes {
        decisions: vec![item("予算案を承認", &[0])],
        action_items: vec![RawItem {
            text: "見積もりを出す".into(),
            assignee: Some("鈴木".into()),
            due: Some("来週".into()),
            cited_start_ms: vec![2000],
        }],
        ..RawMinutes::default()
    };
    let mut m = extract_minutes(&transcript(), &Fixed(raw), 10_000).unwrap();
    assert!(m.set_done(0, true));
    assert!(!m.set_done(5, true));
    assert_eq!(
        m.to_text(),
        "## 決定事項\n- 予算案を承認\n\n## ToDo\n- [x] 見積もりを出す (鈴木) 期限: 来週\n\n"
    );
}
