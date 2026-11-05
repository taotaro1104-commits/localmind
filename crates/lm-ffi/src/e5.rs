//! 同梱した multilingual-e5-small（ONNX）で動く検索エンジン。feature `onnx` で有効。
//!
//! 推論と tokenizer は Rust 側で行う（Kotlin に tokenizer を持ち込まない）。モデルはアプリに同梱し、
//! 起動時に展開先ディレクトリから読む。各ファイルは固定した SHA-256 と一致しなければ読まない。

use std::path::{Component, Path};
use std::sync::Arc;

use crate::error::FfiError;
use crate::search::{build_engine, check_abs_path, SearchConfig, SearchEngine};
use crate::workspace::{open_with, WorkspaceEngine};
use lm_search::{LinderaTokenizer, OnnxEmbedder, SqliteStore};
use lm_workspace::E5_RELEVANCE_MARGIN;

/// 実行時に読む ONNX Runtime のファイル名（AAR が `nativeLibraryDir` に置くもの）。
const ORT_LIBRARY_NAME: &str = "libonnxruntime.so";

/// `..` を含まない絶対パスの `libonnxruntime.so`（通常のファイル）か。
/// 別名のライブラリや相対パスを読み込ませないための最低限の確認。置き場所の信頼は、呼び出し側が
/// `nativeLibraryDir`（root 所有・読み取り専用）のパスだけを渡すことで担保する。
pub(crate) fn is_expected_ort_library(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && path.file_name().is_some_and(|n| n == ORT_LIBRARY_NAME)
        && path.is_file()
}

/// Android では ONNX Runtime を実行時に読む。パスの指定が無い・不適切だと、ライブラリ側が
/// パニックで失敗するので、その前に分かりやすいエラーにする。
fn check_ort_library() -> Result<(), FfiError> {
    if cfg!(target_os = "android") {
        let ok = std::env::var_os("ORT_DYLIB_PATH")
            .map(|p| is_expected_ort_library(Path::new(&p)))
            .unwrap_or(false);
        if !ok {
            return Err(FfiError::Embedding {
                msg: "ONNX Runtime のライブラリが見つかりません（環境変数 ORT_DYLIB_PATH に \
                      libonnxruntime.so の絶対パスを設定してください）"
                    .into(),
            });
        }
    }
    Ok(())
}

#[uniffi::export]
impl SearchEngine {
    /// 同梱モデルで動く永続索引を開く。`model_dir` はモデル 5 ファイルの展開先、`path` は索引ファイル。
    /// 読み込みには時間がかかる（モデルのハッシュ検証と初期化。エミュレータで約 20 秒、端末による）。
    /// UI スレッドから呼ばないこと。モデルの読み込みで約 1GB のメモリを使うので、同時に 2 つ開かないこと。
    #[uniffi::constructor]
    pub fn open_e5(
        path: String,
        model_dir: String,
        config: SearchConfig,
    ) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&path, "索引のパス")?;
        check_abs_path(&model_dir, "モデルのディレクトリ")?;
        config.validate()?;
        check_ort_library()?;
        // ONNX Runtime の読み込み失敗はパニックになりうるので、FFI の外へ出さずエラーにする。
        let embedder = std::panic::catch_unwind(|| OnnxEmbedder::from_bundled_dir(&model_dir))
            .map_err(|_| FfiError::Embedding {
            msg: "ONNX Runtime の初期化に失敗しました".into(),
        })??;
        let store =
            SqliteStore::open(path, Box::new(LinderaTokenizer::new()?), Box::new(embedder))?;
        Ok(build_engine(store, config))
    }
}

#[uniffi::export]
impl WorkspaceEngine {
    /// 同梱モデル（e5）で動くワークスペースを開く。`model_dir` はモデルの展開先、`path` は索引、
    /// `docs_dir` は文書の原本の保存先。読み込みに時間がかかるので UI スレッドから呼ばないこと
    /// （`SearchEngine::open_e5` と同じ。同時に 2 つ開かないこと）。
    #[uniffi::constructor]
    pub fn open_e5(
        path: String,
        docs_dir: String,
        model_dir: String,
        config: SearchConfig,
    ) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&model_dir, "モデルのディレクトリ")?;
        check_abs_path(&path, "索引のパス")?;
        check_abs_path(&docs_dir, "文書の保存先")?;
        config.validate()?;
        check_ort_library()?;
        let embedder = std::panic::catch_unwind(|| OnnxEmbedder::from_bundled_dir(&model_dir))
            .map_err(|_| FfiError::Embedding {
            msg: "ONNX Runtime の初期化に失敗しました".into(),
        })??;
        // e5 で測った幅で、資料ごとの足切り（関連の薄い結果を落とす）を有効にする。
        open_with(
            path,
            docs_dir,
            config,
            Box::new(embedder),
            E5_RELEVANCE_MARGIN,
        )
    }
}
