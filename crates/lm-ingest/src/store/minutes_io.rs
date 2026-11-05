//! 議事録の保存と読み込み（録音ディレクトリの `minutes.txt`）。

use super::format::corrupt;
use super::fsio::{ensure_real_dir, read_regular_limited, real_dir_exists, write_atomic};
use super::FsChunkStore;
use crate::error::IngestError;
use crate::minutes::{record, Minutes};

const MINUTES_FILE: &str = "minutes.txt";
/// 議事録ファイルの最大サイズ（1 項目 1000 文字 × 800 項目でも収まる余裕を見る）。
const MAX_MINUTES_BYTES: u64 = 4 * 1024 * 1024;

impl FsChunkStore {
    /// 議事録を保存する（原子的に置き換える。完了の切り替えもこれで保存する）。
    pub fn save_minutes(&self, id: &str, minutes: &Minutes) -> Result<(), IngestError> {
        let dir = self.dir(id)?;
        // 保存前に、読み込みと同じ検証を通す（手で組み立てた不正な議事録で、行の偽造や、
        // 後で読めないファイルを作らない）。
        let text = record::encode(minutes);
        if text.len() as u64 > MAX_MINUTES_BYTES {
            return Err(IngestError::InvalidFormat("議事録が大きすぎます".into()));
        }
        // 読み戻して元と一致することも確かめる（改行やタブで行が増える・列がずれるものを弾く）。
        if record::decode(&text)? != *minutes {
            return Err(IngestError::InvalidFormat(
                "議事録に保存できない文字が含まれています".into(),
            ));
        }
        ensure_real_dir(&dir)?;
        write_atomic(&dir.join(MINUTES_FILE), text.as_bytes())
    }

    /// 議事録を読む。まだ保存されていなければ `None`。形式が不正なら `Corrupt`。
    pub fn load_minutes(&self, id: &str) -> Result<Option<Minutes>, IngestError> {
        let dir = self.dir(id)?;
        if !real_dir_exists(&dir)? {
            return Ok(None);
        }
        let path = dir.join(MINUTES_FILE);
        // 無いのは「未保存」。リンク切れや調べられないものはエラーにする（未保存と混同しない）。
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let bytes = read_regular_limited(&path, MAX_MINUTES_BYTES)?;
        let text =
            String::from_utf8(bytes).map_err(|_| corrupt("議事録が UTF-8 ではありません"))?;
        record::decode(&text).map(Some)
    }
}
