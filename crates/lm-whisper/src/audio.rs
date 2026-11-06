//! whisper に渡す音声の整形（16kHz・モノラル・f32 へ）。

use lm_ingest::IngestError;

/// whisper.cpp が要求するサンプルレート。
pub const WHISPER_RATE: u32 = 16_000;
/// 1 回に渡せる音声の長さ（秒）。発話区間 1 つ分を想定し、巨大な入力で時間とメモリを使い切らない。
pub const MAX_SECONDS: u64 = 600;
/// 受け付けるサンプルレートの範囲。
const RATES: std::ops::RangeInclusive<u32> = 8_000..=96_000;

/// 16bit PCM を、-1.0〜1.0 の f32 にして、16kHz へそろえる。
///
/// 16kHz より高いレートは、出力の 1 サンプルに対応する範囲の平均を取って落とす（箱形の低域通過。単純な間引きや
/// 補間で起きる折り返しの雑音を抑える）。低いレートは線形補間で引き伸ばす。
pub fn to_whisper_input(pcm: &[i16], sample_rate: u32) -> Result<Vec<f32>, IngestError> {
    if !RATES.contains(&sample_rate) {
        return Err(IngestError::InvalidFormat(format!(
            "サンプルレートは {}〜{} Hz でなければなりません: {sample_rate}",
            RATES.start(),
            RATES.end()
        )));
    }
    if pcm.len() as u64 > u64::from(sample_rate) * MAX_SECONDS {
        return Err(IngestError::InvalidFormat(format!(
            "1 回に文字起こしできる音声は {MAX_SECONDS} 秒までです"
        )));
    }
    let float: Vec<f32> = pcm.iter().map(|&s| f32::from(s) / 32_768.0).collect();
    if sample_rate == WHISPER_RATE {
        return Ok(float);
    }
    let ratio = f64::from(sample_rate) / f64::from(WHISPER_RATE);
    let out_len = (float.len() as f64 / ratio).floor() as usize;
    Ok((0..out_len)
        .map(|i| {
            if ratio > 1.0 {
                let lo = (i as f64 * ratio).floor() as usize;
                let hi = (((i + 1) as f64 * ratio).ceil() as usize).clamp(lo + 1, float.len());
                let window = &float[lo.min(hi - 1)..hi];
                window.iter().sum::<f32>() / window.len() as f32
            } else {
                let pos = i as f64 * ratio;
                let lo = pos.floor() as usize;
                let hi = (lo + 1).min(float.len() - 1);
                let frac = (pos - lo as f64) as f32;
                float[lo] * (1.0 - frac) + float[hi] * frac
            }
        })
        .collect())
}

#[cfg(test)]
mod tests;
