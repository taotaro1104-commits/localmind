use super::*;
use crate::transcribe::Stage;

pub(super) fn seg(chunk: u32, start: u64, end: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        chunk_index: chunk,
        start_ms: start,
        end_ms: end,
        speaker: None,
        text: text.into(),
        confidence: 0.9,
        stage: Stage::Draft,
    }
}

pub(super) fn texts(t: &Transcript) -> Vec<&str> {
    t.segments().iter().map(|s| s.text.as_str()).collect()
}

pub(super) fn sample() -> Transcript {
    let mut t = Transcript::new();
    t.set_chunk(
        1,
        vec![
            seg(1, 31_000, 33_000, "次の議題です"),
            seg(1, 35_000, 36_000, "了解"),
        ],
    )
    .unwrap();
    t.set_chunk(
        0,
        vec![
            seg(0, 1_000, 3_000, "始めます"),
            seg(0, 5_000, 8_000, "資料を見てください"),
        ],
    )
    .unwrap();
    t
}

#[test]
fn segments_are_kept_in_time_order_regardless_of_registration_order() {
    let t = sample();
    assert_eq!(
        texts(&t),
        ["始めます", "資料を見てください", "次の議題です", "了解"]
    );
    assert_eq!(t.duration_ms(), 36_000);
    assert!(!t.is_empty());
    assert!(Transcript::new().is_empty());
    assert_eq!(Transcript::new().duration_ms(), 0);
}

#[test]
fn registering_a_chunk_again_replaces_it_without_duplicates() {
    let mut t = sample();
    t.set_chunk(0, vec![seg(0, 1_000, 2_000, "やり直し")])
        .unwrap();
    assert_eq!(texts(&t), ["やり直し", "次の議題です", "了解"]);
    t.set_chunk(0, vec![]).unwrap();
    assert_eq!(texts(&t), ["次の議題です", "了解"]);
}

#[test]
fn segments_of_another_chunk_or_with_bad_times_are_rejected() {
    let mut t = sample();
    let before = t.clone();
    assert!(t.set_chunk(0, vec![seg(9, 1, 2, "別チャンク")]).is_err());
    assert!(t.set_chunk(0, vec![seg(0, 5, 5, "長さ 0")]).is_err());
    assert!(t.set_chunk(0, vec![seg(0, 9, 5, "逆転")]).is_err());
    assert!(t.apply_refined(0, vec![seg(2, 1, 2, "x")]).is_err());
    assert_eq!(t, before, "拒否したら状態は変わらない");
}

#[test]
fn refinement_reports_only_the_differences() {
    let mut t = sample();
    let refined = vec![
        seg(0, 1_000, 3_000, "始めます"),           // 同じ
        seg(0, 5_100, 8_000, "資料をご覧ください"), // 文面が変わった
        seg(0, 9_000, 9_500, "以上"),               // 新規
    ];
    let diff = t.apply_refined(0, refined).unwrap();
    assert_eq!(
        diff,
        RefineDiff {
            changed: 1,
            added: 1,
            removed: 0,
            unchanged: 1
        }
    );
    assert!(diff.has_changes());
    assert_eq!(
        texts(&t),
        [
            "始めます",
            "資料をご覧ください",
            "以上",
            "次の議題です",
            "了解"
        ]
    );
}

#[test]
fn refinement_that_drops_a_hallucinated_segment_counts_it_as_removed() {
    let mut t = sample();
    let diff = t
        .apply_refined(0, vec![seg(0, 1_000, 3_000, "始めます")])
        .unwrap();
    assert_eq!(diff.removed, 1);
    assert_eq!(diff.unchanged, 1);
    assert_eq!(texts(&t), ["始めます", "次の議題です", "了解"]);
}

#[test]
fn an_identical_refinement_has_no_changes() {
    let mut t = sample();
    let same: Vec<_> = t
        .segments()
        .iter()
        .filter(|s| s.chunk_index == 0)
        .cloned()
        .collect();
    let diff = t.apply_refined(0, same).unwrap();
    assert!(!diff.has_changes());
    assert_eq!(diff.unchanged, 2);
}

#[test]
fn refinement_keeps_the_speaker_names_assigned_to_the_draft() {
    let mut t = sample();
    assert!(t.set_speaker(0, Some("話者A".into())));
    assert!(t.set_speaker(1, Some("話者B".into())));
    t.apply_refined(
        0,
        vec![
            seg(0, 1_100, 3_000, "始めます"),
            seg(0, 5_000, 8_200, "資料をご覧ください"),
        ],
    )
    .unwrap();
    let speakers: Vec<_> = t
        .segments()
        .iter()
        .take(2)
        .map(|s| s.speaker.clone())
        .collect();
    assert_eq!(speakers, [Some("話者A".into()), Some("話者B".into())]);
}

#[test]
fn a_speaker_set_on_the_refined_result_wins_over_the_draft() {
    let mut t = sample();
    t.set_speaker(0, Some("話者A".into()));
    let mut refined = seg(0, 1_000, 3_000, "始めます");
    refined.speaker = Some("司会".into());
    t.apply_refined(0, vec![refined]).unwrap();
    assert_eq!(t.segments()[0].speaker.as_deref(), Some("司会"));
}

#[test]
fn the_best_overlapping_draft_segment_is_matched() {
    let mut t = Transcript::new();
    let mut a = seg(0, 0, 1_000, "短い");
    a.speaker = Some("A".into());
    let mut b = seg(0, 1_500, 6_000, "長い");
    b.speaker = Some("B".into());
    t.set_chunk(0, vec![a, b]).unwrap();
    // 0.9〜5 秒の 1 本の発話に統合された: 重なりが大きい「長い」(B) の話者を引き継ぐ。
    let diff = t
        .apply_refined(0, vec![seg(0, 900, 5_000, "統合された発話")])
        .unwrap();
    assert_eq!(t.segments()[0].speaker.as_deref(), Some("B"));
    assert_eq!(diff.changed, 1);
    assert_eq!(diff.removed, 1); // 「短い」は対応先が無くなった
}

#[test]
fn speakers_can_be_renamed_in_bulk() {
    let mut t = sample();
    t.set_speaker(0, Some("話者A".into()));
    t.set_speaker(2, Some("話者A".into()));
    t.set_speaker(1, Some("話者B".into()));
    assert_eq!(t.rename_speaker("話者A", "田中"), 2);
    assert_eq!(t.rename_speaker("話者A", "誰か"), 0);
    let names: Vec<_> = t.segments().iter().map(|s| s.speaker.as_deref()).collect();
    assert_eq!(names, [Some("田中"), Some("話者B"), Some("田中"), None]);
    assert!(!t.set_speaker(99, None));
}

#[test]
fn segment_lookup_by_time_for_tap_to_play() {
    let t = sample();
    assert_eq!(t.segment_at(1_000).unwrap().text, "始めます"); // 開始は含む
    assert_eq!(t.segment_at(2_999).unwrap().text, "始めます");
    assert!(t.segment_at(3_000).is_none()); // 終了は含まない
    assert!(t.segment_at(0).is_none());
    assert_eq!(t.segment_at(35_500).unwrap().text, "了解");
    assert!(t.segment_at(u64::MAX).is_none());
    assert!(Transcript::new().segment_at(5).is_none());
}

#[test]
fn text_output_has_timestamps_and_optional_speakers() {
    let mut t = sample();
    t.set_speaker(0, Some("田中".into()));
    assert_eq!(
        t.to_text(),
        "[00:00:01] 田中: 始めます\n[00:00:05] 資料を見てください\n[00:00:31] 次の議題です\n[00:00:35] 了解\n"
    );
    assert_eq!(Transcript::new().to_text(), "");
}

#[test]
fn index_documents_carry_references_that_map_back_to_times() {
    let t = sample();
    let docs = t.index_documents("meeting-1").unwrap();
    assert_eq!(docs.len(), 4);
    assert_eq!(
        docs[0],
        ("meeting-1@1000".to_string(), "始めます".to_string())
    );
    let (id, ms) = parse_segment_ref(&docs[2].0).unwrap();
    assert_eq!((id.as_str(), ms), ("meeting-1", 31_000));
    assert_eq!(t.segment_at(ms).unwrap().text, "次の議題です");
}

#[test]
fn index_documents_reject_unsafe_recording_ids() {
    assert!(sample().index_documents("../x").is_err());
    assert!(sample().index_documents("a@b").is_err());
    assert!(sample().index_documents("").is_err());
}

#[test]
fn segment_refs_parse_strictly() {
    assert_eq!(parse_segment_ref("rec@0"), Some(("rec".into(), 0)));
    assert_eq!(
        parse_segment_ref("a_b-1@123456"),
        Some(("a_b-1".into(), 123_456))
    );
    for bad in [
        "", "rec", "@5", "rec@", "rec@-1", "rec@1.5", "rec@x", "../x@5", "a@b@5x", "rec@+5",
        "rec@007", "rec@05", "Rec@5",
    ] {
        assert_eq!(parse_segment_ref(bad), None, "{bad:?}");
    }
}
