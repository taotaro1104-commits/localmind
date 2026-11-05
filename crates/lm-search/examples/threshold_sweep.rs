//! 使い方: cargo run -p lm-search --example threshold_sweep --features lindera,sqlite -- <kind>
//! kind: ngram | lindera | sqlite | sqlite-lindera
use lm_search::eval::{build_searcher_with, sweep_with, Report};
use lm_search::{NgramTokenizer, Tokenizer};

fn searcher(kind: &str, t: f32) -> lm_search::HybridSearcher {
    match kind {
        #[cfg(feature = "sqlite")]
        "sqlite" => lm_search::eval::build_sqlite_searcher(tokenizer("ngram"), t),
        #[cfg(all(feature = "sqlite", feature = "lindera"))]
        "sqlite-lindera" => lm_search::eval::build_sqlite_searcher(tokenizer("lindera"), t),
        k => build_searcher_with(tokenizer(k), t),
    }
}

fn tokenizer(kind: &str) -> Box<dyn Tokenizer> {
    match kind {
        #[cfg(feature = "lindera")]
        "lindera" => Box::new(lm_search::LinderaTokenizer::new().expect("Lindera 初期化")),
        _ => Box::new(NgramTokenizer),
    }
}

fn print(rows: &[Report]) {
    println!("thr   recall@3  mrr    false_reject  false_accept  balanced_err");
    for r in rows {
        println!(
            "{:.2}  {:>8.3}  {:.3}  {:>12.3}  {:>12.3}  {:>12.3}",
            r.threshold,
            r.recall_at_k,
            r.mrr,
            r.false_reject_rate,
            r.false_accept_rate,
            r.balanced_error()
        );
    }
}

fn main() {
    let kind = std::env::args().nth(1).unwrap_or_else(|| "ngram".into());
    println!("tokenizer = {kind}");
    print(&sweep_with(&searcher(&kind, 0.0), 3));
}
