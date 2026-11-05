//! e5 埋め込みの cosine 分布を評価セットで測る。モデルは初回に取得する（ネットワークが必要）。
//! 使い方: cargo run --release -p lm-search --features onnx --example onnx_probe -- <モデルのキャッシュ先>
use lm_search::eval::{CASES, CORPUS};
use lm_search::{OnnxEmbedder, VectorIndex};

fn stats(label: &str, v: &[f32]) {
    if v.is_empty() {
        return;
    }
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let min = v.iter().cloned().fold(f32::MAX, f32::min);
    let max = v.iter().cloned().fold(f32::MIN, f32::max);
    println!(
        "{label:<28} n={:<3} min={min:.3} mean={mean:.3} max={max:.3}",
        v.len()
    );
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("モデルのキャッシュ先を指定してください");
    let embedder = OnnxEmbedder::multilingual_e5_small(dir).expect("モデルの読み込み");
    let index = VectorIndex::build(Box::new(embedder), CORPUS.iter().copied()).expect("索引");

    let (mut answerable, mut unanswerable) = (Vec::new(), Vec::new());
    let (mut top1_ok, mut total) = (0usize, 0usize);
    for case in CASES {
        let hits = index.search(case.query, 1).expect("検索");
        let (id, score) = hits.first().map_or(("", 0.0), |h| (h.id.as_str(), h.score));
        match case.expected {
            Some(want) => {
                answerable.push(score);
                total += 1;
                top1_ok += usize::from(id == want);
            }
            None => unanswerable.push(score),
        }
    }
    stats("answerable top1 cosine", &answerable);
    stats("unanswerable top1 cosine", &unanswerable);
    println!("top-1 accuracy (answerable): {top1_ok}/{total}");
}
