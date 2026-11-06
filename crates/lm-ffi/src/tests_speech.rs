use super::*;
use lm_ingest::{IngestError, Transcriber, Utterance};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const NOW: u64 = 1_800_000_000_000;
const RATE: u32 = 16_000;

/// 聞こえた音の長さで答える偽のエンジン（モデル無しで、パイプラインの結びつきを確かめる）。
struct Fake {
    calls: AtomicU32,
    fail: bool,
    /// 呼ばれたら、この旗を立ててエラーを返す（実行中に中止されたことを再現する）。
    cancel_on_call: Option<Arc<AtomicBool>>,
}

impl Fake {
    fn ok() -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail: false,
            cancel_on_call: None,
        }
    }
}

impl Transcriber for Fake {
    fn transcribe(&self, pcm: &[i16], _rate: u32) -> Result<Utterance, IngestError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(flag) = &self.cancel_on_call {
            flag.store(true, Ordering::Relaxed);
            return Err(IngestError::Transcribe("中止されました".into()));
        }
        if self.fail {
            return Err(IngestError::Transcribe("失敗".into()));
        }
        Ok(Utterance {
            text: format!("発話{}秒", pcm.len() / RATE as usize),
            confidence: 0.9,
        })
    }
    fn id(&self) -> String {
        "fake".into()
    }
}

fn engine(fake: Fake, refined: bool) -> (SpeechEngine, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    (
        SpeechEngine::from_transcriber(Box::new(fake), flag.clone(), refined),
        flag,
    )
}

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lm-ffi-speech-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
    fn store(&self) -> Arc<RecordingStore> {
        RecordingStore::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 3 秒ごとに 1 秒の「声」（大きな音）がある、`seconds` 秒の録音。
fn speechy(seconds: usize) -> Vec<i16> {
    (0..RATE as usize * seconds)
        .map(|i| {
            if (i / RATE as usize) % 3 == 1 {
                ((i % 100) as i16 - 50) * 300
            } else {
                0
            }
        })
        .collect()
}

fn record(store: &Arc<RecordingStore>, id: &str, seconds: usize) {
    let rec = store.begin(id.into(), true, RATE, NOW).unwrap();
    rec.push(speechy(seconds)).unwrap();
    rec.finish().unwrap();
}

struct Watcher {
    seen: Mutex<Vec<(u32, u32)>>,
    stop_after: u32,
}

impl SpeechProgress for Watcher {
    fn on_progress(&self, done: u32, total: u32) -> bool {
        self.seen.lock().unwrap().push((done, total));
        done < self.stop_after
    }
}

#[test]
fn a_saved_recording_is_transcribed_with_times_from_the_start_of_the_recording() {
    let d = Dir::new("basic");
    let store = d.store();
    record(&store, "r1", 40); // 30 秒 + 10 秒の 2 チャンク
    let (e, _) = engine(Fake::ok(), false);
    let w = Arc::new(Watcher {
        seen: Mutex::new(vec![]),
        stop_after: 99,
    });
    let r = e
        .transcribe_recording(store, "r1".into(), Some(w.clone()))
        .unwrap();
    assert!(!r.segments.is_empty() && !r.cancelled && r.skipped_chunks == 0);
    assert_eq!(*w.seen.lock().unwrap(), [(0, 2), (1, 2), (2, 2)]);
    assert!(r
        .segments
        .windows(2)
        .all(|p| p[0].start_ms <= p[1].start_ms));
    assert!(r.segments.iter().any(|s| s.start_ms >= 30_000));
    assert!(r
        .segments
        .iter()
        .all(|s| s.end_ms > s.start_ms && s.text.starts_with("発話")));
    assert_eq!((r.model_id.as_str(), r.refined), ("fake", false));
}

#[test]
fn accurate_engines_report_a_refined_result() {
    let d = Dir::new("refined");
    let store = d.store();
    record(&store, "r1", 10);
    let (e, _) = engine(Fake::ok(), true);
    let r = e.transcribe_recording(store, "r1".into(), None).unwrap();
    assert!(r.refined);
}

#[test]
fn returning_false_from_progress_stops_and_keeps_the_results_so_far() {
    let d = Dir::new("progress-cancel");
    let store = d.store();
    record(&store, "r1", 100); // 4 チャンク
    let (e, _) = engine(Fake::ok(), false);
    let w = Arc::new(Watcher {
        seen: Mutex::new(vec![]),
        stop_after: 2,
    });
    let r = e
        .transcribe_recording(store, "r1".into(), Some(w.clone()))
        .unwrap();
    assert!(r.cancelled);
    assert_eq!(w.seen.lock().unwrap().len(), 3);
    assert!(r.segments.iter().all(|s| s.start_ms < 60_000));
}

#[test]
fn false_at_the_very_start_does_no_work() {
    let d = Dir::new("start-cancel");
    let store = d.store();
    record(&store, "r1", 40);
    let (e, _) = engine(Fake::ok(), false);
    let w = Arc::new(Watcher {
        seen: Mutex::new(vec![]),
        stop_after: 0,
    });
    let r = e.transcribe_recording(store, "r1".into(), Some(w)).unwrap();
    assert!(r.cancelled && r.segments.is_empty());
}

#[test]
fn cancelling_during_inference_is_a_cancellation_not_an_error() {
    let d = Dir::new("midrun");
    let store = d.store();
    record(&store, "r1", 40);
    let flag = Arc::new(AtomicBool::new(false));
    let fake = Fake {
        calls: AtomicU32::new(0),
        fail: false,
        cancel_on_call: Some(flag.clone()),
    };
    let e = SpeechEngine::from_transcriber(Box::new(fake), flag, false);
    let r = e.transcribe_recording(store, "r1".into(), None).unwrap();
    assert!(r.cancelled);
}

#[test]
fn the_cancel_instruction_is_cleared_when_the_next_run_starts() {
    let d = Dir::new("rerun");
    let store = d.store();
    record(&store, "r1", 10);
    let (e, flag) = engine(Fake::ok(), false);
    e.cancel();
    assert!(flag.load(Ordering::Relaxed));
    let r = e.transcribe_recording(store, "r1".into(), None).unwrap();
    assert!(!r.cancelled && !r.segments.is_empty());
}

#[test]
fn a_missing_recording_is_empty_and_an_engine_failure_is_an_error() {
    let d = Dir::new("errors");
    let store = d.store();
    let (ok, _) = engine(Fake::ok(), false);
    assert!(ok
        .transcribe_recording(store.clone(), "none".into(), None)
        .unwrap()
        .segments
        .is_empty());
    assert!(ok
        .transcribe_recording(store.clone(), "../x".into(), None)
        .is_err());
    record(&store, "r1", 10);
    let (broken, _) = engine(
        Fake {
            calls: AtomicU32::new(0),
            fail: true,
            cancel_on_call: None,
        },
        false,
    );
    let err = broken
        .transcribe_recording(store, "r1".into(), None)
        .unwrap_err();
    assert!(matches!(err, FfiError::Ingest { .. }), "{err:?}");
}

#[test]
fn the_model_path_threads_and_hash_are_checked_before_loading() {
    let open = |path: &str, sha: &str, threads: u32| {
        SpeechEngine::open(
            path.into(),
            sha.into(),
            "ja".into(),
            SpeechQuality::Fast,
            threads,
        )
        .err()
    };
    let zero = "0".repeat(64);
    assert!(open("model.bin", &zero, 2).is_some());
    let missing = std::env::temp_dir()
        .join("lm-ffi-no-such-dir")
        .join("ggml.bin");
    let missing = missing.to_string_lossy().into_owned();
    for threads in [0, 9, 100] {
        assert!(matches!(
            open(&missing, &zero, threads),
            Some(FfiError::Ingest { .. })
        ));
    }
    assert!(matches!(
        open(&missing, &zero, 2),
        Some(FfiError::Integrity { .. })
    ));
    assert!(matches!(
        open(&missing, "xyz", 2),
        Some(FfiError::Ingest { .. })
    ));
}
