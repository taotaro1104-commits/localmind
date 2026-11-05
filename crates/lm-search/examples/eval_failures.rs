use lm_search::eval::{build_searcher_with, CASES};
use lm_search::SearchOutcome;
use lm_search::{NgramTokenizer, Tokenizer};

fn main() {
    let thr: f32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.25);
    let kind = std::env::args().nth(2).unwrap_or_else(|| "ngram".into());
    let tok: Box<dyn Tokenizer> = match kind.as_str() {
        #[cfg(feature = "lindera")]
        "lindera" => Box::new(lm_search::LinderaTokenizer::new().expect("Lindera 初期化")),
        _ => Box::new(NgramTokenizer),
    };
    let s = build_searcher_with(tok, thr);
    for c in CASES {
        let out = s.search(c.query, 3).unwrap();
        let (g, top) = match &out {
            SearchOutcome::Found { hits, grounding } => {
                (*grounding, hits.first().map(|h| h.id.clone()))
            }
            SearchOutcome::NotFound { grounding } => (*grounding, None),
        };
        let ok = match (c.expected, &top) {
            (Some(w), Some(t)) => w == t,
            (None, None) => true,
            _ => false,
        };
        if !ok {
            println!(
                "NG g={g:.2} q={:?} want={:?} got={:?}",
                c.query, c.expected, top
            );
        }
    }
}
