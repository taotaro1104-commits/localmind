//! 実モデルでの確認（通常は実行しない）。モデルと音声を環境変数で渡して実行する:
//!
//! ```text
//! LM_WHISPER_MODEL=<ggml-*.bin> LM_WHISPER_SHA256=<64 桁> LM_WHISPER_WAV=<16kHz mono の wav> \
//!   cargo test -p lm-whisper --release --test real_model -- --ignored --nocapture
//! ```
//! 音声は、日本語で「予算」「増員」「承認」などと話すものを想定する。含まれているべき語は、環境変数
//! `LM_WHISPER_KEYWORDS`（カンマ区切り。既定は「予算」）で指定する。小さなモデル（base）は漢字の誤りが多いので、
//! 厳しい語での確認は、大きなモデル（small 以上）で行う。

use std::path::PathBuf;
use std::time::Instant;

use lm_ingest::transcribe::transcribe_chunk;
use lm_ingest::{Chunk, EnergyVad, Stage, Transcriber, VadConfig};
use lm_whisper::{Quality, WhisperConfig, WhisperTranscriber};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// 文字起こしに含まれているべき語。
fn keywords() -> Vec<String> {
    env("LM_WHISPER_KEYWORDS")
        .unwrap_or_else(|| "予算".into())
        .split(',')
        .map(str::to_string)
        .collect()
}

/// 16bit・モノラルの wav から、サンプルレートと PCM を取り出す（`data` チャンクを探す最小限の読み取り）。
fn read_wav(path: &PathBuf) -> (u32, Vec<i16>) {
    let b = std::fs::read(path).expect("wav を読めません");
    assert_eq!(&b[0..4], b"RIFF");
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let mut at = 12;
    while at + 8 <= b.len() {
        let size = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        if &b[at..at + 4] == b"data" {
            let data = &b[at + 8..(at + 8 + size).min(b.len())];
            return (
                rate,
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| i16::from_le_bytes(*s))
                    .collect(),
            );
        }
        at += 8 + size;
    }
    panic!("data チャンクがありません");
}

fn open(quality: Quality) -> Option<(WhisperTranscriber, u32, Vec<i16>)> {
    let (model, sha, wav) = (
        env("LM_WHISPER_MODEL")?,
        env("LM_WHISPER_SHA256")?,
        env("LM_WHISPER_WAV")?,
    );
    let started = Instant::now();
    let t = WhisperTranscriber::open(
        &PathBuf::from(model),
        &sha,
        WhisperConfig {
            quality,
            ..WhisperConfig::default()
        },
    )
    .expect("モデルを開けません");
    println!("モデルの読み込み: {:?}", started.elapsed());
    let (rate, pcm) = read_wav(&PathBuf::from(wav));
    Some((t, rate, pcm))
}

#[test]
#[ignore = "モデルと音声が要る（環境変数 LM_WHISPER_MODEL / LM_WHISPER_SHA256 / LM_WHISPER_WAV）"]
fn japanese_speech_is_transcribed() {
    for quality in [Quality::Fast, Quality::Accurate] {
        let Some((t, rate, pcm)) = open(quality) else {
            return;
        };
        let started = Instant::now();
        let u = t.transcribe(&pcm, rate).expect("文字起こし");
        let took = started.elapsed();
        let seconds = pcm.len() as f64 / f64::from(rate);
        println!(
            "{quality:?}: {took:?}（音声 {seconds:.1} 秒、{:.2} 倍速）信頼度 {:.2}\n  {}",
            seconds / took.as_secs_f64(),
            u.confidence,
            u.text
        );
        for word in keywords() {
            assert!(
                u.text.contains(&word),
                "{quality:?}: 「{word}」が含まれない: {}",
                u.text
            );
        }
        assert!((0.0..=1.0).contains(&u.confidence));
    }
}

#[test]
#[ignore = "モデルと音声が要る"]
fn the_pipeline_cuts_speech_with_the_vad_and_keeps_absolute_times() {
    let Some((t, rate, pcm)) = open(Quality::Fast) else {
        return;
    };
    // 先頭に 1.5 秒の無音を足して、発話の開始位置が無音の分だけ後ろになることも確かめる。
    let mut samples = vec![0i16; (rate as usize) * 3 / 2];
    samples.extend_from_slice(&pcm);
    let chunk = Chunk {
        index: 2,
        start_ms: 60_000,
        sample_rate: rate,
        samples,
    };
    let vad = EnergyVad::new(VadConfig::default()).unwrap();
    let segments = transcribe_chunk(&chunk, &vad, &t, Stage::Draft).expect("パイプライン");
    println!("{segments:#?}");
    assert!(!segments.is_empty());
    assert!(
        segments[0].start_ms >= 60_000 + 1_000,
        "録音全体の時刻で、無音の後ろから始まる: {}",
        segments[0].start_ms
    );
    let all: String = segments.iter().map(|s| s.text.as_str()).collect();
    for word in keywords() {
        assert!(all.contains(&word), "「{word}」が含まれない: {all}");
    }
}

#[test]
#[ignore = "モデルと音声が要る"]
fn cancelling_stops_a_running_inference_quickly_and_the_engine_works_afterwards() {
    use std::sync::atomic::Ordering;
    use std::time::Duration;
    let Some((t, rate, pcm)) = open(Quality::Fast) else {
        return;
    };
    let long: Vec<i16> = pcm.iter().cycle().take(pcm.len() * 6).copied().collect(); // 約 60 秒
    let flag = t.cancel_flag();
    let stopper = {
        let flag = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            flag.store(true, Ordering::Relaxed);
        })
    };
    let started = Instant::now();
    let result = t.transcribe(&long, rate);
    let took = started.elapsed();
    stopper.join().unwrap();
    println!("中止まで {took:?}: {result:?}");
    assert!(result.is_err(), "中止されたら、結果ではなくエラー");
    assert!(
        took < Duration::from_secs(20),
        "中止が効くまでが長すぎる: {took:?}"
    );
    // 旗を戻せば、同じエンジンでまた使える。
    flag.store(false, Ordering::Relaxed);
    let again = t.transcribe(&pcm, rate).expect("中止のあとにも使える");
    assert!(!again.text.is_empty());
}
