//! 録音ディレクトリの走査（チャンクファイルの収集と、読まないファイルの報告）。

use std::fs;
use std::path::{Path, PathBuf};

use super::{
    display_name, parse_chunk_file_name, IngestError, MAX_SCAN_ENTRIES, MAX_SKIPPED_REPORTS,
};

/// 報告を `MAX_SKIPPED_REPORTS` 件までにして、超えた分は件数だけ数える（巨大な文字列列を作らない）。
pub(super) fn note_skipped(
    skipped: &mut Vec<String>,
    omitted: &mut usize,
    message: impl FnOnce() -> String,
) {
    if skipped.len() < MAX_SKIPPED_REPORTS {
        skipped.push(message());
    } else {
        *omitted += 1;
    }
}

/// ディレクトリ走査の結果。
pub(super) struct Scan {
    /// 読めるチャンクファイル（連番順）。
    pub found: Vec<(u32, PathBuf)>,
    /// 読まないファイルの報告（`MAX_SKIPPED_REPORTS` 件まで）。
    pub skipped: Vec<String>,
    /// 報告に載せきれなかった件数。
    pub omitted: usize,
    /// エントリ数の上限で走査を打ち切ったか（`read_dir` の順序は不定なので、見つかった分は一部のみ）。
    pub truncated: bool,
}

impl Scan {
    fn skip(&mut self, message: impl FnOnce() -> String) {
        note_skipped(&mut self.skipped, &mut self.omitted, message);
    }
}

/// ディレクトリ内のチャンクファイル（通常のファイルのみ）と、読まないファイルの報告を集める。
pub(super) fn scan_dir(dir: &Path) -> Result<Scan, IngestError> {
    scan_dir_limited(dir, MAX_SCAN_ENTRIES)
}

/// `scan_dir` の本体。見るエントリ数を `max_entries` までにし、超えたら打ち切って報告する
/// （全体を拒否せず、それまでに見つけた分で復元を続ける）。
pub(super) fn scan_dir_limited(dir: &Path, max_entries: usize) -> Result<Scan, IngestError> {
    let mut scan = Scan {
        found: Vec::new(),
        skipped: Vec::new(),
        omitted: 0,
        truncated: false,
    };
    for (seen, entry) in fs::read_dir(dir)?.enumerate() {
        if seen >= max_entries {
            scan.truncated = true;
            break;
        }
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tmp") {
            // ファイル名は外部由来なので、制御文字をエスケープして報告する。
            scan.skip(|| format!("{}: 未完了の一時ファイル", display_name(&name)));
        } else if let Some(index) = parse_chunk_file_name(&name) {
            // 種類の確認は、チャンクの名前を持つものだけに行う（無関係なファイルで走査を止めない）。
            if entry.file_type()?.is_file() {
                scan.found.push((index, entry.path()));
            } else {
                scan.skip(|| {
                    format!(
                        "{}: 通常のファイルではないため読みません",
                        display_name(&name)
                    )
                });
            }
        }
    }
    // 連番順に並べておく（復元側で位置による参照ができ、探索が O(n) で済む）。
    scan.found.sort_unstable_by_key(|(index, _)| *index);
    Ok(scan)
}
