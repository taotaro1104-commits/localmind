//! 保存済みの録音の一覧（録音画面の一覧と、途中で止まった録音の発見に使う）。
//!
//! 一覧は録音ディレクトリの名前とファイルの数・大きさだけを見る。チャンクの中身（音声）は読まないので、
//! 録音が多くても軽い。録音の長さは、チャンクの数（標準は 1 つ 30 秒）からおおよそが分かる。

use std::fs;

use super::scan::scan_dir;
use super::{validate_recording_id, FsChunkStore, IngestError, RecordingMeta, MAX_SCAN_ENTRIES};

/// 一覧の 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSummary {
    pub id: String,
    /// 開始時刻・同意の記録。読めない（無い・壊れている）ときは `None`（削除はできる）。
    pub meta: Option<RecordingMeta>,
    /// 保存されているチャンクファイルの数（通常のファイルだけ）。
    pub chunk_count: u32,
    /// チャンクファイルの合計バイト数。
    pub bytes: u64,
}

impl FsChunkStore {
    /// 保存済みの録音を、id の昇順で返す。録音 id として正しい名前の、通常のディレクトリ（リンクでない）だけが対象。
    /// ルートが無ければ空。ルートがリンクなら拒否する。
    pub fn list_recordings(&self) -> Result<Vec<RecordingSummary>, IngestError> {
        self.check_root()?;
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut ids = Vec::new();
        for entry in entries.take(MAX_SCAN_ENTRIES) {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_recording_id(&name).is_ok() && entry.file_type()?.is_dir() {
                ids.push(name);
            }
        }
        ids.sort_unstable();
        Ok(ids.into_iter().map(|id| self.summarize(id)).collect())
    }

    fn summarize(&self, id: String) -> RecordingSummary {
        let meta = self.load_meta(&id).ok();
        let (mut chunk_count, mut bytes) = (0u32, 0u64);
        if let Ok(scan) = scan_dir(&self.root.join(&id)) {
            for (_, path) in &scan.found {
                chunk_count = chunk_count.saturating_add(1);
                bytes = bytes.saturating_add(fs::metadata(path).map_or(0, |m| m.len()));
            }
        }
        RecordingSummary {
            id,
            meta,
            chunk_count,
            bytes,
        }
    }
}

#[cfg(test)]
mod tests;
