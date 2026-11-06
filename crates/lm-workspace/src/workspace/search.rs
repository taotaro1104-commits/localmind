//! 検索（グループでの絞り込みと、関連の弱い命中の足切り）。

use super::*;

impl Workspace {
    /// 検索する。結果は近い順で、各件に出典（文書・見出し・行番号・抜粋）がつく。
    /// `k` は [`MAX_RESULTS`] に丸める。
    pub fn search(&self, query: &str, k: usize) -> Result<Report, WorkspaceError> {
        self.search_in(query, k, None)
    }

    /// [`Workspace::search`] を、グループで絞る（`Some("")` は未分類だけ。`None` は絞らない）。
    ///
    /// 「見つかったか」と足切りは、すべての資料での検索の結果で決める。そこから、指定したグループの資料の命中だけを残す。
    /// そのため、絞ると命中が減る・無くなることはあっても、全体で見つからない質問が、絞ったことで見つかることはない。
    /// 足切り（`with_relevance_margin`）で落とされる命中は、絞っても戻らない（ほかのグループの資料のほうがずっと関連が
    /// 強いときに、指定したグループの弱い命中は出ない）。ただし、語彙の一致だけで拾われた命中（cosine が分からないもの）は、
    /// 絞らないときと同じく足切りの対象外。`grounding` は、絞る前の全体の値。
    pub fn search_in(
        &self,
        query: &str,
        k: usize,
        group: Option<&str>,
    ) -> Result<Report, WorkspaceError> {
        let group = group.map(check_group).transpose()?;
        let k = k.min(MAX_RESULTS);
        // 絞る前の候補は、多めに取る
        let fetch = if group.is_some() { MAX_RESULTS } else { k };
        let (outcome, semantic) = self.searcher.search_with_cosine(query, fetch)?;
        let (hits, grounding) = match outcome {
            SearchOutcome::Found { hits, grounding } => (hits, grounding),
            SearchOutcome::NotFound { grounding } => {
                return Ok(Report {
                    found: false,
                    grounding,
                    citations: Vec::new(),
                })
            }
        };
        let hits = self.cut_weak_hits(query, hits, &semantic);
        // 同じ文書の命中が複数あっても、原本の読み込みと分割は 1 回で済ませる。
        let mut cache: HashMap<String, Option<(StoredDoc, Vec<Chunk>)>> = HashMap::new();
        let mut citations = Vec::with_capacity(hits.len());
        for hit in hits {
            let Some((doc_id, seq)) = parse_chunk_id(&hit.id) else {
                continue;
            };
            // 検索のあいだに文書が消された・壊れたときは、その件だけ出さない。
            let loaded = cache.entry(doc_id.to_string()).or_insert_with(|| {
                let doc = self.docs.load(doc_id).ok()?;
                let chunks = split(&doc.text, doc.format).ok()?;
                Some((doc, chunks))
            });
            let Some((doc, chunks)) = loaded else {
                continue;
            };
            if group.as_ref().is_some_and(|g| *g != doc.group) {
                continue;
            }
            let Some(chunk) = chunks.iter().find(|c| c.seq == seq) else {
                continue;
            };
            citations.push(Citation {
                doc_id: doc.id.clone(),
                title: doc.title.clone(),
                heading_path: chunk.heading_path.clone(),
                start_line: chunk.start_line,
                end_line: chunk.end_line,
                excerpt: excerpt(&chunk.text),
                score: hit.score,
            });
        }
        citations.truncate(k);
        Ok(Report {
            found: !citations.is_empty(),
            grounding,
            citations,
        })
    }

    /// 1 位との cosine の差が [`Workspace::with_relevance_margin`] を超える結果を落とす（1 位は必ず残す）。
    /// 次の場合は落とさない: 足切りが無効、結果が 1 件以下、質問に数字を含む（型番・条文番号などの語彙一致が
    /// 強い質問は、cosine だけでは関連の強さを測れない）、cosine が分からない結果（意味検索の上位に無く、
    /// 語彙の一致だけで拾われたもの）。
    fn cut_weak_hits(&self, query: &str, hits: Vec<Hit>, semantic: &[Hit]) -> Vec<Hit> {
        let margin = self.relevance_margin;
        if hits.len() < 2 || margin.is_infinite() || query.chars().any(char::is_numeric) {
            return hits;
        }
        let cosine: HashMap<&str, f32> =
            semantic.iter().map(|h| (h.id.as_str(), h.score)).collect();
        let Some(&top) = cosine.get(hits[0].id.as_str()) else {
            return hits;
        };
        hits.into_iter()
            .enumerate()
            .filter(|(i, h)| *i == 0 || cosine.get(h.id.as_str()).is_none_or(|c| top - c <= margin))
            .map(|(_, h)| h)
            .collect()
    }
}
