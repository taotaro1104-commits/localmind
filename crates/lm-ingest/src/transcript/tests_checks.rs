//! 重なりの検証・差分の数え方・出力の安全性・性能のテスト。

use super::tests::{sample, seg, texts};
use super::*;

// ---- 重なりの検証と、拒否時に状態が変わらないこと -----------------------------------------

#[test]
fn overlapping_segments_inside_one_chunk_are_rejected_without_touching_the_state() {
    let mut t = sample();
    let before = t.clone();
    // A[0,100) と B[10,20) は重なる（segment_at の二分探索の前提が崩れる）。
    let err = t.set_chunk(
        5,
        vec![seg(5, 100_000, 100_100, "A"), seg(5, 100_010, 100_020, "B")],
    );
    assert!(matches!(err, Err(IngestError::InvalidFormat(_))));
    assert_eq!(t, before);
}

#[test]
fn segments_with_the_same_start_are_rejected_so_references_stay_unique() {
    let mut t = Transcript::new();
    let err = t.set_chunk(0, vec![seg(0, 5, 10, "a"), seg(0, 5, 20, "b")]);
    assert!(matches!(err, Err(IngestError::InvalidFormat(_))));
    assert!(t.is_empty());
}

#[test]
fn a_chunk_overlapping_another_chunks_time_is_rejected_and_the_old_result_is_kept() {
    let mut t = sample();
    let before = t.clone();
    // チャンク 1（31〜36 秒）の既存セグメントと時間が重なるチャンク 0 の結果。
    let err = t.set_chunk(0, vec![seg(0, 30_000, 32_000, "はみ出し")]);
    assert!(matches!(err, Err(IngestError::InvalidFormat(_))));
    assert_eq!(t, before, "元のチャンク 0 の結果が残る");

    let err = t.apply_refined(0, vec![seg(0, 34_000, 35_500, "はみ出し")]);
    assert!(matches!(err, Err(IngestError::InvalidFormat(_))));
    assert_eq!(t, before);
}

#[test]
fn adjacent_segments_that_only_touch_are_allowed() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 10, "a"), seg(0, 10, 20, "b")])
        .unwrap();
    t.set_chunk(1, vec![seg(1, 20, 30, "c")]).unwrap();
    assert_eq!(texts(&t), ["a", "b", "c"]);
    assert_eq!(t.segment_at(10).unwrap().text, "b");
}

// ---- 差分の数え方（分割・重複）-------------------------------------------------------------

#[test]
fn duplicating_a_segment_is_reported_as_an_addition() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 100, "AB")]).unwrap();
    let diff = t
        .apply_refined(0, vec![seg(0, 0, 50, "AB"), seg(0, 50, 100, "AB")])
        .unwrap();
    assert_eq!(
        diff,
        RefineDiff {
            changed: 0,
            added: 1,
            removed: 0,
            unchanged: 1
        }
    );
    assert!(
        diff.has_changes(),
        "セグメントが増えたのに変化なしと報告された"
    );
}

#[test]
fn splitting_a_segment_counts_the_first_part_as_changed_and_the_rest_as_added() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 100, "AB")]).unwrap();
    let diff = t
        .apply_refined(0, vec![seg(0, 0, 50, "A"), seg(0, 50, 100, "B")])
        .unwrap();
    assert_eq!((diff.changed, diff.added, diff.removed), (1, 1, 0));
    assert_eq!(texts(&t), ["A", "B"]);
}

#[test]
fn a_speaker_is_carried_over_to_every_part_of_a_split_segment() {
    let mut t = Transcript::new();
    let mut old = seg(0, 0, 100, "AB");
    old.speaker = Some("田中".into());
    t.set_chunk(0, vec![old]).unwrap();
    t.apply_refined(0, vec![seg(0, 0, 50, "A"), seg(0, 50, 100, "B")])
        .unwrap();
    assert!(t
        .segments()
        .iter()
        .all(|s| s.speaker.as_deref() == Some("田中")));
}

// ---- 出力の安全性 ---------------------------------------------------------------------------

#[test]
fn newlines_in_text_or_speaker_cannot_forge_extra_lines() {
    let mut t = Transcript::new();
    let mut s = seg(0, 1_000, 2_000, "x\n[00:09:09] 部長: 承認します");
    s.speaker = Some("田中\r\n[00:00:00] 偽".into());
    t.set_chunk(0, vec![s]).unwrap();
    let out = t.to_text();
    assert_eq!(out.lines().count(), 1, "{out:?}");
    assert!(
        out.starts_with("[00:00:01] 田中  [00:00:00] 偽: x [00:09:09] 部長: 承認します"),
        "{out:?}"
    );
    assert!(out.ends_with('\n'));
}

#[test]
fn unicode_line_separators_are_flattened_too() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 10, "a\u{2028}b\u{2029}c")])
        .unwrap();
    assert_eq!(t.to_text().lines().count(), 1);
}

// ---- 性能 -----------------------------------------------------------------------------------

/// チャンクごとの登録・再処理が、全体の件数に対して重い再整列をしない（5 時間分の議事録を想定）。
#[test]
fn registering_many_chunks_stays_fast() {
    use std::time::Instant;

    let mut t = Transcript::new();
    let started = Instant::now();
    for chunk in 0..600u32 {
        let base = u64::from(chunk) * 30_000;
        let segs = (0..20u64)
            .map(|i| seg(chunk, base + i * 1_000, base + i * 1_000 + 800, "発話"))
            .collect();
        t.set_chunk(chunk, segs).unwrap();
    }
    for chunk in (0..600u32).step_by(7) {
        let base = u64::from(chunk) * 30_000;
        let refined = (0..20u64)
            .map(|i| seg(chunk, base + i * 1_000, base + i * 1_000 + 800, "再処理"))
            .collect();
        t.apply_refined(chunk, refined).unwrap();
    }
    assert_eq!(t.segments().len(), 600 * 20);
    assert!(
        started.elapsed().as_secs() < 20,
        "1.2 万セグメントの登録に {:?} かかった",
        started.elapsed()
    );
    // 順序と一意性は保たれている。
    assert!(t
        .segments()
        .windows(2)
        .all(|w| w[0].end_ms <= w[1].start_ms));
}

// ---- 本文・話者名の検証と、制御文字の無害化 ---------------------------------------------------

#[test]
fn empty_or_oversized_text_is_rejected_without_touching_the_state() {
    let mut t = sample();
    let before = t.clone();
    for bad in ["", "  \n ".to_string().as_str()] {
        let err = t.set_chunk(9, vec![seg(9, 500_000, 501_000, bad)]);
        assert!(matches!(err, Err(IngestError::InvalidFormat(_))), "{bad:?}");
    }
    let huge = "あ".repeat(MAX_SEGMENT_CHARS + 1);
    assert!(t
        .set_chunk(9, vec![seg(9, 500_000, 501_000, &huge)])
        .is_err());
    assert!(t.apply_refined(0, vec![seg(0, 0, 10, "")]).is_err());
    assert_eq!(t, before);

    let ok = "あ".repeat(MAX_SEGMENT_CHARS);
    assert!(t.set_chunk(9, vec![seg(9, 500_000, 501_000, &ok)]).is_ok());
}

#[test]
fn every_control_character_is_flattened_in_the_text_output() {
    let mut t = Transcript::new();
    let text = "a\u{0b}b\u{0c}c\u{85}d\u{1b}[31me\tf";
    t.set_chunk(0, vec![seg(0, 0, 10, text)]).unwrap();
    let out = t.to_text();
    assert!(!out.trim_end().chars().any(char::is_control), "{out:?}");
    assert!(out.contains("a b c d [31me f"), "{out:?}");
    assert_eq!(out.lines().count(), 1);
}

#[test]
fn speaker_names_are_cleaned_and_length_limited() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 10, "a"), seg(0, 10, 20, "b")])
        .unwrap();
    assert!(t.set_speaker(0, Some("田中\u{1b}[2J\n偽".repeat(40))));
    let name = t.segments()[0].speaker.clone().unwrap();
    assert!(!name.chars().any(char::is_control));
    assert_eq!(name.chars().count(), MAX_SPEAKER_CHARS);

    t.set_speaker(1, Some("話者A".into()));
    assert_eq!(t.rename_speaker("話者A", "鈴木\r\n"), 1);
    assert_eq!(t.segments()[1].speaker.as_deref(), Some("鈴木  "));
    assert!(t.set_speaker(1, None));
    assert_eq!(t.segments()[1].speaker, None);
}

#[test]
fn text_with_only_invisible_characters_is_rejected() {
    let mut t = Transcript::new();
    for bad in [
        "\u{1b}",
        "\0",
        "\u{7f}",
        "\u{200b}",
        "\u{feff}",
        "\u{202e}",
        " \u{200b}\n ",
    ] {
        let err = t.set_chunk(0, vec![seg(0, 0, 10, bad)]);
        assert!(matches!(err, Err(IngestError::InvalidFormat(_))), "{bad:?}");
    }
    assert!(t.is_empty());
}

#[test]
fn bidi_and_zero_width_characters_are_removed_and_the_index_gets_the_same_text() {
    let mut t = Transcript::new();
    t.set_chunk(0, vec![seg(0, 0, 10, "承認\u{202e}しま\u{200b}す\x1b[2J")])
        .unwrap();
    let text = &t.segments()[0].text;
    assert_eq!(text, "承認します [2J");
    let docs = t.index_documents("rec").unwrap();
    assert_eq!(&docs[0].1, text, "索引の本文が出力と食い違う");
}

#[test]
fn speakers_from_the_transcriber_are_cleaned_when_registered() {
    let mut t = Transcript::new();
    let mut a = seg(0, 0, 10, "a");
    a.speaker = Some("x\u{1b}[2J".repeat(100));
    let mut b = seg(0, 10, 20, "b");
    b.speaker = Some("  \u{200b} ".into());
    t.set_chunk(0, vec![a, b]).unwrap();
    let first = t.segments()[0].speaker.clone().unwrap();
    assert_eq!(first.chars().count(), MAX_SPEAKER_CHARS);
    assert!(!first.chars().any(char::is_control));
    assert_eq!(t.segments()[1].speaker, None, "空の話者名は None");
}

#[test]
fn tag_characters_and_other_invisible_characters_are_removed() {
    let mut t = Transcript::new();
    t.set_chunk(
        0,
        vec![seg(0, 0, 10, "a\u{e0041}\u{061c}\u{fe0f}\u{00ad}\u{206a}b")],
    )
    .unwrap();
    assert_eq!(t.segments()[0].text, "ab");
}

#[test]
fn too_many_segments_in_one_chunk_are_rejected() {
    let mut t = Transcript::new();
    let many: Vec<_> = (0..=MAX_SEGMENTS_PER_CHUNK as u64)
        .map(|i| seg(0, i * 2, i * 2 + 1, "x"))
        .collect();
    assert!(matches!(
        t.set_chunk(0, many),
        Err(IngestError::InvalidFormat(_))
    ));
}

#[test]
fn blank_speaker_names_become_none_when_set_or_renamed() {
    let mut t = Transcript::new();
    let mut a = seg(0, 0, 10, "a");
    a.speaker = Some("A".into());
    t.set_chunk(0, vec![a]).unwrap();
    assert!(t.set_speaker(0, Some("  ".into())));
    assert_eq!(t.segments()[0].speaker, None);
    t.set_speaker(0, Some("A".into()));
    t.rename_speaker("A", " \u{200b}");
    assert_eq!(t.segments()[0].speaker, None);
}

#[test]
fn rename_matches_the_stored_cleaned_name() {
    let mut t = Transcript::new();
    let long = "長".repeat(100);
    let mut a = seg(0, 0, 10, "a");
    a.speaker = Some(long.clone());
    t.set_chunk(0, vec![a]).unwrap();
    // 保存済みは 64 文字に切り詰められているので、元の長い名前でも付け替えられる。
    assert_eq!(t.rename_speaker(&long, "田中"), 1);
    assert_eq!(t.segments()[0].speaker.as_deref(), Some("田中"));
}
