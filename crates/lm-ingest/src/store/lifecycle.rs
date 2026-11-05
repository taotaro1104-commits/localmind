//! 録音の開始・同意の撤回（同意の強制）と、録音ディレクトリの保守（古い一時ファイルの掃除）。

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::fsio::{
    ensure_real_dir, read_regular_limited, real_dir_exists, write_atomic, write_new_atomic,
};
use super::{
    meta_text, parse_meta, validate_recording_id, FsChunkStore, RecordingMeta, MAX_META_BYTES,
    MAX_SCAN_ENTRIES, META_FILE,
};
use crate::audio::validate_sample_rate;
use crate::error::IngestError;

/// `clean_stale_tmp` が受け付ける最小の経過時間。これより新しい一時ファイルは、書き込み中の
/// 保存かもしれない（消すと、書き手の rename が失敗してデータを失う）ので、指定できないようにする。
pub const MIN_TMP_AGE: Duration = Duration::from_secs(60);

/// `clean_stale_tmp` の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TmpCleanReport {
    /// 消したファイル（`<録音id>/<ファイル名>`、昇順）。
    pub removed: Vec<String>,
    /// 調べられなかった・消せなかったもの（理由つき。録音ディレクトリ名までで、フルパスは含めない）。
    pub failed: Vec<String>,
}

/// 保存の一時ファイル名（`<元の名前>.<pid>-<連番>.tmp`）か。名前が `.tmp` で終わるだけの別ファイルは対象外。
fn is_tmp_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".tmp") else {
        return false;
    };
    let Some((base, tail)) = stem.rsplit_once('.') else {
        return false;
    };
    let Some((pid, seq)) = tail.split_once('-') else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    !base.is_empty() && digits(pid) && digits(seq)
}

fn not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

impl FsChunkStore {
    /// 録音を始める。同意が確認されていなければ拒否し、何も作らない。
    /// 同じ id の録音（メタ）が既にあれば拒否する。メタは「無ければ作る」を原子的に行うので、同時に 2 回
    /// 呼ばれても片方だけが成功し、開始時刻は上書きされない（自動削除の期限を延ばせない）。
    /// 録音の取り込みは、チャンクを保存する前に必ずこれを呼ぶこと（`save_chunk` は同意の記録が無ければ拒否する）。
    pub fn begin_recording(&self, id: &str, meta: &RecordingMeta) -> Result<(), IngestError> {
        self.begin_recording_with(id, meta, write_new_atomic)
    }

    /// `begin_recording` の本体。`create` はメタを「無ければ作る」操作（テストで失敗を再現する）。
    /// メタを作れなかったら、このために作った空のディレクトリを片付ける（メタの無い録音を残さない。
    /// 中身のあるディレクトリは消えない）。
    pub(super) fn begin_recording_with(
        &self,
        id: &str,
        meta: &RecordingMeta,
        create: impl FnOnce(&Path, &[u8]) -> Result<bool, IngestError>,
    ) -> Result<(), IngestError> {
        if !meta.consent_confirmed {
            return Err(IngestError::Consent(
                "同意が確認されていないため録音を始められません".into(),
            ));
        }
        validate_sample_rate(meta.sample_rate)?;
        let dir = self.dir(id)?;
        ensure_real_dir(&dir)?;
        match create(&dir.join(META_FILE), meta_text(meta).as_bytes()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(IngestError::InvalidFormat(format!(
                "同じ id の録音が既にあります: {id}"
            ))),
            Err(e) => {
                let _ = fs::remove_dir(&dir); // 空のときだけ消える
                Err(e)
            }
        }
    }

    /// 同意を撤回する。以後、この録音へはチャンクを保存できない。開始時刻など他の項目は変えない
    /// （同意を「確認済み」に戻す・開始時刻を書き換える手段は公開しない）。録音が無ければエラー。
    pub fn revoke_consent(&self, id: &str) -> Result<(), IngestError> {
        let mut meta = self.load_meta(id)?;
        meta.consent_confirmed = false;
        write_atomic(&self.dir(id)?.join(META_FILE), meta_text(&meta).as_bytes())
    }

    /// 同意が確認された録音にだけ、チャンクの保存を許す。メタが無い・壊れている・同意なしは、すべて拒否する。
    /// （確認と書き込みの間にメタが書き換わる競合は防げない。アプリ専用の保存先を前提にした、誤用への歯止め。）
    pub(super) fn require_consent(&self, dir: &Path) -> Result<(), IngestError> {
        let path = dir.join(META_FILE);
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(e) if not_found(&e) => return Err(no_consent_record()),
            Err(e) => return Err(e.into()),
        }
        let bytes = read_regular_limited(&path, MAX_META_BYTES)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| IngestError::Corrupt("メタ情報が UTF-8 ではありません".into()))?;
        if parse_meta(&text)?.consent_confirmed {
            Ok(())
        } else {
            Err(IngestError::Consent(
                "同意が確認されていない録音には保存できません".into(),
            ))
        }
    }

    /// `max_age` より古い一時ファイル（書き込み中に止まった断片）を消す。
    /// `max_age` は `MIN_TMP_AGE` 以上（新しいものは書き込み中かもしれないので消さない）。
    /// 対象は、通常のファイルで、保存が作る一時ファイルの名前（`<名前>.<pid>-<連番>.tmp`）のものだけ。
    /// 書き手と競合して消えたものは飛ばし、読めない録音ディレクトリは `failed` に記録して続ける。
    pub fn clean_stale_tmp(&self, max_age: Duration) -> Result<TmpCleanReport, IngestError> {
        if max_age < MIN_TMP_AGE {
            return Err(IngestError::InvalidFormat(format!(
                "経過時間は {} 秒以上にしてください",
                MIN_TMP_AGE.as_secs()
            )));
        }
        self.check_root()?;
        let mut report = TmpCleanReport::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if not_found(&e) => return Ok(report),
            Err(e) => return Err(e.into()),
        };
        let now = SystemTime::now();
        let mut seen = 0usize;
        'ids: for entry in entries {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if validate_recording_id(&id).is_err() || !entry.file_type()?.is_dir() {
                continue;
            }
            match real_dir_exists(&entry.path()) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    report.failed.push(format!("{id}: {e}"));
                    continue;
                }
            }
            let files = match fs::read_dir(entry.path()) {
                Ok(files) => files,
                Err(e) if not_found(&e) => continue,
                Err(e) => {
                    report.failed.push(format!("{id}: 読めません: {e}"));
                    continue;
                }
            };
            for file in files {
                seen += 1;
                if seen > MAX_SCAN_ENTRIES {
                    report
                        .failed
                        .push(format!("{MAX_SCAN_ENTRIES} 件目以降は調べていません"));
                    break 'ids;
                }
                let Ok(file) = file else { continue };
                let name = file.file_name().to_string_lossy().into_owned();
                if !is_tmp_name(&name) {
                    continue;
                }
                let stale = file
                    .file_type()
                    .ok()
                    .filter(fs::FileType::is_file)
                    .and_then(|_| file.metadata().ok())
                    .and_then(|m| m.modified().ok())
                    .and_then(|m| now.duration_since(m).ok())
                    .is_some_and(|age| age >= max_age);
                if !stale {
                    continue;
                }
                match fs::remove_file(file.path()) {
                    Ok(()) => report.removed.push(format!("{id}/{name}")),
                    Err(e) if not_found(&e) => {}
                    Err(e) => report.failed.push(format!("{id}/{name}: 消せません: {e}")),
                }
            }
        }
        report.removed.sort();
        Ok(report)
    }
}

fn no_consent_record() -> IngestError {
    IngestError::Consent("同意の記録がありません（先に begin_recording を呼んでください）".into())
}
