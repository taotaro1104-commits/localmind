//! モデルファイルの検証つき読み込み。固定した SHA-256 と一致しないファイルは読み込まない
//! （差し替え・破損・取得の失敗を弾く）。
//!
//! ファイルは 1 回だけ開き、そこから読んだバイト列そのものを検証して返す。検証したあとで同じパスを開き直さない
//! ので、検証と読み込みの間にファイルを差し替えても、検証していない内容は読み込まれない（呼び出し側は、このバイト列
//! から whisper.cpp を初期化する）。シンボリックリンクを辿っても、読んだ内容のハッシュが一致しなければ使えない。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

/// 受け付けるモデルの最大サイズ（バイト）。メモリに全体を載せるので、small〜medium 級までに限る
/// （検証用のバッファとモデル本体で、一時的に 2 倍近くのメモリを使う）。
pub const MAX_MODEL_BYTES: u64 = 1024 * 1024 * 1024;
/// これより小さいファイルは、モデルではない（空・途中で切れた取得）として断る。
pub const MIN_MODEL_BYTES: u64 = 1024 * 1024;

/// モデルを読み込めない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// 呼び出しの誤り（期待するハッシュの書式が不正）。モデルを入れ直しても直らない。
    InvalidArgument(String),
    /// ファイルが無い・大きさが違う・内容が固定したハッシュと一致しない（破損・差し替え・取得の失敗）。
    /// モデルを入れ直せば直る。文言にパスは入れない。
    Integrity(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(m) | Self::Integrity(m) => f.write_str(m),
        }
    }
}

fn integrity(msg: &str) -> ModelError {
    ModelError::Integrity(format!("モデルファイルを使えません: {msg}"))
}

/// `expected_sha256`（小文字の 16 進 64 文字）と一致するファイルの内容を返す。
/// 通常のファイルで、大きさが [`MIN_MODEL_BYTES`]〜[`MAX_MODEL_BYTES`] のものだけを受け付ける
/// （読む量は、開いたファイルから上限まで。巨大なファイルへ差し替えられても、メモリを使い切らない）。
pub fn load_verified(path: &Path, expected_sha256: &str) -> Result<Vec<u8>, ModelError> {
    let valid_hash = expected_sha256.len() == 64
        && expected_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid_hash {
        return Err(ModelError::InvalidArgument(
            "期待するハッシュは小文字の 16 進 64 文字で指定してください".into(),
        ));
    }
    let file = File::open(path).map_err(|_| integrity("開けません"))?;
    let meta = file.metadata().map_err(|_| integrity("調べられません"))?;
    if !meta.is_file() {
        return Err(integrity("通常のファイルではありません"));
    }
    if !(MIN_MODEL_BYTES..=MAX_MODEL_BYTES).contains(&meta.len()) {
        return Err(integrity("大きさが想定の範囲外です"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_MODEL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| integrity("読めません"))?;
    if bytes.len() as u64 > MAX_MODEL_BYTES {
        return Err(integrity("大きさが想定の範囲外です"));
    }
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual == expected_sha256 {
        Ok(bytes)
    } else {
        Err(integrity(
            "内容が固定したハッシュと一致しません（破損か差し替え）",
        ))
    }
}

#[cfg(test)]
mod tests;
