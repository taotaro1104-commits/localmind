//! 録音の自動削除期限（A-2）。録音開始時刻から保持日数を過ぎた録音を削除する。
//!
//! 削除は取り返しがつかないので、判断は保守的にする。
//! - 保持日数 0 は拒否する（全削除にならないように）。
//! - メタ情報が読めない録音、開始時刻が非現実的（`MIN_PLAUSIBLE_MS` 未満）な録音は削除しない。理由を報告する。
//! - 現在時刻が非現実的（`MIN_PLAUSIBLE_MS` 未満）なら何も消さずに拒否する。
//! - 1 回の削除数に上限（`MAX_DELETIONS_PER_RUN`）を設ける。
//! - 録音 id として不正な名前や、通常のディレクトリでないものには触れない。

use std::fs;

use super::{validate_recording_id, FsChunkStore, MAX_SCAN_ENTRIES};
use crate::error::IngestError;

const MS_PER_DAY: u64 = 24 * 60 * 60 * 1000;
/// 現実的な時刻の下限（2024-01-01 UTC、UNIX ミリ秒）。これより前の `now_ms` は、秒とミリ秒の取り違えや
/// 時計の初期化ミスとみなして拒否する（全録音が期限切れになる事故を防ぐ）。
pub const MIN_PLAUSIBLE_MS: u64 = 1_704_067_200_000;
/// 1 回の実行で削除する録音の上限。超えた分は次回に回す（誤った時刻でも被害を抑える）。
pub const MAX_DELETIONS_PER_RUN: usize = 100;

/// 期限切れ削除の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// 削除した録音 id（昇順）。
    pub deleted: Vec<String>,
    /// 削除しなかった録音とその理由（メタ情報が読めない、削除に失敗した、など）。
    pub kept_with_reason: Vec<String>,
}

/// 開始時刻から `retention_days` 日を過ぎているか。時計が巻き戻っている（`now_ms` が開始より前）
/// 場合は期限切れにしない。
pub fn is_expired(started_at_ms: u64, now_ms: u64, retention_days: u32) -> bool {
    let limit = u64::from(retention_days).saturating_mul(MS_PER_DAY);
    now_ms >= started_at_ms && now_ms - started_at_ms >= limit
}

impl FsChunkStore {
    /// 保持期限を過ぎた録音を削除する。`now_ms` は現在時刻（UNIX ミリ秒。呼び出し側が渡す）。
    pub fn purge_expired(
        &self,
        now_ms: u64,
        retention_days: u32,
    ) -> Result<PurgeReport, IngestError> {
        if retention_days == 0 {
            return Err(IngestError::InvalidFormat(
                "保持日数は 1 以上でなければなりません".into(),
            ));
        }
        if now_ms < MIN_PLAUSIBLE_MS {
            return Err(IngestError::InvalidFormat(
                "現在時刻が不正です（ミリ秒の UNIX 時刻を渡してください）".into(),
            ));
        }
        self.check_root()?;
        let mut report = PurgeReport::default();
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
            Err(e) => return Err(e.into()),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // 録音 id として正しく、通常のディレクトリ（リンクでない）ものだけを対象にする。
            if validate_recording_id(&name).is_ok() && entry.file_type()?.is_dir() {
                // 上限は録音の数だけに適用する（無関係なエントリで本物が調べられなくならないように）。
                if ids.len() >= MAX_SCAN_ENTRIES {
                    report
                        .kept_with_reason
                        .push(format!("{MAX_SCAN_ENTRIES} 件目以降は調べていません"));
                    break;
                }
                ids.push(name);
            }
        }
        ids.sort();
        for id in ids {
            match self.load_meta(&id) {
                Ok(meta) if meta.started_at_ms < MIN_PLAUSIBLE_MS => report
                    .kept_with_reason
                    .push(format!("{id}: 開始時刻が不正なため残します")),
                Ok(meta) if is_expired(meta.started_at_ms, now_ms, retention_days) => {
                    if report.deleted.len() >= MAX_DELETIONS_PER_RUN {
                        report.kept_with_reason.push(format!(
                            "削除数の上限（{MAX_DELETIONS_PER_RUN} 件）に達したため、残りは次回に回します"
                        ));
                        break;
                    }
                    match self.delete_recording(&id) {
                        Ok(_) => report.deleted.push(id),
                        Err(e) => report
                            .kept_with_reason
                            .push(format!("{id}: 削除できません: {e}")),
                    }
                }
                Ok(_) => {}
                Err(e) => report
                    .kept_with_reason
                    .push(format!("{id}: メタ情報を読めないため残します: {e}")),
            }
        }
        Ok(report)
    }
}
