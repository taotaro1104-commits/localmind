//! 保存済みの録音の文字起こし（whisper.cpp）。feature `whisper` で有効。
//!
//! モデルは、固定した SHA-256 と一致する内容だけを読み込む。文字起こしは録音を 30 秒のチャンクごとに、
//! VAD で切り出した発話の区間だけに行い、録音全体の時刻つきのセグメントで返す。
//! 重い処理なので、UI スレッドから呼ばないこと。進み具合を [`SpeechProgress`] で受け取れる。
//! [`SpeechEngine::cancel`] は別のスレッドから呼べて、実行中の推論も途中で止める。

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::error::FfiError;
use crate::recording::RecordingStore;
use crate::search::check_abs_path;
use lm_ingest::{transcribe::transcribe_chunk, EnergyVad, Stage, Transcriber, VadConfig};
use lm_whisper::{OpenError, Quality, WhisperConfig, WhisperTranscriber};

/// スレッド数の上限（端末の発熱と、ほかの処理との競合を抑える）。
const MAX_THREADS: u32 = 8;

/// 探索の質。
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SpeechQuality {
    /// 速い（録音中・仮の結果向け）。
    Fast,
    /// 遅いが正確（充電中の再処理向け）。
    Accurate,
}

/// 文字起こしの 1 区間。時刻は録音の開始からのミリ秒。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SpokenSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// 0.0〜1.0。
    pub confidence: f32,
}

/// 文字起こしの結果。
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SpeechResult {
    /// 録音全体の時刻つきで、時刻順。
    pub segments: Vec<SpokenSegment>,
    /// 読めなかったチャンクの数（欠け・破損・復元の上限）。0 でなければ、結果は録音の一部だけ。
    pub skipped_chunks: u32,
    /// 途中で止めたか（`cancel` か、進み具合の通知が `false` を返した）。止めたなら、それまでの結果。
    pub cancelled: bool,
    /// 使ったモデルの識別子（再処理の要否の判断に使う）。
    pub model_id: String,
    /// 高精度の再処理（`Accurate`）の結果か。`false` なら、仮の結果（`Fast`）。
    pub refined: bool,
}

/// 文字起こしの進み具合を受け取る（Kotlin 側が実装する）。
#[uniffi::export(with_foreign)]
pub trait SpeechProgress: Send + Sync {
    /// `done` 個目のチャンクが終わった（全部で `total` 個。始める前にも `done = 0` で 1 度呼ばれる）。
    /// `false` を返すと、そこで止める（それまでの結果は返る）。
    fn on_progress(&self, done: u32, total: u32) -> bool;
}

#[derive(uniffi::Object)]
pub struct SpeechEngine {
    inner: Box<dyn Transcriber>,
    cancel: Arc<AtomicBool>,
    refined: bool,
}

impl SpeechEngine {
    pub(crate) fn from_transcriber(
        inner: Box<dyn Transcriber>,
        cancel: Arc<AtomicBool>,
        refined: bool,
    ) -> Self {
        Self {
            inner,
            cancel,
            refined,
        }
    }
}

#[uniffi::export]
impl SpeechEngine {
    /// モデルを読み込む。`sha256` は小文字の 16 進 64 文字。内容が一致しないモデルは読み込まない
    /// （`FfiError::Integrity`。入れ直せば直る）。`language` は `ja` など、`threads` は 1〜8。
    /// 読み込みに時間とメモリがかかる（検証のため、一時的にモデルの約 2 倍）。同時に 2 つ開かないこと。
    #[uniffi::constructor]
    pub fn open(
        model_path: String,
        sha256: String,
        language: String,
        quality: SpeechQuality,
        threads: u32,
    ) -> Result<Arc<Self>, FfiError> {
        check_abs_path(&model_path, "モデルのパス")?;
        if !(1..=MAX_THREADS).contains(&threads) {
            return Err(FfiError::Ingest {
                msg: format!("スレッド数は 1〜{MAX_THREADS} でなければなりません"),
            });
        }
        let config = WhisperConfig {
            language,
            quality: match quality {
                SpeechQuality::Fast => Quality::Fast,
                SpeechQuality::Accurate => Quality::Accurate,
            },
            threads: threads as usize,
        };
        // ネイティブの初期化の失敗は、パニックでも FFI の外へ出さない。
        let opened =
            catch_unwind(|| WhisperTranscriber::open(Path::new(&model_path), &sha256, config))
                .map_err(|_| FfiError::Ingest {
                    msg: "文字起こしエンジンの初期化に失敗しました".into(),
                })?;
        let engine = opened.map_err(|e| match e {
            OpenError::Integrity(msg) => FfiError::Integrity { msg },
            OpenError::InvalidConfig(inner) => inner.into(),
            OpenError::Engine(msg) => FfiError::Ingest { msg },
        })?;
        let cancel = engine.cancel_flag();
        Ok(Arc::new(Self::from_transcriber(
            Box::new(engine),
            cancel,
            quality == SpeechQuality::Accurate,
        )))
    }

    /// 実行中の文字起こしを止める（別のスレッドから呼べる。推論の途中でも止まる）。止めた文字起こしは、
    /// それまでの結果を `cancelled = true` で返す。次の `transcribe_recording` の始めに、止める指示は解除される。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 保存済みの録音を文字起こしする。録音が無ければ空の結果。
    /// 録音は 30 秒のチャンクごとに処理し、進み具合を `progress` へ（`None` なら通知しない）。
    pub fn transcribe_recording(
        &self,
        store: Arc<RecordingStore>,
        id: String,
        progress: Option<Arc<dyn SpeechProgress>>,
    ) -> Result<SpeechResult, FfiError> {
        let recovery = store.fs().load_chunks(&id)?;
        let vad = EnergyVad::new(VadConfig::default())?;
        let stage = if self.refined {
            Stage::Refined
        } else {
            Stage::Draft
        };
        let total = u32::try_from(recovery.chunks.len()).unwrap_or(u32::MAX);
        let notify = |done: u32| progress.as_ref().is_none_or(|p| p.on_progress(done, total));

        self.cancel.store(false, Ordering::Relaxed);
        let mut segments = Vec::new();
        let mut cancelled = !notify(0);
        for (i, chunk) in recovery.chunks.iter().enumerate() {
            if cancelled || self.cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            // ネイティブの推論は、パニックでも FFI の外へ出さない。
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                transcribe_chunk(chunk, &vad, self.inner.as_ref(), stage)
            }))
            .map_err(|_| FfiError::Ingest {
                msg: "文字起こしの途中で失敗しました".into(),
            })?;
            match outcome {
                Ok(found) => segments.extend(found.into_iter().map(|s| SpokenSegment {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                    confidence: s.confidence,
                })),
                // 止めた結果のエラーは、失敗ではなく中止として扱う（それまでの結果を返す）。
                Err(_) if self.cancel.load(Ordering::Relaxed) => {
                    cancelled = true;
                    break;
                }
                Err(e) => return Err(e.into()),
            }
            cancelled = !notify(i as u32 + 1);
        }
        Ok(SpeechResult {
            segments,
            skipped_chunks: u32::try_from(recovery.skipped.len()).unwrap_or(u32::MAX),
            cancelled: cancelled || self.cancel.load(Ordering::Relaxed),
            model_id: self.inner.id(),
            refined: self.refined,
        })
    }
}
