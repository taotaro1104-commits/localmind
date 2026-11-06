use super::*;
use lm_ingest::{FsChunkStore, RecordingMeta};
use lm_llm::{GenerateError, Generator, LlmMinutesExtractor};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

struct Canned {
    reply: Result<String, GenerateError>,
    calls: Arc<AtomicU32>,
    cancel_flag: Arc<AtomicBool>,
}

impl Generator for Canned {
    fn generate(&self, _p: &str, _g: &str, _n: usize) -> Result<String, GenerateError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.reply.clone()
    }
    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
    fn id(&self) -> String {
        "canned".into()
    }
}

const REPLY: &str = r#"{"summary":[{"text":"見積書を送る","assignee":"","due":"","cited":[0]}],
"decisions":[{"text":"捏造","assignee":"","due":"","cited":[99999]}],
"action_items":[{"text":"見積書を送る","assignee":"田中","due":"来週金曜","cited":[0]}],
"open_issues":[]}"#;

fn engine(reply: Result<String, GenerateError>) -> (MinutesEngine, Arc<AtomicU32>) {
    let calls = Arc::new(AtomicU32::new(0));
    let g = Canned {
        reply,
        calls: calls.clone(),
        cancel_flag: Arc::new(AtomicBool::new(false)),
    };
    (
        MinutesEngine::from_extractor(LlmMinutesExtractor::new(Box::new(g))),
        calls,
    )
}

fn lines() -> Vec<SpokenLine> {
    vec![
        SpokenLine {
            start_ms: 0,
            end_ms: 2000,
            text: "来週の金曜までに見積書を送ります".into(),
        },
        SpokenLine {
            start_ms: 2000,
            end_ms: 4000,
            text: "予算は来期に決めます".into(),
        },
    ]
}

struct Dir(std::path::PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("lm-llm-ffi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
    fn root(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn extraction_returns_only_items_that_cite_a_real_utterance() {
    let (e, _) = engine(Ok(REPLY.into()));
    let r = e.extract(lines()).unwrap();
    assert_eq!(r.summary.len(), 1);
    assert!(r.decisions.is_empty(), "実在しない根拠の項目は載らない");
    assert_eq!(r.rejected, 1);
    assert_eq!(r.action_items[0].assignee.as_deref(), Some("田中"));
    assert_eq!(r.action_items[0].due.as_deref(), Some("来週金曜"));
    assert_eq!(r.model_id, "canned");
}

#[test]
fn an_empty_transcript_does_not_call_the_model() {
    let (e, calls) = engine(Ok(REPLY.into()));
    let r = e.extract(vec![]).unwrap();
    assert!(r.summary.is_empty() && r.rejected == 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn bad_transcripts_are_invalid_input_and_never_reach_the_model() {
    let (e, calls) = engine(Ok(REPLY.into()));
    let overlapping = vec![
        SpokenLine {
            start_ms: 0,
            end_ms: 3000,
            text: "a".into(),
        },
        SpokenLine {
            start_ms: 1000,
            end_ms: 2000,
            text: "b".into(),
        },
    ];
    let too_many: Vec<SpokenLine> = (0..5_001)
        .map(|i| SpokenLine {
            start_ms: i * 10,
            end_ms: i * 10 + 5,
            text: "あ".into(),
        })
        .collect();
    let too_long = vec![SpokenLine {
        start_ms: 0,
        end_ms: 1000,
        text: "あ".repeat(6_000),
    }];
    for bad in [overlapping, too_many, too_long] {
        assert!(matches!(e.extract(bad), Err(LlmError::InvalidInput { .. })));
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn model_failures_and_unreadable_output_are_engine_errors() {
    let (e, _) = engine(Err(GenerateError::Failed("メモリ不足".into())));
    assert!(matches!(e.extract(lines()), Err(LlmError::Engine { .. })));
    let (bad_json, _) = engine(Ok("途中で切れた {".into()));
    assert!(matches!(
        bad_json.extract(lines()),
        Err(LlmError::Engine { .. })
    ));
}

#[test]
fn extract_and_save_persists_minutes_that_load_back() {
    let d = Dir::new("save");
    let store = FsChunkStore::new(d.root());
    store
        .begin_recording(
            "r1",
            &RecordingMeta {
                consent_confirmed: true,
                sample_rate: 16_000,
                started_at_ms: 1,
            },
        )
        .unwrap();
    let (e, _) = engine(Ok(REPLY.into()));
    let r = e.extract_and_save(d.root(), "r1".into(), lines()).unwrap();
    let saved = store.load_minutes("r1").unwrap().expect("保存されている");
    assert_eq!(saved.action_items.len(), r.action_items.len());
    assert_eq!(saved.action_items[0].assignee.as_deref(), Some("田中"));
}

#[test]
fn saving_needs_an_existing_recording_and_a_safe_path() {
    let d = Dir::new("save-bad");
    let (e, calls) = engine(Ok(REPLY.into()));
    for (root, id) in [
        (d.root(), "nothing"),
        (d.root(), "../x"),
        ("relative/dir".to_string(), "r1"),
        (format!("{}/../x", d.root()), "r1"),
    ] {
        let err = e.extract_and_save(root, id.into(), lines()).unwrap_err();
        assert!(matches!(err, LlmError::InvalidInput { .. }), "{err:?}");
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "保存できない録音では、重い抽出をしない"
    );
    assert!(!d.0.join("nothing").exists(), "空のディレクトリを作らない");
}

#[test]
fn opening_checks_path_threads_and_hash_before_loading() {
    let missing = std::env::temp_dir().join("lm-llm-ffi-none").join("m.gguf");
    let missing = missing.to_string_lossy().into_owned();
    let zero = "0".repeat(64);
    let open = |p: &str, s: &str, t: u32| MinutesEngine::open(p.into(), s.into(), t).err();
    assert!(matches!(
        open("m.gguf", &zero, 2),
        Some(LlmError::InvalidInput { .. })
    ));
    for threads in [0, 9] {
        assert!(matches!(
            open(&missing, &zero, threads),
            Some(LlmError::InvalidInput { .. })
        ));
    }
    assert!(matches!(
        open(&missing, &zero, 2),
        Some(LlmError::Integrity { .. })
    ));
    assert!(matches!(
        open(&missing, "xyz", 2),
        Some(LlmError::InvalidInput { .. })
    ));
}

/// 実エンジンと同じ約束の偽物: 中止が立っていれば `Cancelled`。終わったら中止を戻す。
struct Stoppable {
    flag: Arc<AtomicBool>,
}

impl Generator for Stoppable {
    fn generate(&self, _p: &str, _g: &str, _n: usize) -> Result<String, GenerateError> {
        let stop = self.flag.swap(false, Ordering::Relaxed);
        if stop {
            Err(GenerateError::Cancelled)
        } else {
            Ok(REPLY.into())
        }
    }
    fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
    fn id(&self) -> String {
        "stoppable".into()
    }
}

#[test]
fn a_cancel_is_reported_as_cancelled_even_if_it_came_first_and_the_engine_is_reusable() {
    let flag = Arc::new(AtomicBool::new(false));
    let e = MinutesEngine::from_extractor(LlmMinutesExtractor::new(Box::new(Stoppable {
        flag: flag.clone(),
    })));
    e.cancel(); // 開始前の中止も効く
    assert!(matches!(e.extract(lines()), Err(LlmError::Cancelled)));
    let again = e.extract(lines()).expect("中止のあとも、もう一度使える");
    assert_eq!(again.model_id, "stoppable");
}

struct Fixed(GenerateError);

impl Generator for Fixed {
    fn generate(&self, _p: &str, _g: &str, _n: usize) -> Result<String, GenerateError> {
        Err(self.0.clone())
    }
    fn cancel(&self) {}
    fn id(&self) -> String {
        "fixed".into()
    }
}

fn fixed_engine(e: GenerateError) -> MinutesEngine {
    MinutesEngine::from_extractor(LlmMinutesExtractor::new(Box::new(Fixed(e))))
}

#[test]
fn too_long_for_the_model_is_an_input_error_with_a_way_out() {
    let err = fixed_engine(GenerateError::TooLong)
        .extract(lines())
        .unwrap_err();
    assert!(
        matches!(&err, LlmError::InvalidInput { msg } if msg.contains("短い区間")),
        "{err:?}"
    );
}

#[test]
fn a_real_failure_is_not_reported_as_a_cancel_even_if_cancel_was_requested() {
    let e = fixed_engine(GenerateError::Failed("メモリ不足".into()));
    e.cancel();
    assert!(matches!(e.extract(lines()), Err(LlmError::Engine { .. })));
}
