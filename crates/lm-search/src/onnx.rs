//! ONNX 埋め込みモデル（multilingual-e5-small, 384 次元）。feature `onnx` で有効。
//!
//! - モデルは初回に取得して `cache_dir` に保存し、以後はオフラインで動く（初回のみネットワークが必要）。
//! - e5 は入力の書式が決まっている: クエリは `query: `、文書は `passage: ` を先頭に付ける。
//!   `Embedder::embed_query` / `embed_document` で振り分け、索引側と検索側で取り違えない。
//! - 取得元はリビジョンとファイルごとの SHA-256 で固定する（`onnx_pin`）。ハッシュが一致したファイルだけを
//!   メモリから読み込むので、ミラーの偽装や保存先での差し替えがあっても、別のモデルは使われない。
//!   `HF_*` の環境変数は参照せず、保存先は引数 `cache_dir` のみ（プロキシの環境変数は尊重する）。
//!   ネットワーク失敗・ハッシュ不一致は `SearchError::Embedding` になる。
//! - e5 は約 512 トークンまでしか読まない。長い文書は呼び出し側でセグメントに分けて登録すること
//!   （議事録は発話セグメントごと、文書は段落ごと）。巨大な文字列のトークン化に時間を使わないよう、
//!   入力は `MAX_INPUT_CHARS` 文字で切り詰める。
//! - e5 の cosine 類似度は無関係な文でも高い値（目安 0.7 以上）に偏る。根拠スコアに使うときは
//!   `HybridConfig::semantic_floor` で下駄を外すこと。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use fastembed::{
    InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};

use crate::index::SearchError;
use crate::onnx_pin::{
    bundled_path, load, read_verified, PinnedFile, CONFIG_FILE, MODEL_FILE, SPECIAL_TOKENS_FILE,
    TOKENIZER_CONFIG_FILE, TOKENIZER_FILE,
};
use crate::vector::Embedder;

/// multilingual-e5-small の埋め込み次元。
pub const E5_SMALL_DIM: usize = 384;
/// 1 回の埋め込みに渡す最大文字数。
pub const MAX_INPUT_CHARS: usize = 2048;

const QUERY_PREFIX: &str = "query: ";
const PASSAGE_PREFIX: &str = "passage: ";

/// 接頭辞を付け、`MAX_INPUT_CHARS` 文字までに切り詰める（char 境界を保つ）。
fn prepare_input(prefix: &str, text: &str) -> String {
    let body: String = text.chars().take(MAX_INPUT_CHARS).collect();
    format!("{prefix}{body}")
}

pub struct OnnxEmbedder {
    model: Mutex<TextEmbedding>,
}

impl OnnxEmbedder {
    /// multilingual-e5-small を読み込む。`cache_dir` に無ければ取得する（ネットワークが必要）。
    pub fn multilingual_e5_small(cache_dir: impl Into<PathBuf>) -> Result<Self, SearchError> {
        let cache_dir = cache_dir.into();
        Self::from_loader(|file: &PinnedFile| load(&cache_dir, file))
    }

    /// アプリに同梱したモデルを読み込む（ネットワークは使わない）。`dir` に、リポジトリ内のパスの区切りを
    /// `__` にした名前（`onnx__model.onnx`, `tokenizer.json`, ...）で 5 ファイルを置く。
    /// 各ファイルは固定した SHA-256 と一致しなければ読み込まない（展開先での破損・差し替えを防ぐ）。
    pub fn from_bundled_dir(dir: impl AsRef<Path>) -> Result<Self, SearchError> {
        let dir = dir.as_ref();
        Self::from_loader(|file: &PinnedFile| read_verified(&bundled_path(dir, file), file))
    }

    fn from_loader(
        load: impl Fn(&PinnedFile) -> Result<Vec<u8>, SearchError>,
    ) -> Result<Self, SearchError> {
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: load(&TOKENIZER_FILE)?,
            config_file: load(&CONFIG_FILE)?,
            special_tokens_map_file: load(&SPECIAL_TOKENS_FILE)?,
            tokenizer_config_file: load(&TOKENIZER_CONFIG_FILE)?,
        };
        // e5 は平均プーリング。検証済みのバイト列だけを ONNX Runtime に渡す。
        let definition = UserDefinedEmbeddingModel::new(load(&MODEL_FILE)?, tokenizer_files)
            .with_pooling(Pooling::Mean);
        let model =
            TextEmbedding::try_new_from_user_defined(definition, InitOptionsUserDefined::new())
                .map_err(|e| SearchError::Embedding(format!("モデルを読み込めません: {e}")))?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// モデルのロック。中毒していても復旧する（推論は呼び出しごとに独立している）。
    fn lock(&self) -> MutexGuard<'_, TextEmbedding> {
        self.model.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn run(&self, prefix: &str, text: &str) -> Result<Vec<f32>, SearchError> {
        let input = prepare_input(prefix, text);
        let mut out = self
            .lock()
            .embed([input], None)
            .map_err(|e| SearchError::Embedding(format!("推論に失敗しました: {e}")))?;
        let v = out
            .pop()
            .ok_or_else(|| SearchError::Embedding("推論結果が空です".into()))?;
        if v.len() == E5_SMALL_DIM {
            Ok(v)
        } else {
            Err(SearchError::Embedding(format!(
                "埋め込み次元が想定外です: {} （期待 {E5_SMALL_DIM}）",
                v.len()
            )))
        }
    }
}

impl Embedder for OnnxEmbedder {
    /// 接頭辞の区別がない呼び出しは、文書として扱う。
    fn embed(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.run(PASSAGE_PREFIX, text)
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.run(QUERY_PREFIX, text)
    }

    fn embed_document(&self, text: &str) -> Result<Vec<f32>, SearchError> {
        self.run(PASSAGE_PREFIX, text)
    }

    fn dim(&self) -> usize {
        E5_SMALL_DIM
    }

    fn id(&self) -> String {
        // モデルのリビジョン（`onnx_pin::REVISION`）を更新するときは、この id も更新する
        // （同じ id のまま別のモデルのベクトルが既存の索引に混ざるのを防ぐ）。
        "fastembed-multilingual-e5-small-v1".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_get_the_e5_prefixes() {
        assert_eq!(
            prepare_input(QUERY_PREFIX, "契約の更新日"),
            "query: 契約の更新日"
        );
        assert_eq!(
            prepare_input(PASSAGE_PREFIX, "更新日は三月"),
            "passage: 更新日は三月"
        );
        assert_eq!(prepare_input(QUERY_PREFIX, ""), "query: ");
    }

    #[test]
    fn long_inputs_are_truncated_on_a_char_boundary() {
        let long = "あ".repeat(MAX_INPUT_CHARS * 5);
        let s = prepare_input(PASSAGE_PREFIX, &long);
        assert_eq!(s.chars().count(), PASSAGE_PREFIX.len() + MAX_INPUT_CHARS);
        assert!(s.starts_with(PASSAGE_PREFIX));
    }
}
