//! 実際の GGUF モデルでの確認（`--ignored`）。
//!
//! 実行: LM_LLM_MODEL=<gguf> LM_LLM_SHA256=<sha256> cargo test -p lm-llm --features llama --release \
//!       --test real_model -- --ignored --nocapture --test-threads=1
#![cfg(feature = "llama")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use lm_ingest::minutes::extract_minutes;
use lm_ingest::{Stage, Transcript, TranscriptSegment};
use lm_llm::llama::{LlamaConfig, LlamaGenerator};
use lm_llm::{GenerateError, Generator, LlmMinutesExtractor, MAX_INPUT_CHARS};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("環境変数 {name} を設定してください"))
}

fn generator() -> LlamaGenerator {
    LlamaGenerator::open(
        &PathBuf::from(env("LM_LLM_MODEL")),
        &env("LM_LLM_SHA256"),
        LlamaConfig::default(),
    )
    .expect("モデルを開けません")
}

fn seg(start: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        chunk_index: 0,
        start_ms: start,
        end_ms: start + 2000,
        speaker: None,
        text: text.into(),
        confidence: 0.9,
        stage: Stage::Draft,
    }
}

fn meeting() -> Transcript {
    let mut t = Transcript::new();
    t.set_chunk(
        0,
        vec![
            seg(0, "来週の金曜日までに田中さんが見積書を送ります。"),
            seg(2000, "来期の予算は営業部の増員を含めて承認されました。"),
            seg(
                4000,
                "納期についてはまだ決まっていないので、次回に持ち越します。",
            ),
            seg(6000, "以上で定例会議を終わります。"),
        ],
    )
    .unwrap();
    t
}

#[test]
#[ignore = "実モデルが要る"]
fn a_real_model_produces_validated_minutes_with_real_citations() {
    let ex = LlmMinutesExtractor::new(Box::new(generator()));
    let started = Instant::now();
    let m = extract_minutes(&meeting(), &ex, MAX_INPUT_CHARS).expect("抽出");
    println!("{:.1} 秒\n{m:#?}", started.elapsed().as_secs_f32());
    let known = [0, 2000, 4000, 6000];
    let all: Vec<&Vec<u64>> = m
        .summary
        .iter()
        .map(|i| &i.citations)
        .chain(m.decisions.iter().map(|i| &i.citations))
        .chain(m.action_items.iter().map(|i| &i.citations))
        .chain(m.open_issues.iter().map(|i| &i.citations))
        .collect();
    assert!(!all.is_empty(), "何かしら抽出される");
    assert!(all.iter().all(|c| c.iter().all(|x| known.contains(x))));
}

#[test]
#[ignore = "実モデルが要る"]
fn cancelling_stops_generation_and_the_engine_is_reusable() {
    let g = Arc::new(generator());
    let worker = g.clone();
    let handle = std::thread::spawn(move || {
        let ex = LlmMinutesExtractor::new(Box::new(ArcGen(worker)));
        extract_minutes(&meeting(), &ex, MAX_INPUT_CHARS)
    });
    std::thread::sleep(std::time::Duration::from_secs(3));
    g.cancel();
    let started = Instant::now();
    let result = handle.join().unwrap();
    println!("中止から {:.1} 秒で戻った", started.elapsed().as_secs_f32());
    assert!(result.is_err(), "中止は、エラーで返る");
    // 続けて使える（中止の指示は、次の生成の始めに解除される）。
    let again = g
        .generate(
            "<|im_start|>user\nこんにちは<|im_end|>\n<|im_start|>assistant\n",
            "root ::= \"ok\"",
            8,
        )
        .expect("再利用できる");
    assert_eq!(again, "ok");
}

#[test]
#[ignore = "実モデルが要る"]
fn an_over_long_prompt_is_refused_up_front() {
    let g = generator();
    let long = "あ".repeat(40_000);
    assert_eq!(
        g.generate(&long, "root ::= \"x\"", 8),
        Err(GenerateError::TooLong)
    );
}

/// `Arc` 越しに `Generator` を使うための薄い包み。
struct ArcGen(Arc<LlamaGenerator>);

impl Generator for ArcGen {
    fn generate(&self, p: &str, g: &str, n: usize) -> Result<String, GenerateError> {
        self.0.generate(p, g, n)
    }
    fn cancel(&self) {
        self.0.cancel();
    }
    fn id(&self) -> String {
        self.0.id()
    }
}
