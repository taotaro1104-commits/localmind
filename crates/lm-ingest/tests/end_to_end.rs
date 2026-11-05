//! 録音 → チャンク保存 →（クラッシュ）→ 復元 → 文字起こし → 議事録 → 検索索引 → 引用から時刻へ、の通し。

use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;

use lm_ingest::transcript::parse_segment_ref;
use lm_ingest::{
    transcribe::transcribe_chunk, ChunkWriter, EnergyVad, FsChunkStore, IngestError, RecordingMeta,
    Stage, Transcriber, Transcript, Utterance, VadConfig,
};
use lm_search::{Bm25Index, NgramTokenizer};

const RATE: u32 = 8000;
const CHUNK_SECONDS: u32 = 4;

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

/// 12 秒の会議: 発話 3 回。チャンク 4 秒なので 3 チャンクにまたがる。
fn meeting_audio() -> Vec<i16> {
    [
        silence(500),
        tone(1500),
        silence(2000), // チャンク 0: 0.5〜2.0 秒に発話
        silence(500),
        tone(1500),
        silence(2000), // チャンク 1: 4.5〜6.0 秒に発話
        silence(500),
        tone(1500),
        silence(2000), // チャンク 2: 8.5〜10.0 秒に発話
    ]
    .concat()
}

struct Script(Mutex<VecDeque<&'static str>>);

impl Transcriber for Script {
    fn transcribe(&self, _: &[i16], _: u32) -> Result<Utterance, IngestError> {
        let text = self.0.lock().unwrap().pop_front().unwrap_or("(なし)");
        Ok(Utterance {
            text: text.into(),
            confidence: 0.9,
        })
    }
    fn id(&self) -> String {
        "script".into()
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("lm-ingest-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    p
}

#[test]
fn record_crash_recover_transcribe_index_and_cite() {
    let root = temp_root("full");
    let store = FsChunkStore::new(&root);
    store
        .save_meta(
            "meeting1",
            &RecordingMeta {
                consent_confirmed: true,
                sample_rate: RATE,
                started_at_ms: 1_730_000_000_000,
            },
        )
        .unwrap();

    // 録音: 小さな単位で PCM が届き、チャンクが埋まるたびに保存する。
    let audio = meeting_audio();
    let mut writer = ChunkWriter::new(RATE, CHUNK_SECONDS).unwrap();
    let mut saved = 0;
    for piece in audio.chunks(1234) {
        for chunk in writer.push(piece) {
            store.save_chunk("meeting1", &chunk).unwrap();
            saved += 1;
        }
    }
    assert!(writer.finish().is_none(), "ちょうど 3 チャンクで割り切れる");
    assert_eq!(saved, 3);

    // クラッシュ: 最後のチャンクの保存が途中で切れた。直前のチャンクまでは復元できる。
    let last = root.join("meeting1").join("00000002.pcm");
    let bytes = fs::read(&last).unwrap();
    fs::write(&last, &bytes[..bytes.len() / 2]).unwrap();
    let recovered = store.load_chunks("meeting1").unwrap();
    assert_eq!(recovered.chunks.len(), 2);
    assert_eq!(recovered.skipped.len(), 1);

    // 録音停止後に文字起こし（復元できたチャンクだけ）。
    let vad = EnergyVad::new(VadConfig::default()).unwrap();
    let engine = Script(Mutex::new(VecDeque::from([
        "契約の更新日を確認します",
        "予算は来月決定します",
    ])));
    let mut transcript = Transcript::new();
    for chunk in &recovered.chunks {
        let segments = transcribe_chunk(chunk, &vad, &engine, Stage::Draft).unwrap();
        assert_eq!(segments.len(), 1, "チャンク {}", chunk.index);
        transcript.set_chunk(chunk.index, segments).unwrap();
    }
    transcript.rename_speaker("話者A", "田中"); // 未割り当てなので何も変わらない
    assert_eq!(transcript.segments().len(), 2);
    // 2 つ目の発話はチャンク 1（録音の 4 秒目以降）なので、絶対時刻になっている。
    assert!(
        transcript.segments()[1].start_ms >= 4_000,
        "{:?}",
        transcript.segments()[1]
    );

    // 議事録を検索索引へ。
    let docs = transcript.index_documents("meeting1").unwrap();
    let index = Bm25Index::build(Box::new(NgramTokenizer), docs);

    // 検索 → 引用（参照 id）→ 時刻 → 該当音声の位置。
    let hits = index.search("予算はいつ決まる", 3);
    assert!(!hits.is_empty());
    let (recording, start_ms) = parse_segment_ref(&hits[0].id).unwrap();
    assert_eq!(recording, "meeting1");
    let cited = transcript.segment_at(start_ms).unwrap();
    assert_eq!(cited.text, "予算は来月決定します");
    assert!(start_ms >= 4_000);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn refinement_updates_only_what_changed_and_keeps_speaker_names() {
    let audio = meeting_audio();
    let mut writer = ChunkWriter::new(RATE, CHUNK_SECONDS).unwrap();
    let chunks = writer.push(&audio);
    let vad = EnergyVad::new(VadConfig::default()).unwrap();

    let draft_engine = Script(Mutex::new(VecDeque::from([
        "けいやくの こうしんび",
        "よさんは らいげつ",
        "おわり",
    ])));
    let mut transcript = Transcript::new();
    for c in &chunks {
        let s = transcribe_chunk(c, &vad, &draft_engine, Stage::Draft).unwrap();
        transcript.set_chunk(c.index, s).unwrap();
    }
    assert!(transcript.set_speaker(0, Some("話者A".into())));
    transcript.rename_speaker("話者A", "田中");

    // 充電中に高精度モデルで再処理。1 つ目だけ文面が変わる。
    let refined_engine = Script(Mutex::new(VecDeque::from([
        "契約の更新日",
        "よさんは らいげつ",
        "おわり",
    ])));
    let mut total = 0;
    for c in &chunks {
        let refined = transcribe_chunk(c, &vad, &refined_engine, Stage::Refined).unwrap();
        let diff = transcript.apply_refined(c.index, refined).unwrap();
        total += diff.changed;
        assert_eq!(diff.added + diff.removed, 0);
    }
    assert_eq!(total, 1);
    assert_eq!(transcript.segments()[0].text, "契約の更新日");
    assert_eq!(
        transcript.segments()[0].speaker.as_deref(),
        Some("田中"),
        "話者名は引き継がれる"
    );
    assert!(transcript
        .segments()
        .iter()
        .all(|s| s.stage == Stage::Refined));
}
