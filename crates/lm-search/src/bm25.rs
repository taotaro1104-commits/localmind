//! BM25 全文検索（インメモリ）。永続版は `sqlite::SqliteStore`（FTS5）。

use std::collections::HashMap;

use crate::index::{query_terms, signals, DocSignals};
use crate::tokenize::Tokenizer;
use crate::Hit;

const K1: f32 = 1.2;
const B: f32 = 0.75;

#[derive(Debug)]
struct Entry {
    id: String,
    len: usize,
    tf: HashMap<String, u32>,
}

/// 構築後は不変の BM25 索引。
pub struct Bm25Index {
    tokenizer: Box<dyn Tokenizer>,
    entries: Vec<Entry>,
    df: HashMap<String, u32>,
    avg_len: f32,
}

impl Bm25Index {
    /// `(id, text)` の列から索引を構築する。同じ id が複数あれば最後のものが残る。
    pub fn build<I, S, T>(tokenizer: Box<dyn Tokenizer>, docs: I) -> Self
    where
        I: IntoIterator<Item = (S, T)>,
        S: Into<String>,
        T: AsRef<str>,
    {
        // 重複 id は後勝ち（SQLite 版の upsert と同じ意味）。順序は初出位置を保つ。
        let mut order: Vec<String> = Vec::new();
        let mut latest: HashMap<String, String> = HashMap::new();
        for (id, text) in docs {
            let id = id.into();
            if latest
                .insert(id.clone(), text.as_ref().to_string())
                .is_none()
            {
                order.push(id);
            }
        }

        let mut entries = Vec::with_capacity(order.len());
        let mut df: HashMap<String, u32> = HashMap::new();
        for id in order {
            let tokens = tokenizer.tokenize(&latest[&id]);
            let mut tf: HashMap<String, u32> = HashMap::new();
            for t in &tokens {
                *tf.entry(t.clone()).or_insert(0) += 1;
            }
            for term in tf.keys() {
                *df.entry(term.clone()).or_insert(0) += 1;
            }
            entries.push(Entry {
                id,
                len: tokens.len(),
                tf,
            });
        }
        let total: usize = entries.iter().map(|e| e.len).sum();
        let avg_len = if entries.is_empty() {
            0.0
        } else {
            total as f32 / entries.len() as f32
        };
        Self {
            tokenizer,
            entries,
            df,
            avg_len,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn idf(&self, term: &str) -> f32 {
        let n = self.entries.len() as f32;
        let df = self.df.get(term).copied().unwrap_or(0) as f32;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    fn query_terms(&self, query: &str) -> Vec<String> {
        query_terms(self.tokenizer.as_ref(), query)
    }

    /// スコア降順で上位 `k` 件（スコア 0 は除外）。同点は id 昇順。
    pub fn search(&self, query: &str, k: usize) -> Vec<Hit> {
        let terms = self.query_terms(query);
        if terms.is_empty() || k == 0 {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .entries
            .iter()
            .filter_map(|e| {
                let norm = 1.0 - B + B * (e.len as f32 / self.avg_len.max(1.0));
                let score: f32 = terms
                    .iter()
                    .filter_map(|t| {
                        let tf = *e.tf.get(t)? as f32;
                        Some(self.idf(t) * tf * (K1 + 1.0) / (tf + K1 * norm))
                    })
                    .sum();
                (score > 0.0).then(|| Hit {
                    id: e.id.clone(),
                    score,
                })
            })
            .collect();
        Hit::sort_desc(&mut hits);
        hits.truncate(k);
        hits
    }

    /// 文書 `id` とクエリの突き合わせ（カバー率・識別子の不足）。文書が無ければ None。
    pub fn doc_signals(&self, id: &str, query: &str) -> Option<DocSignals> {
        let entry = self.entries.iter().find(|e| e.id == id)?;
        Some(signals(&self.query_terms(query), |t| {
            entry.tf.contains_key(t)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::NgramTokenizer;

    fn index() -> Bm25Index {
        Bm25Index::build(
            Box::new(NgramTokenizer),
            [
                ("a", "契約の更新日は来年三月です。"),
                ("b", "型番 AB-1234 の取扱説明書"),
                ("c", "第12条 秘密保持義務について"),
                ("d", "今日の天気は晴れでした"),
            ],
        )
    }

    #[test]
    fn finds_japanese_phrase() {
        let hits = index().search("契約更新日", 3);
        assert_eq!(hits[0].id, "a");
    }

    #[test]
    fn finds_model_number_exactly_even_full_width() {
        let hits = index().search("ＡＢ－１２３４", 3);
        assert_eq!(hits[0].id, "b");
    }

    #[test]
    fn finds_article_number_in_kanji_or_arabic() {
        assert_eq!(index().search("第12条", 3)[0].id, "c");
        assert_eq!(index().search("第十二条", 3)[0].id, "c");
    }

    #[test]
    fn unrelated_query_returns_nothing() {
        assert!(index().search("zzzz", 3).is_empty());
    }

    #[test]
    fn k_limits_results_and_empty_query_is_empty() {
        let idx = Bm25Index::build(
            Box::new(NgramTokenizer),
            [
                ("x", "契約書の原本"),
                ("y", "契約の更新"),
                ("z", "天気予報"),
            ],
        );
        assert!(idx.search("", 3).is_empty());
        assert!(idx.search("契約", 0).is_empty());
        // 「契約」は x と y に出る。k で切り詰められる。
        assert_eq!(idx.search("契約", 5).len(), 2);
        assert_eq!(idx.search("契約", 1).len(), 1);
    }

    #[test]
    fn doc_signals_report_coverage_and_missing_identifier() {
        let idx = index();
        let s = idx.doc_signals("b", "ab-1234").unwrap();
        assert!((s.coverage - 1.0).abs() < 1e-6);
        assert!(!s.missing_identifier);

        let s = idx.doc_signals("b", "ab-1234 zzzz").unwrap();
        assert!((s.coverage - 0.5).abs() < 1e-6);

        assert!(idx.doc_signals("c", "第99条").unwrap().missing_identifier);
        assert!(idx.doc_signals("c", "第十三条").unwrap().missing_identifier);
        assert!(!idx.doc_signals("c", "第十二条").unwrap().missing_identifier);
        assert!(!idx.doc_signals("a", "契約更新").unwrap().missing_identifier);
        assert!(idx.doc_signals("missing", "第99条").is_none());
    }

    #[test]
    fn duplicate_ids_keep_the_last_text() {
        let idx = Bm25Index::build(
            Box::new(NgramTokenizer),
            [("x", "りんごとみかん"), ("x", "ぶどうとすいか")],
        );
        assert_eq!(idx.len(), 1);
        assert!(idx.search("りんご", 3).is_empty());
        assert_eq!(idx.search("ぶどう", 3).len(), 1);
    }

    #[test]
    fn empty_index_is_safe() {
        let idx = Bm25Index::build(Box::new(NgramTokenizer), Vec::<(String, String)>::new());
        assert!(idx.is_empty());
        assert!(idx.search("x", 3).is_empty());
    }
}
