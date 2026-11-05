//! エネルギーベースの VAD（無音区間検出）。無音を文字起こしに渡さず、処理時間と電池を削る。
//!
//! 自動閾値が分離できるのは、発話が背景雑音より約 12dB（振幅比 4 倍）以上大きいとき。それより
//! 近い場合は背景も発話扱いになり、無音のカットが効かず文字起こしの処理量が増える（取りこぼしは起きない）。
//!
//! フレームごとの RMS を閾値と比べ、(1) 短い無音は発話の途切れとして結合、(2) 短すぎる発話は雑音として捨て、
//! (3) 前後に余白を付ける。閾値はノイズフロア（RMS の下位 10%）から自動で引き上げられる。

use crate::audio::{samples_to_ms, validate_sample_rate};
use crate::error::IngestError;

/// 発話区間（チャンク先頭からのミリ秒、`end_ms` は排他）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl SpeechSpan {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadConfig {
    /// 解析フレーム長（ミリ秒）。
    pub frame_ms: u32,
    /// 発話とみなす RMS の下限（i16 の振幅）。静かな部屋の小声（RMS 100 台）も拾えるよう低く取る。
    pub min_threshold: f32,
    /// ノイズフロア × この倍率が `min_threshold` を超えるなら、閾値を引き上げる。0 以下で自動調整なし。
    pub noise_ratio: f32,
    /// これより短い発話は雑音として捨てる。
    pub min_speech_ms: u32,
    /// これより短い無音は発話の途切れとして結合する。
    pub min_silence_ms: u32,
    /// 発話の前後に付ける余白。
    pub pad_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            frame_ms: 20,
            min_threshold: 120.0,
            noise_ratio: 3.0,
            min_speech_ms: 200,
            min_silence_ms: 300,
            pad_ms: 100,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EnergyVad {
    config: VadConfig,
}

/// 自動で引き上げる閾値の上限（発話の大きさに対する比）。
const MAX_THRESHOLD_FRACTION: f32 = 0.25;

fn frame_rms(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: u64 = frame
        .iter()
        .map(|&s| (i64::from(s) * i64::from(s)) as u64)
        .sum();
    (sum as f64 / frame.len() as f64).sqrt() as f32
}

/// 昇順に並べた RMS の `q`（0.0〜1.0）分位点。空なら 0。
fn quantile(rms: &[f32], q: f32) -> f32 {
    if rms.is_empty() {
        return 0.0;
    }
    let mut sorted = rms.to_vec();
    sorted.sort_by(f32::total_cmp);
    sorted[((sorted.len() as f32 * q) as usize).min(sorted.len() - 1)]
}

/// 下位 10% 点の値（ノイズフロアの推定）。
fn noise_floor(rms: &[f32]) -> f32 {
    quantile(rms, 0.1)
}

/// 上位 5% 点の値（発話の大きさの推定）。録音の 5% 程度しか話していなくても、背景ではなく発話を指す。
fn loud_level(rms: &[f32]) -> f32 {
    quantile(rms, 0.95)
}

/// 連続する有声フレームの範囲 `[start, end)`（フレーム番号）。
fn voiced_runs(rms: &[f32], threshold: f32) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, &r) in rms.iter().enumerate() {
        match (r >= threshold, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push((s, rms.len()));
    }
    runs
}

impl EnergyVad {
    pub fn new(config: VadConfig) -> Result<Self, IngestError> {
        let finite = config.min_threshold.is_finite() && config.noise_ratio.is_finite();
        if config.frame_ms == 0 || !finite || config.min_threshold < 0.0 {
            return Err(IngestError::InvalidFormat(
                "VAD の frame_ms は 1 以上、閾値は有限の非負値でなければなりません".into(),
            ));
        }
        Ok(Self { config })
    }

    /// 発話とみなす RMS の下限。
    ///
    /// ノイズフロアの `noise_ratio` 倍まで引き上げるが、発話の大きさ（上位 5% 点）の
    /// `MAX_THRESHOLD_FRACTION` 倍を超えない。
    /// 上限が無いと、チャンク全体が発話で無音を含まないとき（30 秒続けて話した場合など）に
    /// 「ノイズフロア」が発話そのものになり、発話を無音と誤判定して文字起こしが丸ごと失われる。
    /// 上限が大きすぎると、声の小さい区間が無音として削られて発話が分断される。
    fn threshold(&self, rms: &[f32]) -> f32 {
        let c = &self.config;
        if c.noise_ratio > 0.0 {
            let raised =
                (noise_floor(rms) * c.noise_ratio).min(loud_level(rms) * MAX_THRESHOLD_FRACTION);
            c.min_threshold.max(raised)
        } else {
            c.min_threshold
        }
    }

    /// 発話区間を時刻順に返す。互いに重ならず、各区間は `start_ms < end_ms`。
    ///
    /// 時刻はフレーム番号 × 実際のフレーム長（サンプル数）から換算する。`frame_ms` を掛けると、
    /// 11025Hz × 20ms = 220.5 サンプルのように割り切れないレートで、位置が累積してずれる。
    pub fn detect(
        &self,
        samples: &[i16],
        sample_rate: u32,
    ) -> Result<Vec<SpeechSpan>, IngestError> {
        validate_sample_rate(sample_rate)?;
        // チャンク 1 つ分を超える入力は拒否する（フレームの二乗和の溢れと巨大な確保を防ぐ）。
        let max_samples = u64::from(sample_rate) * u64::from(crate::audio::MAX_CHUNK_SECONDS);
        if samples.len() as u64 > max_samples {
            return Err(IngestError::InvalidFormat(format!(
                "VAD の入力が長すぎます: {} サンプル（上限 {max_samples}）",
                samples.len()
            )));
        }
        let c = &self.config;
        let frame_len = ((u64::from(sample_rate) * u64::from(c.frame_ms)) / 1000).max(1);
        // 32bit 環境で `as usize` が 0 に切り捨てられて `chunks(0)` が panic しないよう、変換は飽和させる。
        let step = usize::try_from(frame_len).unwrap_or(usize::MAX);
        let rms: Vec<f32> = samples.chunks(step).map(frame_rms).collect();
        let total_ms = samples_to_ms(samples.len() as u64, sample_rate);
        // フレーム境界（番号）の録音先頭からの位置。最後の端数フレームがあっても録音の長さを超えない。
        let ms_at = |frame: usize| {
            samples_to_ms((frame as u64).saturating_mul(frame_len), sample_rate).min(total_ms)
        };

        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in voiced_runs(&rms, self.threshold(&rms)) {
            match merged.last_mut() {
                Some(prev)
                    if ms_at(start).saturating_sub(ms_at(prev.1)) < u64::from(c.min_silence_ms) =>
                {
                    prev.1 = end;
                }
                _ => merged.push((start, end)),
            }
        }

        let pad = u64::from(c.pad_ms);
        let mut spans: Vec<SpeechSpan> = Vec::new();
        for (start, end) in merged {
            if ms_at(end).saturating_sub(ms_at(start)) < u64::from(c.min_speech_ms) {
                continue;
            }
            let span = SpeechSpan {
                start_ms: ms_at(start).saturating_sub(pad),
                end_ms: ms_at(end).saturating_add(pad).min(total_ms),
            };
            if span.start_ms >= span.end_ms {
                continue;
            }
            match spans.last_mut() {
                Some(prev) if span.start_ms <= prev.end_ms => prev.end_ms = span.end_ms,
                _ => spans.push(span),
            }
        }
        Ok(spans)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_limits;
