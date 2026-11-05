//! ハイブリッド検索（語彙 + ベクトル + RRF）と根拠スコア判定。

use std::sync::Arc;

use crate::index::{limit_query, DocSignals, LexicalIndex, SearchError, SemanticIndex, MAX_K};
use crate::rrf::{fuse, DEFAULT_K};
use crate::Hit;

/// 検索結果。根拠が閾値未満なら `NotFound`（推測せず「資料には見つかりません」と答える）。
///
/// `Found.hits` の `score` は RRF スコア（順位のみに由来し、BM25/cosine とは別物）。
#[derive(Debug, Clone, PartialEq)]
pub enum SearchOutcome {
    Found { hits: Vec<Hit>, grounding: f32 },
    NotFound { grounding: f32 },
}

/// 根拠スコアの既定閾値。`eval::sweep`（examples/threshold_sweep）で
/// balanced error が最小となった値（評価セット、bi-gram + ハッシュ埋め込み）。
/// 形態素解析・実埋め込みモデルへ差し替えたら再計測すること。
pub const DEFAULT_GROUNDING_THRESHOLD: f32 = 0.25;

/// `LinderaTokenizer` 使用時の閾値（同じ評価セットでの最適値）。
/// 助詞が除かれ語数が減るため、bi-gram 版より高い閾値が最適になる。
pub const LINDERA_GROUNDING_THRESHOLD: f32 = 0.30;

/// `OnnxEmbedder`（multilingual-e5-small）使用時の意味検索の下駄。
///
/// 下駄と閾値は連動する。評価セット（回答可能 29 問・回答不能 14 問）で、誤りゼロになる閾値の範囲を
/// 0.01 刻みで実測した結果（`examples/onnx_sweep`）:
///
/// | 下駄 | 誤りゼロの閾値 | 幅 |
/// |---|---|---|
/// | 0.74 | 0.37〜0.41 | 5 点 |
/// | 0.76 | 0.32〜0.36 | 5 点 |
/// | 0.78 | 0.26〜0.30 | 5 点 |
/// | 0.80 | 0.18〜0.23 | 6 点 |
/// | 0.82 | なし | — |
///
/// 中央の 0.78 を採る。閾値は範囲の端に置かない（0.25 以下で誤受理 7.1%、0.31 以上で取りこぼし 3.4%）。
/// 評価セットが小さい（43 問）ので、実際の質問・資料で測り直すこと。
pub const ONNX_E5_SEMANTIC_FLOOR: f32 = 0.78;

/// `OnnxEmbedder` 使用時の根拠スコア閾値（`ONNX_E5_SEMANTIC_FLOOR` とセットで使う）。
/// 下駄 0.78 での誤りゼロの範囲 0.26〜0.30 の中央（両側に 0.02 の余裕）。
pub const ONNX_E5_GROUNDING_THRESHOLD: f32 = 0.28;

/// 型番・条文番号などの識別子が最上位文書に無いときの根拠スコアの上限。
/// 「第99条」が「第12条」の文書に部分一致して受理されるのを防ぐ。
const MISSING_IDENTIFIER_CAP: f32 = 0.1;

#[derive(Debug, Clone, Copy)]
pub struct HybridConfig {
    /// 各検索器から取る候補数の下限（実際は `max(pool, k)`、`MAX_K` で頭打ち）。
    pub pool: usize,
    /// 根拠スコアの下限。調整は評価セット（G-6）で行う。NaN は拒否される。
    pub grounding_threshold: f32,
    pub rrf_k: f32,
    /// 意味検索の cosine から差し引く「下駄」（0.0 以上 1.0 未満）。
    /// ハッシュ埋め込みでは無関係な文の cosine はほぼ 0 だが、e5 などの実モデルは無関係でも
    /// 0.7〜0.85 に偏る。`(cosine - floor) / (1 - floor)` に引き直してから根拠スコアに使う。
    /// 0.0 なら cosine をそのまま使う。モデルごとに評価セットで測ること。
    pub semantic_floor: f32,
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            pool: 20,
            grounding_threshold: DEFAULT_GROUNDING_THRESHOLD,
            rrf_k: DEFAULT_K,
            semantic_floor: 0.0,
        }
    }
}

impl HybridConfig {
    /// `OnnxEmbedder`（multilingual-e5-small）向けの推奨設定。
    pub fn onnx_e5() -> Self {
        Self {
            grounding_threshold: ONNX_E5_GROUNDING_THRESHOLD,
            semantic_floor: ONNX_E5_SEMANTIC_FLOOR,
            ..Self::default()
        }
    }

    /// 設定値の検証。判定が常に通る（fail-open）ような非有限値を拒否する。
    pub fn validate(&self) -> Result<(), SearchError> {
        // NaN と -∞ は、どんな根拠でも受理してしまう（fail-open）ので拒否する。
        // +∞（常に「見つかりません」）は安全側なので許す。
        // 根拠スコアは 0.0 以上なので、負の閾値も -∞ と同じく何でも受理する。0.0 以上のみ許す。
        if self.grounding_threshold.is_nan() || self.grounding_threshold < 0.0 {
            return Err(SearchError::Config(format!(
                "grounding_threshold は 0.0 以上（+∞ は可）でなければなりません: {}",
                self.grounding_threshold
            )));
        }
        if !self.rrf_k.is_finite() || self.rrf_k < 0.0 {
            return Err(SearchError::Config(format!(
                "rrf_k は 0 以上の有限値でなければなりません: {}",
                self.rrf_k
            )));
        }
        if !(0.0..1.0).contains(&self.semantic_floor) {
            return Err(SearchError::Config(format!(
                "semantic_floor は 0.0 以上 1.0 未満でなければなりません: {}",
                self.semantic_floor
            )));
        }
        Ok(())
    }

    /// cosine を `semantic_floor` で引き直して 0.0〜1.0 にする。
    fn rescale(&self, cosine: f32) -> f32 {
        ((cosine - self.semantic_floor) / (1.0 - self.semantic_floor)).clamp(0.0, 1.0)
    }
}

pub struct HybridSearcher {
    lexical: Arc<dyn LexicalIndex>,
    semantic: Arc<dyn SemanticIndex>,
    config: HybridConfig,
}

/// 文書の突き合わせ結果から根拠値を求める。文書が無い（削除された）なら根拠なし（fail-closed）。
fn capped(raw: f32, signals: Option<DocSignals>) -> f32 {
    match signals {
        None => 0.0,
        Some(s) if s.missing_identifier => raw.min(MISSING_IDENTIFIER_CAP),
        Some(_) => raw,
    }
}

impl HybridSearcher {
    pub fn new(
        lexical: Arc<dyn LexicalIndex>,
        semantic: Arc<dyn SemanticIndex>,
        config: HybridConfig,
    ) -> Self {
        Self {
            lexical,
            semantic,
            config,
        }
    }

    /// 同じ索引を共有したまま、根拠スコアの閾値だけを変えた検索器を返す（閾値の走査用）。
    /// 閾値はここでは検証せず、`search` の時点で `validate` が拒否する（負・NaN は `Err(Config)`）。
    pub fn with_threshold(&self, grounding_threshold: f32) -> Self {
        Self {
            lexical: Arc::clone(&self.lexical),
            semantic: Arc::clone(&self.semantic),
            config: HybridConfig {
                grounding_threshold,
                ..self.config
            },
        }
    }

    /// 同じ索引を共有したまま、意味検索の下駄だけを変えた検索器を返す（下駄の走査用）。
    pub fn with_semantic_floor(&self, semantic_floor: f32) -> Self {
        Self {
            lexical: Arc::clone(&self.lexical),
            semantic: Arc::clone(&self.semantic),
            config: HybridConfig {
                semantic_floor,
                ..self.config
            },
        }
    }

    fn signals_for(
        &self,
        hit: Option<&Hit>,
        query: &str,
    ) -> Result<Option<DocSignals>, SearchError> {
        match hit {
            Some(h) => self.lexical.doc_signals(&h.id, query),
            None => Ok(None),
        }
    }

    /// 根拠スコア = max(語彙検索最上位のクエリ語カバー率, ベクトル最上位の cosine)。
    /// 最上位文書にクエリの識別子（数字を含む語）が無い場合は各値を上限で抑える。
    /// 語彙・ベクトルの最上位が同じ文書なら突き合わせは 1 回で済ませる。
    fn grounding(
        &self,
        query: &str,
        lexical: &[Hit],
        semantic: &[Hit],
    ) -> Result<f32, SearchError> {
        let (lex_top, sem_top) = (lexical.first(), semantic.first());
        let lex_sig = self.signals_for(lex_top, query)?;
        let sem_sig = match (lex_top, sem_top) {
            (Some(l), Some(s)) if l.id == s.id => lex_sig,
            _ => self.signals_for(sem_top, query)?,
        };
        let lex = capped(lex_sig.map_or(0.0, |s| s.coverage), lex_sig);
        let sem = capped(
            sem_top.map_or(0.0, |h| self.config.rescale(h.score)),
            sem_sig,
        );
        Ok(lex.max(sem))
    }

    /// 上位 `k` 件を検索する。`k` は 1 以上（0 は `Config` エラー）で、`MAX_K` に丸められる。
    pub fn search(&self, query: &str, k: usize) -> Result<SearchOutcome, SearchError> {
        Ok(self.search_with_cosine(query, k)?.0)
    }

    /// [`HybridSearcher::search`] に加えて、意味検索の上位（`score` は cosine）も返す。結果の足切りなど、
    /// 融合前の cosine が要る呼び出し側が、同じ質問を埋め込み直さずに済む。
    pub fn search_with_cosine(
        &self,
        query: &str,
        k: usize,
    ) -> Result<(SearchOutcome, Vec<Hit>), SearchError> {
        self.config.validate()?;
        if k == 0 {
            return Err(SearchError::Config(
                "k は 1 以上でなければなりません".into(),
            ));
        }
        let query = limit_query(query);
        let k = k.min(MAX_K);
        let pool = self.config.pool.max(k).clamp(1, MAX_K);
        let lexical = self.lexical.search(query, pool)?;
        let semantic = self.semantic.search(query, pool)?;
        let grounding = self.grounding(query, &lexical, &semantic)?;

        // 閾値以上のときだけ受理する。比較できない値（NaN）は拒否側に倒す。
        let accepted = matches!(
            grounding.partial_cmp(&self.config.grounding_threshold),
            Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
        );
        if !accepted {
            return Ok((SearchOutcome::NotFound { grounding }, semantic));
        }
        let mut hits = fuse(&[lexical, semantic.clone()], self.config.rrf_k);
        hits.truncate(k);
        Ok((SearchOutcome::Found { hits, grounding }, semantic))
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_floor;
