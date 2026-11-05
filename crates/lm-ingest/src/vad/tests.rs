use super::*;
use crate::audio::samples_to_ms;

const RATE: u32 = 8000;

/// 振幅 `amp` の 440Hz 正弦波（ミリ秒指定）。
fn tone(ms: u64, amp: f32) -> Vec<i16> {
    let n = (u64::from(RATE) * ms / 1000) as usize;
    (0..n)
        .map(|i| (amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin()) as i16)
        .collect()
}

fn silence(ms: u64) -> Vec<i16> {
    vec![0; (u64::from(RATE) * ms / 1000) as usize]
}

fn concat(parts: &[Vec<i16>]) -> Vec<i16> {
    parts.iter().flatten().copied().collect()
}

fn vad() -> EnergyVad {
    EnergyVad::new(VadConfig::default()).unwrap()
}

fn detect(samples: &[i16]) -> Vec<SpeechSpan> {
    vad().detect(samples, RATE).unwrap()
}

#[test]
fn pure_silence_has_no_speech() {
    assert!(detect(&silence(3000)).is_empty());
    assert!(detect(&[]).is_empty());
}

#[test]
fn one_burst_is_found_with_padding() {
    let samples = concat(&[silence(1000), tone(1000, 8000.0), silence(1000)]);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 1);
    // 1000ms〜2000ms の発話に前後 100ms の余白。フレーム境界（20ms）の誤差を許容する。
    assert!((spans[0].start_ms as i64 - 900).abs() <= 20, "{spans:?}");
    assert!((spans[0].end_ms as i64 - 2100).abs() <= 20, "{spans:?}");
}

#[test]
fn two_bursts_separated_by_a_long_silence_stay_separate() {
    let samples = concat(&[tone(600, 8000.0), silence(1500), tone(600, 8000.0)]);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert!(spans[0].end_ms < spans[1].start_ms);
}

#[test]
fn a_short_pause_inside_speech_is_bridged() {
    // 200ms の無音（< min_silence 300ms）は発話の途切れとして結合される。
    let samples = concat(&[tone(600, 8000.0), silence(200), tone(600, 8000.0)]);
    assert_eq!(detect(&samples).len(), 1);
}

#[test]
fn a_blip_shorter_than_min_speech_is_dropped() {
    let samples = concat(&[silence(1000), tone(100, 8000.0), silence(1000)]);
    assert!(detect(&samples).is_empty());
}

#[test]
fn speech_touching_the_edges_is_clamped_to_the_signal() {
    let samples = tone(1000, 8000.0);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].start_ms, 0);
    assert_eq!(spans[0].end_ms, 1000);
}

#[test]
fn quiet_noise_below_the_threshold_is_not_speech() {
    assert!(detect(&tone(2000, 100.0)).is_empty());
}

#[test]
fn threshold_adapts_to_a_noisy_floor() {
    // 常時ある振幅 1000 の雑音（固定閾値 300 では全部が発話になる）の中に、振幅 9000 の発話。
    let noise = |ms| tone(ms, 1000.0);
    let samples = concat(&[noise(1500), tone(800, 9000.0), noise(1500)]);
    let adaptive = detect(&samples);
    assert_eq!(adaptive.len(), 1, "{adaptive:?}");
    assert!(adaptive[0].duration_ms() < 1200, "{adaptive:?}");

    let fixed = EnergyVad::new(VadConfig {
        noise_ratio: 0.0,
        ..VadConfig::default()
    })
    .unwrap();
    let spans = fixed.detect(&samples, RATE).unwrap();
    assert_eq!(spans.len(), 1);
    assert!(
        spans[0].duration_ms() > 3000,
        "固定閾値では全体が発話になる: {spans:?}"
    );
}

#[test]
fn spans_are_ordered_and_never_overlap() {
    let mut parts = Vec::new();
    for _ in 0..6 {
        parts.push(tone(300, 8000.0));
        parts.push(silence(350));
    }
    let spans = detect(&concat(&parts));
    for w in spans.windows(2) {
        assert!(w[0].end_ms < w[1].start_ms, "{spans:?}");
    }
    for s in &spans {
        assert!(s.start_ms < s.end_ms);
    }
}

#[test]
fn extreme_amplitudes_do_not_overflow() {
    let samples = vec![i16::MIN; 8000];
    assert_eq!(detect(&samples).len(), 1);
    let samples = vec![i16::MAX; 8000];
    assert_eq!(detect(&samples).len(), 1);
}

#[test]
fn a_signal_shorter_than_one_frame_is_handled() {
    let samples = tone(5, 9000.0); // 40 サンプル < 160 サンプル（20ms）
    let _ = detect(&samples); // パニックしないこと
}

#[test]
fn invalid_settings_are_rejected() {
    let bad = [
        VadConfig {
            frame_ms: 0,
            ..VadConfig::default()
        },
        VadConfig {
            min_threshold: f32::NAN,
            ..VadConfig::default()
        },
        VadConfig {
            min_threshold: -1.0,
            ..VadConfig::default()
        },
        VadConfig {
            noise_ratio: f32::INFINITY,
            ..VadConfig::default()
        },
    ];
    for c in bad {
        assert!(EnergyVad::new(c).is_err(), "{c:?}");
    }
    assert!(vad().detect(&[0; 10], 0).is_err());
}

/// 無音を一切含まない連続発話を、無音と誤判定して失わない（自動閾値の上限の回帰テスト）。
#[test]
fn continuous_speech_without_any_silence_is_not_lost() {
    // 声の大きさが揺れる 30 秒の連続発話。
    let parts: Vec<Vec<i16>> = (0..100)
        .map(|i| tone(300, if i % 2 == 0 { 8000.0 } else { 3000.0 }))
        .collect();
    let samples = concat(&parts);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert_eq!(spans[0].start_ms, 0);
    assert!(spans[0].end_ms >= 29_900, "{spans:?}");
}

#[test]
fn threshold_never_exceeds_a_quarter_of_the_loud_level() {
    let v = vad();
    // 全フレームが同じ大きさ: ノイズフロア × 3 ではなく、その 4 分の 1 が閾値になる。
    let rms = vec![4000.0f32; 100];
    assert_eq!(v.threshold(&rms), 1000.0);
    // 十分に静かな信号では下限の min_threshold が優先される。
    let quiet = vec![100.0f32; 100];
    assert_eq!(v.threshold(&quiet), VadConfig::default().min_threshold);
}

#[test]
fn quantiles_pick_the_expected_points() {
    let rms: Vec<f32> = (1..=10).map(|i| i as f32 * 100.0).collect();
    assert_eq!(noise_floor(&rms), 200.0); // sorted[10 * 0.1 = 1]
    assert_eq!(loud_level(&rms), 1000.0); // sorted[min(9, 9)]
    assert_eq!(quantile(&[], 0.5), 0.0);
    assert_eq!(quantile(&[500.0], 0.9), 500.0);
}

#[test]
fn noise_floor_uses_the_lower_decile() {
    let rms: Vec<f32> = (1..=10).map(|i| i as f32 * 100.0).collect();
    assert_eq!(noise_floor(&rms), 200.0); // sorted[10/10 = 1]
    assert_eq!(noise_floor(&[]), 0.0);
    assert_eq!(noise_floor(&[500.0]), 500.0);
}

// ---- 時刻の算術（割り切れないサンプルレート）---------------------------------------------

/// 任意のレートの正弦波 / 無音（ミリ秒指定）。
fn tone_at(rate: u32, ms: u64, amp: f32) -> Vec<i16> {
    let n = (u64::from(rate) * ms / 1000) as usize;
    (0..n)
        .map(|i| (amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin()) as i16)
        .collect()
}

fn silence_at(rate: u32, ms: u64) -> Vec<i16> {
    vec![0; (u64::from(rate) * ms / 1000) as usize]
}

/// 11025Hz × 20ms = 220.5 サンプル。フレームを 20ms と数えると、10 分のチャンクの末尾で約 1.4 秒ずれる。
#[test]
fn positions_do_not_drift_at_rates_that_do_not_divide_evenly() {
    let rate = 11_025;
    let samples = concat(&[silence_at(rate, 598_000), tone_at(rate, 2_000, 8000.0)]);
    let no_pad = EnergyVad::new(VadConfig {
        pad_ms: 0,
        ..VadConfig::default()
    })
    .unwrap();
    let spans = no_pad.detect(&samples, rate).unwrap();
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(
        (spans[0].start_ms as i64 - 598_000).abs() <= 25,
        "{spans:?}"
    );
    assert_eq!(spans[0].end_ms, 600_000);
}

/// 最後の端数フレームだけが有声でも、`start_ms >= end_ms` の区間は出ない。
#[test]
fn spans_are_always_well_formed_for_every_tail_length_and_rate() {
    for rate in [1_000u32, 8_000, 11_025, 22_050, 44_100, 48_000] {
        let v = EnergyVad::new(VadConfig {
            min_speech_ms: 0,
            min_silence_ms: 0,
            ..VadConfig::default()
        })
        .unwrap();
        for tail in [1usize, 2, 5, 17, 100, 219, 220, 221, 441, 1000] {
            let mut samples = silence_at(rate, 1_000);
            samples.extend(std::iter::repeat_n(20_000i16, tail));
            let total = samples_to_ms(samples.len() as u64, rate);
            for span in v.detect(&samples, rate).unwrap() {
                assert!(
                    span.start_ms < span.end_ms,
                    "rate {rate} tail {tail}: {span:?}"
                );
                assert!(
                    span.end_ms <= total,
                    "rate {rate} tail {tail}: {span:?} > {total}"
                );
            }
        }
    }
}

// ---- 背景雑音・小声 -----------------------------------------------------------------------

fn nearly(actual: u64, expected: u64, tolerance: u64) -> bool {
    actual.abs_diff(expected) <= tolerance
}

/// 30 秒のうち 2 秒（6.7%）だけ話す。背景（振幅 1000）に対して発話（振幅 9000）。
#[test]
fn a_short_utterance_over_steady_background_noise_is_isolated() {
    let samples = concat(&[
        tone(14_000, 1000.0),
        tone(2_000, 9000.0),
        tone(14_000, 1000.0),
    ]);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(nearly(spans[0].start_ms, 13_900, 150), "{spans:?}");
    assert!(nearly(spans[0].end_ms, 16_100, 150), "{spans:?}");
}

#[test]
fn a_quiet_voice_in_a_quiet_room_is_not_dropped() {
    // 振幅 350（RMS 約 247）の小声。以前の下限 300 では区間が 0 件になり、発話が丸ごと落ちていた。
    let samples = concat(&[silence(2_000), tone(1_500, 350.0), silence(2_000)]);
    let spans = detect(&samples);
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert!(nearly(spans[0].start_ms, 1_900, 60), "{spans:?}");
}

/// 背景と発話が近い（振幅比 2.7 倍）と分離できないが、発話は必ず区間に含まれる（取りこぼしは起きない）。
#[test]
fn when_speech_and_background_are_too_close_nothing_is_lost() {
    let samples = concat(&[
        tone(3_000, 1500.0),
        tone(2_000, 4000.0),
        tone(3_000, 1500.0),
    ]);
    let spans = detect(&samples);
    assert!(
        spans
            .iter()
            .any(|s| s.start_ms <= 3_000 && s.end_ms >= 5_000),
        "発話 3〜5 秒が区間に含まれていない: {spans:?}"
    );
}

// ---- 境界値（ちょうど閾値・ちょうど最短長）---------------------------------------------

/// 1000Hz・10ms フレーム（= 10 サンプル）で、ミリ秒を正確に作れる固定閾値の VAD。
fn exact_vad(threshold: f32, pad_ms: u32) -> EnergyVad {
    EnergyVad::new(VadConfig {
        frame_ms: 10,
        min_threshold: threshold,
        noise_ratio: 0.0,
        min_speech_ms: 200,
        min_silence_ms: 300,
        pad_ms,
    })
    .unwrap()
}

fn frames(pattern: &[(u32, i16)]) -> Vec<i16> {
    pattern
        .iter()
        .flat_map(|&(count, value)| std::iter::repeat_n(value, count as usize * 10))
        .collect()
}

#[test]
fn speech_of_exactly_the_minimum_length_is_kept_and_one_frame_less_is_dropped() {
    let v = exact_vad(1000.0, 0);
    let kept = v
        .detect(&frames(&[(30, 0), (20, 10_000), (30, 0)]), 1000)
        .unwrap(); // 200ms
    assert_eq!(
        kept,
        vec![SpeechSpan {
            start_ms: 300,
            end_ms: 500
        }]
    );
    let dropped = v
        .detect(&frames(&[(30, 0), (19, 10_000), (30, 0)]), 1000)
        .unwrap(); // 190ms
    assert!(dropped.is_empty(), "{dropped:?}");
}

#[test]
fn a_silence_of_exactly_the_minimum_length_splits_and_one_frame_less_bridges() {
    let v = exact_vad(1000.0, 0);
    let split = v
        .detect(&frames(&[(30, 10_000), (30, 0), (30, 10_000)]), 1000) // 無音 300ms
        .unwrap();
    assert_eq!(split.len(), 2, "{split:?}");
    let bridged = v
        .detect(&frames(&[(30, 10_000), (29, 0), (30, 10_000)]), 1000) // 無音 290ms
        .unwrap();
    assert_eq!(
        bridged,
        vec![SpeechSpan {
            start_ms: 0,
            end_ms: 890
        }]
    );
}

#[test]
fn a_frame_exactly_at_the_threshold_counts_as_speech() {
    // 一定値 1000 のフレームの RMS はちょうど 1000.0。
    let samples = frames(&[(30, 1000)]);
    assert_eq!(
        exact_vad(1000.0, 0).detect(&samples, 1000).unwrap().len(),
        1
    );
    assert!(exact_vad(1000.5, 0)
        .detect(&samples, 1000)
        .unwrap()
        .is_empty());
}

#[test]
fn spans_that_touch_after_padding_are_merged_and_one_ms_short_stays_split() {
    let signal = frames(&[(30, 10_000), (30, 0), (30, 10_000)]); // 無音 300ms は結合されない
                                                                 // 余白 150ms ずつ: 先の終了 450 と後の開始 450 がちょうど接する → 1 つに結合。
    let touching = exact_vad(1000.0, 150).detect(&signal, 1000).unwrap();
    assert_eq!(touching.len(), 1, "{touching:?}");
    let apart = exact_vad(1000.0, 149).detect(&signal, 1000).unwrap();
    assert_eq!(apart.len(), 2, "{apart:?}");
}
