use super::*;

#[test]
fn sixteen_bit_samples_become_floats_in_range() {
    let out = to_whisper_input(&[0, 16_384, -16_384, i16::MAX, i16::MIN], 16_000).unwrap();
    assert_eq!(out[..3], [0.0, 0.5, -0.5]);
    assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)));
    assert_eq!(out[4], -1.0);
}

#[test]
fn other_rates_are_resampled_to_16k_with_the_right_length() {
    let pcm: Vec<i16> = (0..48_000).map(|i| (i % 1000) as i16).collect();
    assert_eq!(to_whisper_input(&pcm, 48_000).unwrap().len(), 16_000);
    let pcm8: Vec<i16> = vec![100; 8_000];
    let out = to_whisper_input(&pcm8, 8_000).unwrap();
    assert_eq!(out.len(), 16_000, "8kHz の 1 秒は 16kHz の 1 秒");
    assert!(
        out.iter().all(|&s| (s - 100.0 / 32_768.0).abs() < 1e-6),
        "一定の値は変わらない"
    );
}

#[test]
fn a_ramp_stays_a_ramp_after_resampling() {
    let pcm: Vec<i16> = (0..8_000).map(|i| i as i16).collect();
    let out = to_whisper_input(&pcm, 8_000).unwrap();
    assert!(out.windows(2).all(|w| w[1] >= w[0] - 1e-6), "単調に増える");
}

#[test]
fn empty_input_is_empty_and_bad_rates_or_lengths_are_refused() {
    assert!(to_whisper_input(&[], 16_000).unwrap().is_empty());
    assert!(to_whisper_input(&[], 44_100).unwrap().is_empty());
    for rate in [0, 7_999, 96_001] {
        assert!(to_whisper_input(&[0], rate).is_err(), "{rate}");
    }
    let too_long = vec![0i16; (16_000 * MAX_SECONDS) as usize + 1];
    assert!(to_whisper_input(&too_long, 16_000).is_err());
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn tone(freq: f32, rate: u32, seconds: u32) -> Vec<i16> {
    (0..rate * seconds)
        .map(|i| ((i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * 16_000.0) as i16)
        .collect()
}

#[test]
fn downsampling_keeps_speech_band_tones_and_suppresses_aliasing_ones() {
    // 1kHz（声の帯域）は、ほぼそのまま。
    let voice = to_whisper_input(&tone(1_000.0, 48_000, 1), 48_000).unwrap();
    let voice_in = rms(&to_whisper_input(&tone(1_000.0, 16_000, 1), 16_000).unwrap());
    assert!(
        rms(&voice) > voice_in * 0.85,
        "{} / {}",
        rms(&voice),
        voice_in
    );
    // 20kHz（16kHz では表せない高い音）は、折り返して雑音にならないよう、大きく弱める。
    let ultra = to_whisper_input(&tone(20_000.0, 48_000, 1), 48_000).unwrap();
    assert!(
        rms(&ultra) < voice_in * 0.35,
        "{} / {}",
        rms(&ultra),
        voice_in
    );
}
