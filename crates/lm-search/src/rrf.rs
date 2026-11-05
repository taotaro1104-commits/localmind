//! Reciprocal Rank Fusion。

use std::collections::HashMap;

use crate::Hit;

/// 標準的な RRF 定数。
pub const DEFAULT_K: f32 = 60.0;

/// 複数のランキングを RRF で統合する。スコア降順、同点は id 昇順。
pub fn fuse(rankings: &[Vec<Hit>], k: f32) -> Vec<Hit> {
    let mut scores: HashMap<&str, f32> = HashMap::new();
    for ranking in rankings {
        for (rank, hit) in ranking.iter().enumerate() {
            *scores.entry(hit.id.as_str()).or_insert(0.0) += 1.0 / (k + rank as f32 + 1.0);
        }
    }
    let mut fused: Vec<Hit> = scores
        .into_iter()
        .map(|(id, score)| Hit {
            id: id.to_string(),
            score,
        })
        .collect();
    Hit::sort_desc(&mut fused);
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(id: &str) -> Hit {
        Hit {
            id: id.into(),
            score: 0.0,
        }
    }

    #[test]
    fn doc_in_both_lists_beats_single_list_docs() {
        let a = vec![h("x"), h("y")];
        let b = vec![h("z"), h("y")];
        let fused = fuse(&[a, b], DEFAULT_K);
        assert_eq!(fused[0].id, "y");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn score_formula() {
        let fused = fuse(&[vec![h("a")]], 60.0);
        assert!((fused[0].score - 1.0 / 61.0).abs() < 1e-7);
    }

    #[test]
    fn ties_break_by_id() {
        let fused = fuse(&[vec![h("b")], vec![h("a")]], DEFAULT_K);
        assert_eq!(fused[0].id, "a");
    }

    #[test]
    fn empty_input() {
        assert!(fuse(&[], DEFAULT_K).is_empty());
        assert!(fuse(&[vec![], vec![]], DEFAULT_K).is_empty());
    }
}
