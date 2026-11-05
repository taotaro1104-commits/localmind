//! 音声の基本: サンプルレート検証、サンプル数 ⇔ ミリ秒の換算。

use crate::error::IngestError;

/// 受け付ける最大サンプルレート。
pub const MAX_SAMPLE_RATE: u32 = 192_000;
/// 1 チャンクの標準の長さ（秒）。クラッシュ時に失う最大量でもある。
pub const DEFAULT_CHUNK_SECONDS: u32 = 30;
/// 1 チャンクの最大の長さ（秒）。保存データの検証に使う。
pub const MAX_CHUNK_SECONDS: u32 = 600;

/// サンプルレートが妥当か確認する。
pub fn validate_sample_rate(rate: u32) -> Result<(), IngestError> {
    if (1..=MAX_SAMPLE_RATE).contains(&rate) {
        Ok(())
    } else {
        Err(IngestError::InvalidFormat(format!(
            "サンプルレートは 1〜{MAX_SAMPLE_RATE} Hz でなければなりません: {rate}"
        )))
    }
}

/// サンプル数をミリ秒へ（切り捨て）。`rate` は `validate_sample_rate` 済みであること。
pub fn samples_to_ms(samples: u64, rate: u32) -> u64 {
    samples.saturating_mul(1000) / u64::from(rate.max(1))
}

/// ミリ秒をサンプル数へ（切り捨て）。
pub fn ms_to_samples(ms: u64, rate: u32) -> usize {
    let n = ms.saturating_mul(u64::from(rate)) / 1000;
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// `HH:MM:SS` 形式の時刻表記。
pub fn timestamp_label(ms: u64) -> String {
    let total_seconds = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}",
        total_seconds / 3600,
        (total_seconds / 60) % 60,
        total_seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_bounds() {
        assert!(validate_sample_rate(16_000).is_ok());
        assert!(validate_sample_rate(1).is_ok());
        assert!(validate_sample_rate(MAX_SAMPLE_RATE).is_ok());
        assert!(validate_sample_rate(0).is_err());
        assert!(validate_sample_rate(MAX_SAMPLE_RATE + 1).is_err());
    }

    #[test]
    fn conversions_round_trip_on_whole_milliseconds() {
        assert_eq!(samples_to_ms(16_000, 16_000), 1000);
        assert_eq!(ms_to_samples(1000, 16_000), 16_000);
        assert_eq!(samples_to_ms(8, 8_000), 1);
        assert_eq!(ms_to_samples(1, 8_000), 8);
    }

    #[test]
    fn conversions_do_not_overflow() {
        // 乗算は飽和するので、巨大な入力でもパニックしない。
        assert_eq!(samples_to_ms(u64::MAX, 1), u64::MAX);
        assert!(ms_to_samples(u64::MAX, MAX_SAMPLE_RATE) > 0);
        assert_eq!(samples_to_ms(10, 0), 10_000); // rate 0 は 1 として扱い、除算でパニックしない
    }

    #[test]
    fn timestamp_labels() {
        assert_eq!(timestamp_label(0), "00:00:00");
        assert_eq!(timestamp_label(83_000), "00:01:23");
        assert_eq!(timestamp_label(3_723_999), "01:02:03");
    }
}
