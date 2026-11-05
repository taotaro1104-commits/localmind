//! ONNX 埋め込み（e5）での、意味検索の下駄 × 根拠スコア閾値の掃引。モデルは初回に取得する。
//! 使い方: cargo run --release -p lm-search --features onnx --example onnx_sweep -- <モデルのキャッシュ先> [lindera]
use lm_search::eval::{best, build_searcher_custom, sweep_steps, sweep_with};
use lm_search::grounding::ONNX_E5_SEMANTIC_FLOOR;
use lm_search::{HybridConfig, NgramTokenizer, OnnxEmbedder, Tokenizer};

fn tokenizer(kind: &str) -> Box<dyn Tokenizer> {
    match kind {
        #[cfg(feature = "lindera")]
        "lindera" => Box::new(lm_search::LinderaTokenizer::new().expect("Lindera 初期化")),
        _ => Box::new(NgramTokenizer),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("モデルのキャッシュ先を指定してください");
    let kind = args.next().unwrap_or_else(|| "ngram".into());
    let embedder = OnnxEmbedder::multilingual_e5_small(dir).expect("モデルの読み込み");
    let base = build_searcher_custom(
        tokenizer(&kind),
        Box::new(embedder),
        HybridConfig::default(),
    );

    println!("tokenizer={kind}");
    println!("-- 下駄ごとの「誤りゼロ」の閾値の範囲（0.01 刻み）--");
    println!("floor  zero_error_range  width");
    for floor in [0.70f32, 0.74, 0.76, 0.78, 0.80, 0.82] {
        let reports = sweep_steps(&base.with_semantic_floor(floor), 3, 0.01);
        let zero: Vec<f32> = reports
            .iter()
            .filter(|r| r.balanced_error() == 0.0)
            .map(|r| r.threshold)
            .collect();
        match (zero.first(), zero.last()) {
            (Some(lo), Some(hi)) => {
                println!("{floor:.2}   {lo:.2}〜{hi:.2}         {} 点", zero.len())
            }
            _ => println!("{floor:.2}   なし"),
        }
    }
    println!("-- 採用値 floor={ONNX_E5_SEMANTIC_FLOOR} の閾値ごと（0.01 刻み、0.20〜0.45）--");
    println!("thr   false_reject  false_accept  balanced_err");
    for r in sweep_steps(&base.with_semantic_floor(ONNX_E5_SEMANTIC_FLOOR), 3, 0.01)
        .iter()
        .filter(|r| (0.20..=0.45).contains(&r.threshold))
    {
        println!(
            "{:.2}  {:>12.3}  {:>12.3}  {:>12.3}",
            r.threshold,
            r.false_reject_rate,
            r.false_accept_rate,
            r.balanced_error()
        );
    }
    println!("-- 下駄ごとの最良閾値 --");
    println!("floor  best_thr  recall@3  false_reject  false_accept  balanced_err");
    for floor in [0.0f32, 0.70, 0.74, 0.76, 0.78, 0.80, 0.82, 0.84] {
        let r = best(&sweep_with(&base.with_semantic_floor(floor), 3));
        println!(
            "{floor:.2}   {:>7.2}  {:>8.3}  {:>12.3}  {:>12.3}  {:>12.3}",
            r.threshold,
            r.recall_at_k,
            r.false_reject_rate,
            r.false_accept_rate,
            r.balanced_error()
        );
    }
}
