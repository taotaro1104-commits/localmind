//! 文字起こしパイプライン: チャンク → VAD で発話区間だけを切り出し → エンジンで文字起こし →
//! 録音全体での絶対時刻を持つセグメントへ。
//!
//! エンジン（whisper.cpp 等）は [`Transcriber`] の実装として差し込む。録音中は小型モデルで仮の
//! 文字起こし（[`Stage::Draft`]）を、充電中に大型モデルで再処理（[`Stage::Refined`]）する 2 段構成を想定する。

use crate::audio::ms_to_samples;
use crate::chunk::Chunk;
use crate::error::IngestError;
use crate::store::validate_chunk;
use crate::vad::EnergyVad;

/// 1 セグメントの文字数の上限。エンジンの異常で巨大な文字列が来ても、メモリを食い潰さない。
pub const MAX_SEGMENT_CHARS: usize = 10_000;

/// エンジンが返す 1 発話の結果。
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub text: String,
    /// 信頼度（0.0〜1.0 を想定。範囲外・非有限は丸める）。
    pub confidence: f32,
}

/// 文字起こしエンジン。呼び出しは音声の発話区間 1 つ分（モノラル PCM 16bit）。
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, pcm: &[i16], sample_rate: u32) -> Result<Utterance, IngestError>;
    /// モデルの識別子（結果とともに記録し、再処理の要否判断に使う）。
    fn id(&self) -> String;
}

/// 文字起こしの段階。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 録音中の小型モデルによる仮の結果。
    Draft,
    /// 充電中の大型モデルによる高精度の結果。
    Refined,
}

/// 録音全体の時間軸での 1 発話。
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSegment {
    pub chunk_index: u32,
    /// 録音開始からの開始位置（ミリ秒）。タップで該当音声を再生するのに使う。
    pub start_ms: u64,
    /// 終了位置（排他）。
    pub end_ms: u64,
    /// 話者ラベル（後から手動で命名する。未設定は None）。
    pub speaker: Option<String>,
    pub text: String,
    pub confidence: f32,
    pub stage: Stage,
}

fn sanitize_confidence(c: f32) -> f32 {
    if c.is_finite() {
        c.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 文字数の上限で切り詰める（char 境界を保つ）。
fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 1 チャンクを文字起こしする。発話区間だけをエンジンに渡し、無音は処理しない。
/// エンジンの失敗は呼び出し側に返す（そのチャンクを再実行できる）。結果は時刻順。
pub fn transcribe_chunk(
    chunk: &Chunk,
    vad: &EnergyVad,
    transcriber: &dyn Transcriber,
    stage: Stage,
) -> Result<Vec<TranscriptSegment>, IngestError> {
    // 保存・復元できるチャンクだけを扱う（サンプルレート・長さ・開始位置。時刻の加算があふれない）。
    validate_chunk(chunk)?;
    let spans = vad.detect(&chunk.samples, chunk.sample_rate)?;
    let mut segments = Vec::with_capacity(spans.len());
    for span in spans {
        let lo = ms_to_samples(span.start_ms, chunk.sample_rate).min(chunk.samples.len());
        let hi = ms_to_samples(span.end_ms, chunk.sample_rate).min(chunk.samples.len());
        if lo >= hi {
            continue;
        }
        let utterance = transcriber.transcribe(&chunk.samples[lo..hi], chunk.sample_rate)?;
        let text = truncate_chars(utterance.text.trim(), MAX_SEGMENT_CHARS);
        if text.is_empty() {
            continue; // 発話として認識されなかった区間
        }
        segments.push(TranscriptSegment {
            chunk_index: chunk.index,
            start_ms: chunk.start_ms + span.start_ms,
            end_ms: chunk.start_ms + span.end_ms,
            speaker: None,
            text,
            confidence: sanitize_confidence(utterance.confidence),
            stage,
        });
    }
    Ok(segments)
}

#[cfg(test)]
mod tests;
