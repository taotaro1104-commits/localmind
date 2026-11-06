//! GGUF ファイルの検証。固定した SHA-256 と一致しないファイルは読み込まない。
//!
//! モデルは 1 GB を超えるので、メモリに全体を載せず、読みながらハッシュを計算する。llama.cpp はパスからしか
//! 読み込めないので、検証から読み込みまでの間にファイルを差し替えられる余地が（whisper と違って）残る。
//! 置き場所はアプリ専用の領域（ほかのアプリが書けない）に限ること。

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

/// 受け付けるモデルの最大サイズ（バイト）。
pub const MAX_MODEL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// これより小さいファイルは、モデルではない（空・途中で切れた取得）として断る。
pub const MIN_MODEL_BYTES: u64 = 1024 * 1024;

/// モデルを検証できない理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// 期待するハッシュの書式が不正（呼び出しの誤り）。
    InvalidArgument(String),
    /// ファイルが無い・大きさが違う・内容が一致しない。モデルを入れ直せば直る。文言にパスは入れない。
    Integrity(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArgument(m) | Self::Integrity(m) => f.write_str(m),
        }
    }
}

/// 検証したファイルの大きさと更新時刻。読み込みの直前に照合して、検証後の差し替えを（完全ではないが）見つける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(meta: &std::fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }

    /// `path` が、検証したときと同じ大きさ・更新時刻の通常のファイルか。
    pub fn matches(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && Self::of(&m) == *self)
    }
}

fn integrity(msg: &str) -> ModelError {
    ModelError::Integrity(format!("モデルファイルを使えません: {msg}"))
}

/// `expected_sha256`（小文字の 16 進 64 文字）と内容が一致する通常のファイルだけを通す。
pub fn verify_file(path: &Path, expected_sha256: &str) -> Result<FileStamp, ModelError> {
    let valid = expected_sha256.len() == 64
        && expected_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !valid {
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
    let stamp = FileStamp::of(&meta);
    // 開いたファイルから、大きさの上限まで読む（差し替えで巨大になっても、読みすぎない）。
    let mut reader = file.take(MAX_MODEL_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|_| integrity("読めません"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    if total > MAX_MODEL_BYTES {
        return Err(integrity("大きさが想定の範囲外です"));
    }
    let after = reader.get_ref().metadata().map(|m| FileStamp::of(&m));
    if after.ok() != Some(stamp) {
        return Err(integrity("検証している間に変更されました"));
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual == expected_sha256 {
        Ok(stamp)
    } else {
        Err(integrity(
            "内容が固定したハッシュと一致しません（破損か差し替え）",
        ))
    }
}

#[cfg(test)]
mod tests;
