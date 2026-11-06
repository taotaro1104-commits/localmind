//! whisper.cpp による文字起こし。`lm_ingest::Transcriber` の実装（A-4）。
//!
//! 録音中は小さなモデル + 貪欲探索で仮の文字起こし（`Stage::Draft`）を、充電中は大きなモデル + ビーム探索で
//! 高精度の再処理（`Stage::Refined`）を行う想定。どちらも同じ型で、モデルのファイルと [`Quality`] だけが違う。
//!
//! モデルは、固定した SHA-256 と一致する内容だけを読み込む（[`model::load_verified`]）。ファイルを 1 回だけ開いて
//! 検証した内容そのものから初期化するので、検証と読み込みの間に差し替えられても、検証していない内容は読まれない。
//! ネットワークには出ない。推論は [`WhisperTranscriber::cancel_flag`] で、途中でも止められる。

pub mod audio;
pub mod model;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lm_ingest::{IngestError, Transcriber, Utterance};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use model::ModelError;

/// 無音とみなす確率（セグメントの `no_speech_probability` がこれを超えたら捨てる）。
const NO_SPEECH_LIMIT: f32 = 0.6;
/// 既定のスレッド数の上限（端末の発熱と、録音との競合を抑える）。
const MAX_DEFAULT_THREADS: usize = 4;

/// 探索の質。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    /// 貪欲探索（候補 1 つ）。速い（録音中の仮の結果向け）。
    Fast,
    /// ビーム探索（幅 5）。遅いが正確（充電中の再処理向け）。
    Accurate,
}

/// 文字起こしの設定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperConfig {
    /// 言語コード（`ja` など）。
    pub language: String,
    pub quality: Quality,
    /// 推論のスレッド数（1 以上）。
    pub threads: usize,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, usize::from);
        Self {
            language: "ja".into(),
            quality: Quality::Fast,
            threads: cores.clamp(1, MAX_DEFAULT_THREADS),
        }
    }
}

impl WhisperConfig {
    fn validate(&self) -> Result<(), IngestError> {
        let lang_ok = (2..=3).contains(&self.language.len())
            && self.language.bytes().all(|b| b.is_ascii_lowercase());
        if !lang_ok {
            return Err(IngestError::InvalidFormat(format!(
                "言語コードは小文字の英字 2〜3 文字で指定してください: {:?}",
                self.language.chars().take(8).collect::<String>()
            )));
        }
        if !(1..=64).contains(&self.threads) {
            return Err(IngestError::InvalidFormat(
                "スレッド数は 1〜64 でなければなりません".into(),
            ));
        }
        Ok(())
    }
}

/// エンジンを開けない理由。呼び出し側が、入れ直しで直るもの（`Integrity`）とそうでないものを型で見分けられる。
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// 設定・引数の誤り。
    InvalidConfig(IngestError),
    /// モデルファイルが無い・大きさが違う・内容が固定したハッシュと一致しない。モデルを入れ直せば直る。
    Integrity(String),
    /// モデルは正しいが、エンジンを初期化できない（形式が違う・メモリが足りない）。
    Engine(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(e) => write!(f, "{e}"),
            Self::Integrity(m) | Self::Engine(m) => f.write_str(m),
        }
    }
}

impl From<OpenError> for IngestError {
    fn from(e: OpenError) -> Self {
        match e {
            OpenError::InvalidConfig(inner) => inner,
            OpenError::Integrity(m) | OpenError::Engine(m) => IngestError::Transcribe(m),
        }
    }
}

/// whisper.cpp のエンジン。モデルを 1 つ読み込み、呼び出しを 1 つずつ直列に処理する。
pub struct WhisperTranscriber {
    state: Mutex<WhisperState>,
    config: WhisperConfig,
    id: String,
    cancel: Arc<AtomicBool>,
}

impl std::fmt::Debug for WhisperTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperTranscriber")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn engine_error(what: &str) -> IngestError {
    IngestError::Transcribe(what.into())
}

impl WhisperTranscriber {
    /// モデルを読み込む。`expected_sha256`（小文字の 16 進 64 文字）と一致しない内容は読み込まない。
    /// 読み込みには時間とメモリがかかる（検証用にモデルを丸ごとメモリへ読むので、一時的にモデルの約 2 倍）。
    /// UI スレッドから呼ばないこと。
    pub fn open(
        model_path: &Path,
        expected_sha256: &str,
        config: WhisperConfig,
    ) -> Result<Self, OpenError> {
        config.validate().map_err(OpenError::InvalidConfig)?;
        let bytes = model::load_verified(model_path, expected_sha256).map_err(|e| match e {
            ModelError::InvalidArgument(m) => {
                OpenError::InvalidConfig(IngestError::InvalidFormat(m))
            }
            ModelError::Integrity(m) => OpenError::Integrity(m),
        })?;
        let ctx = WhisperContext::new_from_buffer_with_params(
            &bytes,
            WhisperContextParameters::default(),
        )
        .map_err(|_| {
            OpenError::Engine("モデルを読み込めません（形式が違うか、メモリが足りません）".into())
        })?;
        drop(bytes); // 初期化が済んだら、検証用のバッファは解放する
        let state = ctx.create_state().map_err(|_| {
            OpenError::Engine("推論の準備に失敗しました（メモリが足りません）".into())
        })?;
        Ok(Self {
            state: Mutex::new(state),
            id: format!(
                "whisper.cpp:{}:{:?}",
                &expected_sha256[..12],
                config.quality
            ),
            config,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 中止の旗。別のスレッドから `true` にすると、実行中の推論も（計算の途中で）止まり、`transcribe` は
    /// エラーを返す。使う側が、始める前に `false` へ戻すこと。
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    fn params(&self) -> FullParams<'_, '_> {
        let strategy = match self.config.quality {
            Quality::Fast => SamplingStrategy::Greedy { best_of: 1 },
            Quality::Accurate => SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0,
            },
        };
        let mut p = FullParams::new(strategy);
        p.set_language(Some(&self.config.language));
        p.set_n_threads(self.config.threads as std::ffi::c_int);
        p.set_translate(false);
        p.set_no_context(true); // 発話ごとに独立（前の発話の誤りを引きずらない）
        p.set_suppress_blank(true);
        p.set_suppress_nst(true);
        p.set_temperature(0.0);
        p.set_print_special(false);
        p.set_print_progress(false);
        p.set_print_realtime(false);
        p.set_print_timestamps(false);
        let cancel = self.cancel.clone();
        let abort: Box<dyn FnMut() -> bool> = Box::new(move || cancel.load(Ordering::Relaxed));
        p.set_abort_callback_safe::<_, Box<dyn FnMut() -> bool>>(Some(abort));
        p
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, pcm: &[i16], sample_rate: u32) -> Result<Utterance, IngestError> {
        let input = audio::to_whisper_input(pcm, sample_rate)?;
        if input.is_empty() {
            return Ok(Utterance {
                text: String::new(),
                confidence: 0.0,
            });
        }
        // 前の推論が途中で失敗してロックが汚れていても、`full` は呼ぶたびに状態を初期化するので、続けて使える。
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(_e) = state.full(self.params(), &input) {
            return Err(engine_error(if self.cancel.load(Ordering::Relaxed) {
                "中止されました"
            } else {
                "推論に失敗しました"
            }));
        }

        let (mut text, mut prob_sum, mut prob_n) = (String::new(), 0.0f32, 0u32);
        for segment in state.as_iter() {
            if segment.no_speech_probability() > NO_SPEECH_LIMIT {
                continue;
            }
            text.push_str(
                segment
                    .to_str_lossy()
                    .map_err(|_| engine_error("結果を読めません"))?
                    .as_ref(),
            );
            for i in 0..segment.n_tokens() {
                let Some(token) = segment.get_token(i) else {
                    continue;
                };
                // 特殊トークン（`[_BEG_]` など）は、信頼度に数えない。
                let special = token.to_str_lossy().map_or(true, |t| t.starts_with("[_"));
                if !special {
                    prob_sum += token.token_probability();
                    prob_n += 1;
                }
            }
        }
        let confidence = if prob_n == 0 {
            0.0
        } else {
            prob_sum / prob_n as f32
        };
        Ok(Utterance {
            text: text.trim().to_string(),
            confidence,
        })
    }

    fn id(&self) -> String {
        self.id.clone()
    }
}

#[cfg(test)]
mod tests;
