//! VAD の入力上限と極端な設定のテスト。

use super::*;
use crate::audio::MAX_CHUNK_SECONDS;

#[test]
fn input_longer_than_one_chunk_is_rejected() {
    let vad = EnergyVad::new(VadConfig::default()).unwrap();
    let max = 1000usize * MAX_CHUNK_SECONDS as usize;
    assert!(vad.detect(&vec![0; max], 1000).is_ok());
    assert!(matches!(
        vad.detect(&vec![0; max + 1], 1000),
        Err(IngestError::InvalidFormat(_))
    ));
}

/// 極端に長いフレーム長でも panic せず、結果は空か 1 区間になる。
#[test]
fn an_enormous_frame_length_does_not_panic() {
    let config = VadConfig {
        frame_ms: u32::MAX,
        ..VadConfig::default()
    };
    let vad = EnergyVad::new(config).unwrap();
    let spans = vad.detect(&vec![10_000; 2000], 1000).unwrap();
    assert!(spans.len() <= 1);
    assert!(spans.iter().all(|s| s.start_ms < s.end_ms));
}
