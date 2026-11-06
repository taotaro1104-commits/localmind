use super::*;
use lm_ingest::minutes::extract_minutes;
use lm_ingest::{Stage, Transcript, TranscriptSegment};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

struct Canned {
    reply: Result<String, GenerateError>,
    seen: Mutex<Vec<(String, String, usize)>>,
    cancelled: AtomicBool,
}

impl Canned {
    fn new(reply: Result<String, GenerateError>) -> Self {
        Self {
            reply,
            seen: Mutex::new(vec![]),
            cancelled: AtomicBool::new(false),
        }
    }
}

impl Generator for &'static Canned {
    fn generate(&self, p: &str, g: &str, n: usize) -> Result<String, GenerateError> {
        self.seen
            .lock()
            .unwrap()
            .push((p.to_string(), g.to_string(), n));
        self.reply.clone()
    }
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    fn reset_cancel(&self) {
        self.cancelled.store(false, Ordering::Relaxed);
    }
    fn id(&self) -> String {
        "canned".into()
    }
}

fn leak(c: Canned) -> &'static Canned {
    Box::leak(Box::new(c))
}

fn seg(start: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        chunk_index: 0,
        start_ms: start,
        end_ms: start + 1000,
        speaker: None,
        text: text.into(),
        confidence: 0.9,
        stage: Stage::Draft,
    }
}

fn transcript() -> Transcript {
    let mut t = Transcript::new();
    t.set_chunk(
        0,
        vec![
            seg(0, "来週の金曜までに見積書を送ります"),
            seg(2000, "予算は来期に決めましょう"),
        ],
    )
    .unwrap();
    t
}

const REPLY: &str = r#"{"summary":[{"text":"見積書を送る","assignee":"","due":"","cited":[0]}],
"decisions":[{"text":"捏造","assignee":"","due":"","cited":[99999]}],
"action_items":[{"text":"見積書を送る","assignee":"田中","due":"来週金曜","cited":[0]}],
"open_issues":[]}"#;

#[test]
fn the_extractor_sends_the_prompt_and_grammar_and_parses_the_reply() {
    let c = leak(Canned::new(Ok(REPLY.into())));
    let ex = LlmMinutesExtractor::new(Box::new(c));
    let raw = ex.extract("[0] こんにちは\n").unwrap();
    assert_eq!(raw.action_items[0].assignee.as_deref(), Some("田中"));
    let seen = c.seen.lock().unwrap();
    assert!(seen[0].0.contains("[0] こんにちは"));
    assert_eq!(seen[0].1, MINUTES_GRAMMAR);
    assert_eq!(seen[0].2, MAX_NEW_TOKENS);
    assert_eq!(ex.id(), "canned");
}

#[test]
fn invented_citations_are_dropped_by_the_existing_validation() {
    let c = leak(Canned::new(Ok(REPLY.into())));
    let ex = LlmMinutesExtractor::new(Box::new(c));
    let m = extract_minutes(&transcript(), &ex, MAX_INPUT_CHARS).unwrap();
    assert_eq!(m.summary.len(), 1);
    assert_eq!(m.action_items.len(), 1);
    assert!(m.decisions.is_empty(), "実在しない根拠の項目は載らない");
    assert_eq!(m.rejected, 1);
}

#[test]
fn generator_failures_become_extract_errors() {
    for (reply, needle) in [
        (Err(GenerateError::TooLong), "長すぎ"),
        (Err(GenerateError::Cancelled), "中止"),
        (
            Err(GenerateError::Failed("メモリ不足".into())),
            "メモリ不足",
        ),
        (Ok("途中で切れた {".into()), "読めません"),
    ] {
        let ex = LlmMinutesExtractor::new(Box::new(leak(Canned::new(reply))));
        let err = ex.extract("[0] a\n").unwrap_err();
        assert!(
            matches!(&err, IngestError::Extract(m) if m.contains(needle)),
            "{err:?}"
        );
    }
}

#[test]
fn cancel_reaches_the_generator() {
    let c = leak(Canned::new(Ok(REPLY.into())));
    LlmMinutesExtractor::new(Box::new(c)).cancel();
    assert!(c.cancelled.load(Ordering::Relaxed));
}

#[test]
fn reset_cancel_clears_a_pending_cancel() {
    let c = leak(Canned::new(Ok(REPLY.into())));
    let ex = LlmMinutesExtractor::new(Box::new(c));
    ex.cancel();
    ex.reset_cancel();
    assert!(!c.cancelled.load(Ordering::Relaxed));
}
