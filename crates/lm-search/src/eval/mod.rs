//! 検索品質の評価（G-6）: Recall@k、MRR、「見つかりません」の誤判定率。

pub mod dataset;

use std::sync::Arc;

use crate::{
    Bm25Index, Embedder, HashingEmbedder, HybridConfig, HybridSearcher, NgramTokenizer,
    SearchOutcome, Tokenizer, VectorIndex,
};
pub use dataset::{EvalCase, CASES, CORPUS};

/// 評価結果。割合は 0.0〜1.0。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    pub threshold: f32,
    /// 回答可能な質問のうち、上位 k 件に正解が入った割合。
    pub recall_at_k: f32,
    /// 回答可能な質問の MRR（正解が見つからなければ 0）。
    pub mrr: f32,
    /// 回答可能なのに「見つかりません」とした割合（取りこぼし）。
    pub false_reject_rate: f32,
    /// 回答不能なのに何か返した割合（幻覚の元）。
    pub false_accept_rate: f32,
}

impl Report {
    /// 閾値選定用の総合スコア: 取りこぼしと誤受理を同じ重みで減点（Balanced error）。
    pub fn balanced_error(&self) -> f32 {
        (self.false_reject_rate + self.false_accept_rate) / 2.0
    }
}

pub fn build_searcher(threshold: f32) -> HybridSearcher {
    build_searcher_with(Box::new(NgramTokenizer), threshold)
}

/// 語彙検索のトークナイザを指定して構築する（ベクトル側は常にハッシュ埋め込み）。
pub fn build_searcher_with(tokenizer: Box<dyn Tokenizer>, threshold: f32) -> HybridSearcher {
    build_searcher_custom(
        tokenizer,
        Box::new(HashingEmbedder::new(512)),
        HybridConfig {
            grounding_threshold: threshold,
            ..HybridConfig::default()
        },
    )
}

/// トークナイザ・埋め込み・設定を全て指定して、評価コーパスのインメモリ検索器を作る。
/// 評価・調整用の API で、構築に失敗すると panic する（`Result` を返さない）。
pub fn build_searcher_custom(
    tokenizer: Box<dyn Tokenizer>,
    embedder: Box<dyn Embedder>,
    config: HybridConfig,
) -> HybridSearcher {
    HybridSearcher::new(
        Arc::new(Bm25Index::build(tokenizer, CORPUS.iter().copied())),
        Arc::new(VectorIndex::build(embedder, CORPUS.iter().copied()).expect("ベクトル索引の構築")),
        config,
    )
}

/// SQLite（FTS5 + sqlite-vec）に評価コーパスを索引して検索器を作る。
#[cfg(feature = "sqlite")]
pub fn build_sqlite_searcher(tokenizer: Box<dyn Tokenizer>, threshold: f32) -> HybridSearcher {
    let store = crate::SqliteStore::in_memory(tokenizer, Box::new(HashingEmbedder::new(512)))
        .expect("SQLite 初期化");
    for (id, text) in CORPUS {
        store.upsert(id, text).expect("索引化");
    }
    let store = Arc::new(store);
    HybridSearcher::new(
        store.clone(),
        store,
        HybridConfig {
            grounding_threshold: threshold,
            ..HybridConfig::default()
        },
    )
}

/// 既定構成（インメモリ・bi-gram）で評価する。
pub fn evaluate(threshold: f32, k: usize) -> Report {
    evaluate_searcher(&build_searcher(threshold), threshold, k)
}

/// 任意の検索器を評価セットで評価する。`threshold` は報告用の表示値。
pub fn evaluate_searcher(searcher: &HybridSearcher, threshold: f32, k: usize) -> Report {
    let (mut answerable, mut hit, mut rr_sum, mut rejected) = (0usize, 0usize, 0.0f32, 0usize);
    let (mut unanswerable, mut accepted) = (0usize, 0usize);

    for case in CASES {
        let outcome = searcher.search(case.query, k).expect("評価中の検索に失敗");
        match (case.expected, outcome) {
            (Some(want), SearchOutcome::Found { hits, .. }) => {
                answerable += 1;
                if let Some(pos) = hits.iter().position(|h| h.id == want) {
                    hit += 1;
                    rr_sum += 1.0 / (pos as f32 + 1.0);
                }
            }
            (Some(_), SearchOutcome::NotFound { .. }) => {
                answerable += 1;
                rejected += 1;
            }
            (None, SearchOutcome::Found { .. }) => {
                unanswerable += 1;
                accepted += 1;
            }
            (None, SearchOutcome::NotFound { .. }) => unanswerable += 1,
        }
    }
    let ratio = |n: usize, d: usize| if d == 0 { 0.0 } else { n as f32 / d as f32 };
    Report {
        threshold,
        recall_at_k: ratio(hit, answerable),
        mrr: if answerable == 0 {
            0.0
        } else {
            rr_sum / answerable as f32
        },
        false_reject_rate: ratio(rejected, answerable),
        false_accept_rate: ratio(accepted, unanswerable),
    }
}

/// 閾値を 0.00〜1.00 まで 0.05 刻みで走査し、各閾値の評価結果を返す（既定構成）。
pub fn sweep(k: usize) -> Vec<Report> {
    sweep_with(&build_searcher(0.0), k)
}

/// 構築済みの検索器の索引を共有したまま、閾値を 0.05 刻みで走査する（索引は再構築しない）。
pub fn sweep_with(base: &HybridSearcher, k: usize) -> Vec<Report> {
    sweep_steps(base, k, 0.05)
}

/// 0.0〜1.0 を `step` 刻みにしたときの区間数。刻みが 0・負・極小でも 1000 区間までに抑える。
fn step_count(step: f32) -> usize {
    (1.0 / clamp_step(step)).round() as usize
}

/// 刻みを 0.001 以上に補正する（0・負・NaN は 0.001）。点の数と閾値の計算で同じ値を使う。
fn clamp_step(step: f32) -> f32 {
    if step.is_nan() {
        0.001
    } else {
        step.max(0.001)
    }
}

/// 閾値を 0.0〜1.0 の範囲で `step` 刻みに走査する。許容幅（誤りゼロの連続区間）を細かく見るために使う。
pub fn sweep_steps(base: &HybridSearcher, k: usize, step: f32) -> Vec<Report> {
    thresholds(step)
        .into_iter()
        .map(|thr| evaluate_searcher(&base.with_threshold(thr), thr, k))
        .collect()
}

/// `sweep_steps` が評価する閾値の列。0.0 から昇順で、最後は必ず 1.0。
/// 刻みが粗くて 1.0 に届かない場合（0.3 刻みなど）も、最後の点を 1.0 にそろえる。
fn thresholds(step: f32) -> Vec<f32> {
    let step = clamp_step(step);
    let count = step_count(step).max(1);
    (0..=count)
        .map(|i| {
            if i == 0 {
                0.0
            } else if i == count {
                1.0
            } else {
                (i as f32 * step).min(1.0)
            }
        })
        .collect()
}

/// balanced error が最小の報告（同値なら閾値が小さい方）。
pub fn best(reports: &[Report]) -> Report {
    *reports
        .iter()
        .min_by(|a, b| a.balanced_error().total_cmp(&b.balanced_error()))
        .expect("走査結果が空です")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grounding::DEFAULT_GROUNDING_THRESHOLD;

    /// 最適閾値との balanced error の許容差（評価セットは小さく、1 問で約 0.02 動く）。
    const OPTIMUM_TOLERANCE: f32 = 0.03;

    #[test]
    fn dataset_is_well_formed() {
        for case in CASES {
            if let Some(id) = case.expected {
                assert!(CORPUS.iter().any(|(cid, _)| *cid == id), "unknown id {id}");
            }
        }
        assert!(CASES.iter().any(|c| c.expected.is_none()));
        assert!(CASES.iter().any(|c| c.expected.is_some()));
    }

    /// 品質の回帰ガード: 既定閾値での性能が下がったら失敗する。
    #[test]
    fn default_threshold_meets_quality_bar() {
        let r = evaluate(DEFAULT_GROUNDING_THRESHOLD, 3);
        assert!(r.recall_at_k >= 0.95, "{r:?}");
        assert!(r.mrr >= 0.95, "{r:?}");
        assert_eq!(r.false_accept_rate, 0.0, "{r:?}");
        assert!(r.false_reject_rate <= 0.05, "{r:?}");
    }

    /// 評価セットで選んだ閾値を同じ評価セットで検証するため循環しやすい。
    /// データ追加で僅かに最適点が動いても落ちないよう、許容幅つきで「最適に近い」ことだけを見る。
    #[test]
    fn default_threshold_is_near_the_sweep_optimum() {
        let best = best(&sweep(3)).balanced_error();
        let at_default = evaluate(DEFAULT_GROUNDING_THRESHOLD, 3).balanced_error();
        assert!(
            at_default <= best + OPTIMUM_TOLERANCE,
            "default={at_default} best={best}"
        );
    }

    #[test]
    fn extreme_thresholds_behave() {
        assert_eq!(evaluate(0.0, 3).false_reject_rate, 0.0);
        assert!(evaluate(0.0, 3).false_accept_rate > 0.9);
        assert_eq!(evaluate(1.1, 3).false_accept_rate, 0.0);
        assert!(evaluate(1.1, 3).false_reject_rate > 0.99);
    }

    #[cfg(feature = "lindera")]
    #[test]
    fn lindera_meets_quality_bar_at_its_threshold() {
        use crate::grounding::LINDERA_GROUNDING_THRESHOLD;
        let tok = || Box::new(crate::LinderaTokenizer::new().unwrap());
        let r = evaluate_searcher(
            &build_searcher_with(tok(), LINDERA_GROUNDING_THRESHOLD),
            LINDERA_GROUNDING_THRESHOLD,
            3,
        );
        assert!(r.recall_at_k >= 0.95, "{r:?}");
        assert_eq!(r.false_accept_rate, 0.0, "{r:?}");
        assert!(r.false_reject_rate <= 0.05, "{r:?}");
        let best = best(&sweep_with(&build_searcher_with(tok(), 0.0), 3)).balanced_error();
        assert!(
            r.balanced_error() <= best + OPTIMUM_TOLERANCE,
            "{r:?} best={best}"
        );
    }
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    #[test]
    fn default_sweep_has_twenty_one_points_and_fine_sweep_has_a_hundred_and_one() {
        let base = build_searcher(0.0);
        let coarse = sweep_with(&base, 3);
        assert_eq!(coarse.len(), 21);
        assert_eq!(coarse[0].threshold, 0.0);
        assert!((coarse[20].threshold - 1.0).abs() < 1e-6);

        let fine = sweep_steps(&base, 3, 0.01);
        assert_eq!(fine.len(), 101);
        assert!((fine[25].threshold - 0.25).abs() < 1e-6);
        assert!((fine[100].threshold - 1.0).abs() < 1e-6);
    }

    #[test]
    fn step_counts_are_bounded_even_for_degenerate_steps() {
        assert_eq!(step_count(0.05), 20);
        assert_eq!(step_count(0.01), 100);
        assert_eq!(step_count(0.0), 1000);
        assert_eq!(step_count(-1.0), 1000);
        assert_eq!(step_count(f32::NAN), 1000);
        assert_eq!(step_count(5.0), 0);
    }

    #[test]
    fn a_coarse_step_still_ends_at_one() {
        let base = build_searcher(0.0);
        let r = sweep_steps(&base, 3, 0.5);
        let thresholds: Vec<f32> = r.iter().map(|x| x.threshold).collect();
        assert_eq!(thresholds, vec![0.0, 0.5, 1.0]);
    }

    /// 刻みが 1 を割り切らない・異常な場合も、閾値は 0.0 から昇順に並び、最後が 1.0 になる（評価は回さない）。
    #[test]
    fn odd_steps_produce_ascending_thresholds_ending_at_one() {
        for step in [0.3, 0.35, 0.7, 0.9, 5.0, 0.0, -1.0, f32::NAN, f32::INFINITY] {
            let t = thresholds(step);
            assert_eq!(t[0], 0.0, "step {step}");
            assert_eq!(*t.last().unwrap(), 1.0, "step {step}");
            assert!(t.windows(2).all(|w| w[0] < w[1]), "step {step}: {t:?}");
            assert!(t.len() <= 1001, "step {step}");
        }
        assert_eq!(thresholds(0.5), [0.0, 0.5, 1.0]);
    }
}
