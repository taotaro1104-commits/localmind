//! 録音の保存の公開面。Kotlin 側がマイクから読んだ PCM を渡すと、30 秒ごとのチャンクに分けて端末内に逐次保存する。
//!
//! 同意の確認が無い録音は始められず、保存もできない（`begin` が拒否する）。保持期限を過ぎた録音は
//! `purge_expired` で消す。クラッシュしても、保存済みのチャンクまでは残る（失うのは最大 1 チャンク分）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::FfiError;
use crate::search::check_abs_path;
use lm_ingest::{
    export_minutes, ChunkWriter, ExportFormat as IngestFormat, ExportMeta, FsChunkStore,
    RecordingMeta, MIN_TMP_AGE,
};

/// 保存済みの録音 1 件の情報。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecordingInfo {
    pub id: String,
    /// 開始時刻（UNIX ミリ秒）。メタ情報が読めないとき（`readable == false`）は 0。
    pub started_at_ms: u64,
    /// 開始時に同意が確認されていたか。読めないときは false。
    pub consent_confirmed: bool,
    /// メタ情報を読めたか。読めない録音も、削除はできる。
    pub readable: bool,
    /// 保存済みのチャンクの数（標準は 1 つ 30 秒）。
    pub chunks: u32,
    pub bytes: u64,
}

/// 期限切れの削除の結果。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PurgeSummary {
    pub deleted: Vec<String>,
    /// 削除しなかった録音の数（メタ情報が読めない、削除に失敗した、など）。
    pub kept: u32,
}

/// 議事録の書き出しの形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ExportFormat {
    Markdown,
    Csv,
    /// アクションアイテムを ToDo にした iCalendar。
    Ics,
}

impl From<ExportFormat> for IngestFormat {
    fn from(f: ExportFormat) -> Self {
        match f {
            ExportFormat::Markdown => IngestFormat::Markdown,
            ExportFormat::Csv => IngestFormat::Csv,
            ExportFormat::Ics => IngestFormat::Ics,
        }
    }
}

/// 書き出した議事録。`text` をファイルに保存・共有する（CSV は先頭に BOM つきの UTF-8）。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExportedFile {
    pub file_name: String,
    pub mime_type: String,
    pub text: String,
}

/// 録音の保存先（アプリ専用ディレクトリ内の 1 つのディレクトリ）。
#[derive(uniffi::Object)]
pub struct RecordingStore {
    store: FsChunkStore,
}

/// 録音 1 回分の書き込み口。`push` で PCM を渡し、終わったら `finish`。複数のスレッドから呼んでも安全。
#[derive(uniffi::Object)]
pub struct Recorder {
    store: FsChunkStore,
    id: String,
    /// `finish` 後は `None`（以後の `push` は拒否する）。
    writer: Mutex<Option<ChunkWriter>>,
    saved: AtomicU32,
}

impl RecordingStore {
    /// 保存先（文字起こしなど、同じ crate の別の機能が録音を読むのに使う）。
    #[cfg(feature = "whisper")]
    pub(crate) fn fs(&self) -> &FsChunkStore {
        &self.store
    }
}

impl Recorder {
    fn writer(&self) -> MutexGuard<'_, Option<ChunkWriter>> {
        // 別スレッドが失敗しても止めない（保存済みのチャンクは壊れない）。
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[uniffi::export]
impl RecordingStore {
    /// 保存先を開く。`root` は `..` を含まない絶対パス（無ければ最初の保存で作られる）。
    #[uniffi::constructor]
    pub fn open(root: String) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&root, "録音の保存先")?;
        Ok(Arc::new(Self {
            store: FsChunkStore::new(root),
        }))
    }

    /// 録音を始める。`consent_confirmed` が false なら拒否し、何も作らない。同じ id の録音があれば拒否する。
    /// `id` は小文字の英数字・`_`・`-`（64 文字まで、OS の予約名は不可）。`sample_rate` は 1〜192000 Hz（音声の録音は 16000 を勧める）。
    pub fn begin(
        &self,
        id: String,
        consent_confirmed: bool,
        sample_rate: u32,
        started_at_ms: u64,
    ) -> Result<Arc<Recorder>, FfiError> {
        let meta = RecordingMeta {
            consent_confirmed,
            sample_rate,
            started_at_ms,
        };
        self.store.begin_recording(&id, &meta)?;
        Ok(Arc::new(Recorder {
            store: self.store.clone(),
            writer: Mutex::new(Some(ChunkWriter::standard(sample_rate)?)),
            id,
            saved: AtomicU32::new(0),
        }))
    }

    /// 保存済みの録音（id の昇順）。
    pub fn recordings(&self) -> Result<Vec<RecordingInfo>, FfiError> {
        Ok(self
            .store
            .list_recordings()?
            .into_iter()
            .map(|r| RecordingInfo {
                readable: r.meta.is_some(),
                started_at_ms: r.meta.as_ref().map_or(0, |m| m.started_at_ms),
                consent_confirmed: r.meta.as_ref().is_some_and(|m| m.consent_confirmed),
                id: r.id,
                chunks: r.chunk_count,
                bytes: r.bytes,
            })
            .collect())
    }

    /// 保存済みの議事録を、Markdown・CSV・iCalendar（ToDo）として書き出す。議事録が無ければエラー。
    /// `title` は題名（空なら「議事録」）、`utc_offset_minutes` は現地の UTC からの差（日本は 540）、
    /// `now_ms` は書き出す時刻（UNIX ミリ秒）。iCalendar は、アクションアイテムが無いとエラー。
    pub fn export_minutes(
        &self,
        id: String,
        format: ExportFormat,
        title: String,
        utc_offset_minutes: i32,
        now_ms: u64,
    ) -> Result<ExportedFile, FfiError> {
        let minutes = self
            .store
            .load_minutes(&id)?
            .ok_or_else(|| FfiError::Ingest {
                msg: "この録音の議事録はまだありません".into(),
            })?;
        let meta = ExportMeta {
            started_at_ms: self.store.load_meta(&id)?.started_at_ms,
            recording_id: id,
            title,
            exported_at_ms: now_ms,
            utc_offset_minutes,
        };
        let out = export_minutes(&minutes, &meta, format.into())?;
        Ok(ExportedFile {
            file_name: out.file_name,
            mime_type: out.mime_type.to_string(),
            text: out.text,
        })
    }

    /// 録音を消す。無ければ `false`。
    pub fn delete_recording(&self, id: String) -> Result<bool, FfiError> {
        Ok(self.store.delete_recording(&id)?)
    }

    /// 同意を撤回する。以後、この録音には保存できない（保存済みの分は残るので、必要なら消す）。
    pub fn revoke_consent(&self, id: String) -> Result<(), FfiError> {
        Ok(self.store.revoke_consent(&id)?)
    }

    /// 保持期限（日数）を過ぎた録音を消す。`now_ms` は現在時刻（UNIX ミリ秒）、`retention_days` は 1 以上。
    /// 1 回に消すのは 100 件まで（残りは次回）。メタ情報が読めない録音は、録音ディレクトリの最終更新時刻から
    /// 保持期間を過ぎていれば消す（最近まで更新されていたものは残す）。
    pub fn purge_expired(
        &self,
        now_ms: u64,
        retention_days: u32,
    ) -> Result<PurgeSummary, FfiError> {
        let r = self.store.purge_expired(now_ms, retention_days)?;
        Ok(PurgeSummary {
            deleted: r.deleted,
            kept: u32::try_from(r.kept_with_reason.len()).unwrap_or(u32::MAX),
        })
    }

    /// 書き込み途中で残った古い一時ファイルを掃除する。消した数を返す。
    pub fn clean_stale_tmp(&self) -> Result<u32, FfiError> {
        let r = self.store.clean_stale_tmp(MIN_TMP_AGE)?;
        Ok(u32::try_from(r.removed.len()).unwrap_or(u32::MAX))
    }
}

#[uniffi::export]
impl Recorder {
    /// PCM（16bit・モノラル）を渡す。たまった分を 30 秒ごとに保存し、保存したチャンクの総数を返す。
    /// 1 回に渡せるのは 600 秒分まで。保存に失敗したら `Err`（録音を止めて、利用者に知らせること）。
    /// 失敗したあとは、この録音は終了扱い（以後の `push` は拒否し、`finish` は端数を保存しない。
    /// 抜けのあるチャンクの並びを作らないため）。
    pub fn push(&self, pcm: Vec<i16>) -> Result<u32, FfiError> {
        let mut guard = self.writer();
        let writer = guard.as_mut().ok_or_else(finished)?;
        let result = (|| -> Result<(), FfiError> {
            for chunk in writer.push(&pcm)? {
                self.store.save_chunk(&self.id, &chunk)?;
                self.saved.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        })();
        if let Err(e) = result {
            *guard = None;
            return Err(e);
        }
        Ok(self.saved.load(Ordering::Relaxed))
    }

    /// 録音を終える。残った端数を最後のチャンクとして保存し、保存したチャンクの総数を返す。2 回目以降は
    /// 何もせず、総数だけを返す。
    pub fn finish(&self) -> Result<u32, FfiError> {
        // 保存が終わるまで、ロックを持ち続ける（同時に呼ばれた `finish` が、保存前の総数を返さない）。
        let mut guard = self.writer();
        if let Some(writer) = guard.take() {
            if let Some(chunk) = writer.finish() {
                self.store.save_chunk(&self.id, &chunk)?;
                self.saved.fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(self.saved.load(Ordering::Relaxed))
    }

    /// これまでに保存したチャンクの数。
    pub fn saved_chunks(&self) -> u32 {
        self.saved.load(Ordering::Relaxed)
    }
}

fn finished() -> FfiError {
    FfiError::Ingest {
        msg: "この録音は終了しています".into(),
    }
}
