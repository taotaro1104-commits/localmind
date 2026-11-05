//! チャンクのファイル保存と、クラッシュ後の復元。
//!
//! - 1 録音 = 1 ディレクトリ（`<root>/<録音id>/`）。チャンクは `NNNNNNNN.pcm`、メタは `meta.txt`。
//! - 保存は一時ファイル → fsync → rename の原子的な手順。途中でクラッシュしても、
//!   読み込み側が見るのは「完全なチャンク」か「存在しない」のどちらかになる。
//!   一時ファイル名は呼び出しごとに一意で、同じチャンクへの同時書き込みでも一時ファイルは衝突しない
//!   （ただし録音のライターは 1 つを前提とする。同時に rename すると OS によっては一方が失敗する）。
//! - 復元は先頭から連続して読める範囲までを返す（欠け・破損の手前まで）。通常のファイル以外
//!   （ディレクトリ・シンボリックリンク）は読まない。
//! - 録音 id は小文字の英数字・`_`・`-` のみで、OS の予約名は不可（パス注入と、大文字小文字を区別しない
//!   ファイルシステムでの衝突を防ぐ）。
//! - 保存前に `validate_chunk` で再読込できるチャンクかを確認し、本体には CRC-32 を付ける
//!   （ヘッダの長さは実ファイルサイズと突き合わせてから確保する）。

use std::fs;
use std::path::{Path, PathBuf};

use crate::audio::{validate_sample_rate, MAX_CHUNK_SECONDS};
use crate::chunk::Chunk;
use crate::error::IngestError;

/// チャンクの開始位置の上限（録音開始から約 30 日）。異常なヘッダによる時刻計算の溢れを防ぐ。
pub const MAX_START_MS: u64 = 30 * 24 * 3600 * 1000;
const MAX_ID_LEN: usize = 64;
const META_FILE: &str = "meta.txt";
/// Windows の予約デバイス名（小文字）。`CON` などは通常のファイル名として使えない。
const RESERVED_NAMES: [&str; 4] = ["con", "prn", "aux", "nul"];
/// チャンクのファイル名は 8 桁の連番なので、連番の上限はそれに収まる値。
pub const MAX_CHUNK_INDEX: u32 = 99_999_999;
/// `meta.txt` の最大サイズ（数行の設定なので小さくてよい）。
const MAX_META_BYTES: u64 = 4096;
/// 1 回の復元でメモリに載せる PCM の上限（バイト）。約 9 時間分（16kHz）。超えた分は未使用として報告する。
pub const MAX_RESTORE_BYTES: u64 = 1 << 30;
/// 1 回の復元で読むチャンク数の上限（30 秒チャンクで約 69 日分）。空のチャンクを大量に置かれても無制限に増えない。
pub const MAX_RESTORE_CHUNKS: usize = 200_000;
/// ディレクトリ走査で見るエントリ数の上限。超えたら破損として扱う。
const MAX_SCAN_ENTRIES: usize = 1_000_000;
/// `Recovery::skipped` に載せる件数の上限（超えた分は件数だけ報告する）。
const MAX_SKIPPED_REPORTS: usize = 1000;

/// 録音のメタ情報（同意確認の記録を含む）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingMeta {
    /// 録音開始時に「参加者の同意を得ましたか」を確認したか。
    pub consent_confirmed: bool,
    pub sample_rate: u32,
    /// 録音開始時刻（UNIX ミリ秒）。
    pub started_at_ms: u64,
}

/// 復元の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// 先頭から連続して読めたチャンク。
    pub chunks: Vec<Chunk>,
    /// 読めなかったファイルとその理由（欠けの先のチャンク、破損、未完了の一時ファイル）。
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FsChunkStore {
    root: PathBuf,
}

/// 予約デバイス名（`com1`〜`com9`、`lpt1`〜`lpt9` を含む）か。
fn is_reserved_name(id: &str) -> bool {
    let numbered = |prefix: &str| {
        id.strip_prefix(prefix)
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
    };
    RESERVED_NAMES.contains(&id) || numbered("com") || numbered("lpt")
}

/// 録音 id がファイル名として安全か検証する。
pub fn validate_recording_id(id: &str) -> Result<(), IngestError> {
    let charset_ok = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !id.is_empty() && id.len() <= MAX_ID_LEN && charset_ok && !is_reserved_name(id) {
        Ok(())
    } else {
        Err(IngestError::InvalidId(format!(
            "小文字の英数字・_・- のみ、1〜{MAX_ID_LEN} 文字、OS の予約名は不可: {id:?}"
        )))
    }
}

/// 保存・復元できるチャンクか検証する（サンプルレート、長さ、開始位置）。
pub fn validate_chunk(chunk: &Chunk) -> Result<(), IngestError> {
    validate_sample_rate(chunk.sample_rate)?;
    if chunk.index > MAX_CHUNK_INDEX {
        return Err(IngestError::InvalidFormat(format!(
            "チャンクの連番が大きすぎます: {}（上限 {MAX_CHUNK_INDEX}）",
            chunk.index
        )));
    }
    let max_samples = u64::from(chunk.sample_rate) * u64::from(MAX_CHUNK_SECONDS);
    if chunk.samples.len() as u64 > max_samples {
        return Err(IngestError::InvalidFormat(format!(
            "チャンクが長すぎます: {} サンプル（上限 {max_samples}）",
            chunk.samples.len()
        )));
    }
    if chunk.start_ms > MAX_START_MS {
        return Err(IngestError::InvalidFormat(format!(
            "チャンクの開始位置が大きすぎます: {} ms（上限 {MAX_START_MS}）",
            chunk.start_ms
        )));
    }
    Ok(())
}

fn chunk_file_name(index: u32) -> String {
    format!("{index:08}.pcm")
}

/// `NNNNNNNN.pcm`（8 桁の数字）なら連番を返す。
fn parse_chunk_file_name(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".pcm")?;
    (stem.len() == 8 && stem.bytes().all(|b| b.is_ascii_digit()))
        .then(|| stem.parse().ok())
        .flatten()
}

/// 通常のファイルだけを、サイズ上限つきで読んでデコードする（破損・悪意あるファイルで巨大な確保をしない）。
fn read_chunk(path: &Path, expected_index: u32) -> Result<Chunk, IngestError> {
    decode(&read_regular_limited(path, MAX_FILE_BYTES)?, expected_index)
}

/// ログや UI に出すファイル名の制御文字（改行・ESC など）をエスケープする。
fn display_name(name: &str) -> String {
    name.escape_debug().to_string()
}

/// 連番のうち、先頭（0）から欠けなく続く範囲を使い、それ以降を未使用として理由つきで返す。
/// 入力の順序は問わない。
fn plan_recovery(indices: &[u32]) -> (Vec<u32>, Vec<(u32, String)>) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    let usable_len = sorted
        .iter()
        .zip(0u32..)
        .take_while(|(index, expected)| *index == expected)
        .count();
    let unusable = sorted[usable_len..]
        .iter()
        .enumerate()
        .map(|(i, &index)| {
            let reason = if i == 0 {
                format!("連番 {usable_len} が欠けているため未使用")
            } else {
                "手前のチャンクが欠けているため未使用".to_string()
            };
            (index, reason)
        })
        .collect();
    sorted.truncate(usable_len);
    (sorted, unusable)
}

fn meta_text(meta: &RecordingMeta) -> String {
    format!(
        "consent_confirmed={}\nsample_rate={}\nstarted_at_ms={}\n",
        meta.consent_confirmed, meta.sample_rate, meta.started_at_ms
    )
}

fn parse_meta(text: &str) -> Result<RecordingMeta, IngestError> {
    let value = |key: &str| -> Result<&str, IngestError> {
        text.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .ok_or_else(|| corrupt(format!("メタ情報に {key} がありません")))
    };
    let bad = |key: &str| corrupt(format!("メタ情報の {key} が不正です"));
    let meta = RecordingMeta {
        consent_confirmed: value("consent_confirmed")?
            .parse()
            .map_err(|_| bad("consent_confirmed"))?,
        sample_rate: value("sample_rate")?
            .parse()
            .map_err(|_| bad("sample_rate"))?,
        started_at_ms: value("started_at_ms")?
            .parse()
            .map_err(|_| bad("started_at_ms"))?,
    };
    validate_sample_rate(meta.sample_rate).map_err(|_| bad("sample_rate"))?;
    Ok(meta)
}

impl FsChunkStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(super) fn dir(&self, id: &str) -> Result<PathBuf, IngestError> {
        validate_recording_id(id)?;
        Ok(self.root.join(id))
    }

    /// チャンクを保存する（同じ連番は上書き）。再読込できないチャンクは保存せずエラーにする。
    pub fn save_chunk(&self, id: &str, chunk: &Chunk) -> Result<(), IngestError> {
        validate_chunk(chunk)?;
        let dir = self.dir(id)?;
        ensure_real_dir(&dir)?;
        write_atomic(&dir.join(chunk_file_name(chunk.index)), &encode(chunk))
    }

    /// 先頭から連続して読めるチャンクを返す。録音が存在しなければ空。
    pub fn load_chunks(&self, id: &str) -> Result<Recovery, IngestError> {
        self.load_chunks_limited(id, MAX_RESTORE_BYTES, MAX_RESTORE_CHUNKS)
    }

    /// `load_chunks` の本体。メモリに載せる PCM の合計を `max_bytes` までに制限する。
    fn load_chunks_limited(
        &self,
        id: &str,
        max_bytes: u64,
        max_chunks: usize,
    ) -> Result<Recovery, IngestError> {
        let dir = self.dir(id)?;
        let mut recovery = Recovery::default();
        if !real_dir_exists(&dir)? {
            return Ok(recovery);
        }
        let Scan {
            found,
            skipped,
            mut omitted,
            truncated,
        } = scan_dir(&dir)?;
        recovery.skipped = skipped;
        if truncated {
            // 件数上限に数えず、必ず報告する（見つかった分は一部のみで、復元量が実行ごとに変わりうる）。
            recovery.skipped.push(format!(
                "ファイルが多すぎるため、{MAX_SCAN_ENTRIES} 件目以降は調べていません"
            ));
        }

        let indices: Vec<u32> = found.iter().map(|(i, _)| *i).collect();
        let (usable, unusable) = plan_recovery(&indices);
        for (index, reason) in unusable {
            note_skipped(&mut recovery.skipped, &mut omitted, || {
                format!("{}: {reason}", chunk_file_name(index))
            });
        }

        let mut restored_bytes: u64 = 0;
        for (pos, index) in usable.iter().enumerate() {
            // `found` は連番順で、`usable` は 0 から欠けなく続く先頭部分なので、位置がそのまま対応する。
            let Some((_, path)) = found.get(pos) else {
                continue;
            };
            if recovery.chunks.len() >= max_chunks {
                for later in &usable[pos..] {
                    note_skipped(&mut recovery.skipped, &mut omitted, || {
                        format!(
                            "{}: 復元できるチャンク数の上限（{max_chunks}）を超えるため未使用",
                            chunk_file_name(*later)
                        )
                    });
                }
                break;
            }
            match read_chunk(path, *index) {
                Ok(chunk) if restored_bytes + chunk.samples.len() as u64 * 2 > max_bytes => {
                    for later in &usable[pos..] {
                        note_skipped(&mut recovery.skipped, &mut omitted, || {
                            format!(
                                "{}: 復元量の上限（{max_bytes} バイト）を超えるため未使用",
                                chunk_file_name(*later)
                            )
                        });
                    }
                    break;
                }
                Ok(chunk) => {
                    restored_bytes += chunk.samples.len() as u64 * 2;
                    recovery.chunks.push(chunk);
                }
                Err(e) => {
                    recovery
                        .skipped
                        .push(format!("{}: {e}", chunk_file_name(*index)));
                    for later in &usable[pos + 1..] {
                        note_skipped(&mut recovery.skipped, &mut omitted, || {
                            format!(
                                "{}: 手前のチャンクが読めないため未使用",
                                chunk_file_name(*later)
                            )
                        });
                    }
                    break;
                }
            }
        }
        if omitted > 0 {
            recovery.skipped.push(format!("ほか {omitted} 件（省略）"));
        }
        Ok(recovery)
    }

    pub fn save_meta(&self, id: &str, meta: &RecordingMeta) -> Result<(), IngestError> {
        validate_sample_rate(meta.sample_rate)?;
        let dir = self.dir(id)?;
        ensure_real_dir(&dir)?;
        write_atomic(&dir.join(META_FILE), meta_text(meta).as_bytes())
    }

    /// メタ情報を読む。通常のファイルで `MAX_META_BYTES` 以下のものだけを読む。
    pub fn load_meta(&self, id: &str) -> Result<RecordingMeta, IngestError> {
        let dir = self.dir(id)?;
        if !real_dir_exists(&dir)? {
            return Err(IngestError::Io(format!("録音がありません: {id}")));
        }
        let bytes = read_regular_limited(&dir.join(META_FILE), MAX_META_BYTES)?;
        let text =
            String::from_utf8(bytes).map_err(|_| corrupt("メタ情報が UTF-8 ではありません"))?;
        parse_meta(&text)
    }

    /// 録音ごと削除する（録音ファイルの自動削除期限用）。存在すれば true。
    /// 録音ディレクトリがシンボリックリンクなら、リンク先を消さないよう拒否する。
    pub fn delete_recording(&self, id: &str) -> Result<bool, IngestError> {
        let dir = self.dir(id)?;
        if !real_dir_exists(&dir)? {
            return Ok(false);
        }
        // メタは最後に消す。途中で失敗しても、次回の期限切れ判定に使えるメタが残る
        // （メタだけ先に消えると、録音本体が永久に「開始時刻不明」で残ってしまう）。
        let mut has_meta = false;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_name() == META_FILE {
                has_meta = true;
            } else if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        if has_meta {
            fs::remove_file(dir.join(META_FILE))?;
        }
        fs::remove_dir(&dir)?;
        Ok(true)
    }
}

mod format;
mod fsio;
mod minutes_io;
mod retention;
mod scan;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_format;
#[cfg(test)]
mod tests_minutes;
#[cfg(test)]
mod tests_retention;
#[cfg(test)]
mod tests_security;
#[cfg(test)]
mod tests_write;

use format::{corrupt, decode, encode, MAX_FILE_BYTES};
#[cfg(test)]
use format::{crc32, HEADER_LEN};
use fsio::{ensure_real_dir, read_regular_limited, real_dir_exists, write_atomic};
#[cfg(test)]
use fsio::{tmp_path, write_atomic_with};
pub use retention::{is_expired, PurgeReport};
use scan::{note_skipped, scan_dir, Scan};
