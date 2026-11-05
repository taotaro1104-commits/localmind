use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;
use crate::vad::VadConfig;

const RATE: u32 = 8000;

fn tone(ms: u64) -> Vec<i16> {
    let n = (u64::from(RATE) * ms / 1000) as usize;
    (0..n)
        .map(|i| {
            (8000.0 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin()) as i16
        })
        .collect()
}

fn silence(ms: u64) -> Vec<i16> {
    vec![0; (u64::from(RATE) * ms / 1000) as usize]
}

fn concat(parts: &[Vec<i16>]) -> Vec<i16> {
    parts.iter().flatten().copied().collect()
}

fn chunk(index: u32, start_ms: u64, samples: Vec<i16>) -> Chunk {
    Chunk {
        index,
        start_ms,
        sample_rate: RATE,
        samples,
    }
}

/// 呼ばれるたびに用意した結果を順に返し、渡された PCM の長さを記録するスタブ。
struct Scripted {
    results: Mutex<VecDeque<Result<Utterance, IngestError>>>,
    seen_lens: Mutex<Vec<usize>>,
}

impl Scripted {
    fn new(results: Vec<Result<Utterance, IngestError>>) -> Self {
        Self {
            results: Mutex::new(results.into()),
            seen_lens: Mutex::new(Vec::new()),
        }
    }

    fn says(texts: &[&str]) -> Self {
        Self::new(texts.iter().map(|t| Ok(utterance(t, 0.9))).collect())
    }
}

impl Transcriber for Scripted {
    fn transcribe(&self, pcm: &[i16], _: u32) -> Result<Utterance, IngestError> {
        self.seen_lens.lock().unwrap().push(pcm.len());
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(utterance("(台本切れ)", 0.5)))
    }
    fn id(&self) -> String {
        "scripted".into()
    }
}

fn utterance(text: &str, confidence: f32) -> Utterance {
    Utterance {
        text: text.into(),
        confidence,
    }
}

fn vad() -> EnergyVad {
    EnergyVad::new(VadConfig::default()).unwrap()
}

#[test]
fn only_speech_spans_reach_the_engine_and_times_are_absolute() {
    // 1 秒無音 → 1 秒発話 → 1.5 秒無音 → 1 秒発話 → 1 秒無音。チャンクは録音の 30 秒目から。
    let samples = concat(&[
        silence(1000),
        tone(1000),
        silence(1500),
        tone(1000),
        silence(1000),
    ]);
    let t = Scripted::says(&["こんにちは", "会議を始めます"]);
    let segs = transcribe_chunk(&chunk(1, 30_000, samples), &vad(), &t, Stage::Draft).unwrap();

    assert_eq!(segs.len(), 2);
    assert_eq!(segs[0].text, "こんにちは");
    assert_eq!(segs[1].text, "会議を始めます");
    assert!((segs[0].start_ms as i64 - 30_900).abs() <= 20, "{segs:?}");
    assert!((segs[1].start_ms as i64 - 33_400).abs() <= 40, "{segs:?}");
    assert!(segs
        .iter()
        .all(|s| s.chunk_index == 1 && s.stage == Stage::Draft));
    assert!(segs.iter().all(|s| s.speaker.is_none()));
    assert!(segs[0].end_ms < segs[1].start_ms);

    // エンジンへ渡った PCM は発話区間（余白込みで約 1.2 秒）だけで、チャンク全体（5.5 秒）ではない。
    let lens = t.seen_lens.lock().unwrap().clone();
    assert_eq!(lens.len(), 2);
    assert!(lens.iter().all(|&n| n < RATE as usize * 2), "{lens:?}");
}

#[test]
fn pure_silence_never_calls_the_engine() {
    let t = Scripted::says(&["幻聴"]);
    let segs = transcribe_chunk(&chunk(0, 0, silence(5000)), &vad(), &t, Stage::Draft).unwrap();
    assert!(segs.is_empty());
    assert!(t.seen_lens.lock().unwrap().is_empty());
}

#[test]
fn empty_or_whitespace_results_are_dropped() {
    let samples = concat(&[tone(800), silence(1000), tone(800)]);
    let t = Scripted::says(&["   \n", "ありがとう"]);
    let segs = transcribe_chunk(&chunk(0, 0, samples), &vad(), &t, Stage::Refined).unwrap();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].text, "ありがとう");
    assert_eq!(segs[0].stage, Stage::Refined);
}

#[test]
fn text_is_trimmed() {
    let t = Scripted::says(&["  前後に空白  "]);
    let segs = transcribe_chunk(&chunk(0, 0, tone(1000)), &vad(), &t, Stage::Draft).unwrap();
    assert_eq!(segs[0].text, "前後に空白");
}

#[test]
fn engine_failure_is_returned_so_the_chunk_can_be_retried() {
    let samples = concat(&[tone(800), silence(1000), tone(800)]);
    let t = Scripted::new(vec![
        Ok(utterance("1つ目", 0.9)),
        Err(IngestError::Transcribe("モデル未ロード".into())),
    ]);
    let err = transcribe_chunk(&chunk(0, 0, samples), &vad(), &t, Stage::Draft).unwrap_err();
    assert_eq!(err, IngestError::Transcribe("モデル未ロード".into()));
}

#[test]
fn confidence_is_sanitized() {
    let samples = concat(&[
        tone(600),
        silence(1000),
        tone(600),
        silence(1000),
        tone(600),
        silence(1000),
        tone(600),
    ]);
    let t = Scripted::new(vec![
        Ok(utterance("a", f32::NAN)),
        Ok(utterance("b", 7.5)),
        Ok(utterance("c", -3.0)),
        Ok(utterance("d", 0.42)),
    ]);
    let segs = transcribe_chunk(&chunk(0, 0, samples), &vad(), &t, Stage::Draft).unwrap();
    let conf: Vec<f32> = segs.iter().map(|s| s.confidence).collect();
    assert_eq!(conf, vec![0.0, 1.0, 0.0, 0.42]);
}

#[test]
fn oversized_text_is_truncated_on_a_char_boundary() {
    let huge = "あ".repeat(MAX_SEGMENT_CHARS * 3);
    let t = Scripted::says(&[huge.as_str()]);
    let segs = transcribe_chunk(&chunk(0, 0, tone(1000)), &vad(), &t, Stage::Draft).unwrap();
    assert_eq!(segs[0].text.chars().count(), MAX_SEGMENT_CHARS);
}

#[test]
fn empty_chunk_yields_nothing() {
    let t = Scripted::says(&["x"]);
    assert!(
        transcribe_chunk(&chunk(0, 0, vec![]), &vad(), &t, Stage::Draft)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn invalid_sample_rate_is_an_error_not_a_panic() {
    let t = Scripted::says(&["x"]);
    let bad = Chunk {
        index: 0,
        start_ms: 0,
        sample_rate: 0,
        samples: vec![1; 100],
    };
    assert!(transcribe_chunk(&bad, &vad(), &t, Stage::Draft).is_err());
}

#[test]
fn a_chunk_with_an_absurd_start_position_is_rejected_instead_of_overflowing() {
    let t = Scripted::says(&["x"]);
    let c = Chunk {
        index: 0,
        start_ms: u64::MAX - 5,
        sample_rate: RATE,
        samples: tone(1000),
    };
    assert!(matches!(
        transcribe_chunk(&c, &vad(), &t, Stage::Draft),
        Err(IngestError::InvalidFormat(_))
    ));
    assert!(t.seen_lens.lock().unwrap().is_empty());
}
