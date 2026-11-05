//! 意味検索の下駄（semantic_floor）と ONNX プリセットのテスト。

use super::tests::{is_not_found, searcher, StubLex, StubSem};
use super::*;

// ---- 意味検索の下駄（semantic_floor） ---------------------------------------------

fn floor_searcher(floor: f32) -> HybridSearcher {
    // 語彙側の根拠は 0.05、意味側の最上位の cosine は 0.9。
    HybridSearcher::new(
        StubLex::new("x", 0.05, &[]),
        Arc::new(StubSem("x")),
        HybridConfig {
            semantic_floor: floor,
            ..HybridConfig::default()
        },
    )
}

fn grounding_of(s: &HybridSearcher) -> f32 {
    match s.search("q", 3).unwrap() {
        SearchOutcome::Found { grounding, .. } | SearchOutcome::NotFound { grounding } => grounding,
    }
}

#[test]
fn floor_zero_uses_the_cosine_as_is() {
    assert!((grounding_of(&floor_searcher(0.0)) - 0.9).abs() < 1e-6);
}

#[test]
fn floor_rescales_a_compressed_cosine_range() {
    // (0.9 - 0.8) / (1 - 0.8) = 0.5
    assert!((grounding_of(&floor_searcher(0.8)) - 0.5).abs() < 1e-6);
    // cosine が下駄以下なら意味側の根拠は 0（語彙側の 0.05 が残る）。
    let g = grounding_of(&floor_searcher(0.95));
    assert!((g - 0.05).abs() < 1e-6, "{g}");
}

#[test]
fn floor_turns_an_accepted_match_into_a_refusal() {
    assert!(matches!(
        floor_searcher(0.0).search("q", 3).unwrap(),
        SearchOutcome::Found { .. }
    ));
    assert!(is_not_found(&floor_searcher(0.95).search("q", 3).unwrap()));
}

#[test]
fn with_semantic_floor_shares_the_indexes_and_leaves_the_original_alone() {
    let base = floor_searcher(0.0);
    let strict = base.with_semantic_floor(0.95);
    assert!(is_not_found(&strict.search("q", 3).unwrap()));
    assert!(matches!(
        base.search("q", 3).unwrap(),
        SearchOutcome::Found { .. }
    ));
}

#[test]
fn invalid_floors_are_rejected() {
    for bad in [f32::NAN, 1.0, 1.5, -0.1, f32::INFINITY, f32::NEG_INFINITY] {
        let s = floor_searcher(bad);
        assert!(
            matches!(s.search("q", 3), Err(SearchError::Config(_))),
            "{bad}"
        );
    }
}

#[test]
fn the_onnx_preset_is_valid_and_distinct_from_the_default() {
    let preset = HybridConfig::onnx_e5();
    assert!(preset.validate().is_ok());
    assert_eq!(preset.semantic_floor, ONNX_E5_SEMANTIC_FLOOR);
    assert_eq!(preset.grounding_threshold, ONNX_E5_GROUNDING_THRESHOLD);
    assert!(preset.semantic_floor > HybridConfig::default().semantic_floor);
}

/// 評価（43 問・下駄 0.78）で誤りゼロだった閾値の範囲 0.26〜0.30 に、プリセットが収まっていること。
/// 定数を後から動かしたときに、範囲を外れて気付かないのを防ぐ。
#[test]
fn the_onnx_preset_stays_inside_the_measured_zero_error_range() {
    assert_eq!(ONNX_E5_SEMANTIC_FLOOR, 0.78);
    assert!((0.26..=0.30).contains(&ONNX_E5_GROUNDING_THRESHOLD));
}

#[test]
fn negative_finite_thresholds_are_rejected_but_zero_is_allowed() {
    for bad in [-0.5, -1e-6, f32::MIN, f32::NAN] {
        let s = searcher().with_threshold(bad);
        assert!(
            matches!(s.search("zzzz", 3), Err(SearchError::Config(_))),
            "{bad}"
        );
    }
    assert!(searcher().with_threshold(0.0).search("AB-1234", 3).is_ok());
}

#[test]
fn negative_infinity_threshold_is_rejected_but_positive_infinity_is_safe() {
    let s = searcher().with_threshold(f32::NEG_INFINITY);
    assert!(matches!(s.search("zzzz", 3), Err(SearchError::Config(_))));
    // +∞ は「何でも見つかりません」になる安全側の設定。
    let strict = searcher().with_threshold(f32::INFINITY);
    assert!(is_not_found(&strict.search("AB-1234", 3).unwrap()));
}
