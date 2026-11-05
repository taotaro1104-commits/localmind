//! 録音を一定長のチャンクに分割する。チャンクは録音中に逐次保存され、クラッシュ時は
//! 直前のチャンクまで復元できる（失うのは最大でチャンク 1 つ分）。

use crate::audio::{samples_to_ms, validate_sample_rate, DEFAULT_CHUNK_SECONDS, MAX_CHUNK_SECONDS};
use crate::error::IngestError;

/// `ChunkWriter::push` に 1 度に渡せる音声の長さ（秒）。
pub const MAX_PUSH_SECONDS: u64 = 600;

/// 録音の 1 区間（モノラル PCM 16bit）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 0 始まりの連番。
    pub index: u32,
    /// 録音開始からの開始位置（ミリ秒）。サンプル数から算出するので誤差が累積しない。
    pub start_ms: u64,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl Chunk {
    pub fn duration_ms(&self) -> u64 {
        samples_to_ms(self.samples.len() as u64, self.sample_rate)
    }

    /// 終了位置（排他）。異常に大きい開始位置でもあふれず、上限で止まる。
    pub fn end_ms(&self) -> u64 {
        self.start_ms.saturating_add(self.duration_ms())
    }
}

/// PCM を受け取り、`chunk_seconds` ごとに `Chunk` を切り出す。
#[derive(Debug)]
pub struct ChunkWriter {
    sample_rate: u32,
    chunk_samples: usize,
    buffer: Vec<i16>,
    next_index: u32,
    emitted_samples: u64,
}

impl ChunkWriter {
    pub fn new(sample_rate: u32, chunk_seconds: u32) -> Result<Self, IngestError> {
        validate_sample_rate(sample_rate)?;
        if !(1..=MAX_CHUNK_SECONDS).contains(&chunk_seconds) {
            return Err(IngestError::InvalidFormat(format!(
                "チャンク長は 1〜{MAX_CHUNK_SECONDS} 秒でなければなりません: {chunk_seconds}"
            )));
        }
        Ok(Self {
            sample_rate,
            chunk_samples: sample_rate as usize * chunk_seconds as usize,
            buffer: Vec::new(),
            next_index: 0,
            emitted_samples: 0,
        })
    }

    /// 標準のチャンク長（`DEFAULT_CHUNK_SECONDS` = 30 秒）で作る。
    pub fn standard(sample_rate: u32) -> Result<Self, IngestError> {
        Self::new(sample_rate, DEFAULT_CHUNK_SECONDS)
    }

    fn emit(&mut self, samples: Vec<i16>) -> Chunk {
        let chunk = Chunk {
            index: self.next_index,
            start_ms: samples_to_ms(self.emitted_samples, self.sample_rate),
            sample_rate: self.sample_rate,
            samples,
        };
        self.next_index = self.next_index.saturating_add(1); // 保存側が MAX_CHUNK_INDEX 超を拒否する
        self.emitted_samples = self
            .emitted_samples
            .saturating_add(chunk.samples.len() as u64);
        chunk
    }

    /// PCM を追加し、満たされたチャンクを返す（0 個以上）。
    /// 1 回の入力は `MAX_PUSH_SECONDS` 秒分まで（巨大な入力でメモリを使い切らないように。超えたら分けて渡す）。
    pub fn push(&mut self, pcm: &[i16]) -> Result<Vec<Chunk>, IngestError> {
        let limit = u64::from(self.sample_rate) * MAX_PUSH_SECONDS;
        if pcm.len() as u64 > limit {
            return Err(IngestError::InvalidFormat(format!(
                "1 回に渡せる音声は {MAX_PUSH_SECONDS} 秒分までです: {} サンプル",
                pcm.len()
            )));
        }
        self.buffer.extend_from_slice(pcm);
        // 先頭から必要な分だけ取り出し、端数は最後に 1 回だけ詰め直す（巨大な入力でも二乗にならない）。
        let n = self.chunk_samples;
        let parts: Vec<Vec<i16>> = self.buffer.chunks_exact(n).map(<[i16]>::to_vec).collect();
        self.buffer.drain(..parts.len() * n);
        let mut full = Vec::with_capacity(parts.len());
        for samples in parts {
            full.push(self.emit(samples));
        }
        Ok(full)
    }

    /// 録音停止。バッファに残った端数を最後のチャンクとして返す。
    pub fn finish(mut self) -> Option<Chunk> {
        if self.buffer.is_empty() {
            return None;
        }
        let samples = std::mem::take(&mut self.buffer);
        Some(self.emit(samples))
    }

    /// まだチャンクにならず、バッファ内にある長さ（ミリ秒）。
    pub fn buffered_ms(&self) -> u64 {
        samples_to_ms(self.buffer.len() as u64, self.sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(n: usize) -> Vec<i16> {
        (0..n).map(|i| (i % 100) as i16).collect()
    }

    #[test]
    fn standard_chunks_are_thirty_seconds() {
        let mut w = ChunkWriter::standard(1000).unwrap();
        assert!(w.push(&pcm(29_999)).unwrap().is_empty());
        let chunks = w.push(&pcm(1)).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].duration_ms(), 30_000);
        assert!(ChunkWriter::standard(0).is_err());
    }

    #[test]
    fn end_position_saturates_instead_of_overflowing() {
        let c = Chunk {
            index: 0,
            start_ms: u64::MAX - 5,
            sample_rate: 1000,
            samples: vec![0; 5000],
        };
        assert_eq!(c.end_ms(), u64::MAX);
    }

    #[test]
    fn rejects_invalid_settings() {
        assert!(ChunkWriter::new(0, 30).is_err());
        assert!(ChunkWriter::new(16_000, 0).is_err());
        assert!(ChunkWriter::new(16_000, MAX_CHUNK_SECONDS + 1).is_err());
        assert!(ChunkWriter::new(16_000, 30).is_ok());
    }

    #[test]
    fn emits_a_chunk_each_time_the_buffer_fills() {
        let mut w = ChunkWriter::new(1000, 2).unwrap(); // 2 秒 = 2000 サンプル
        assert!(w.push(&pcm(1500)).unwrap().is_empty());
        assert_eq!(w.buffered_ms(), 1500);
        let chunks = w.push(&pcm(2600)).unwrap(); // 合計 4100 → 2 チャンク + 100 サンプル
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].start_ms, 0);
        assert_eq!(chunks[1].index, 1);
        assert_eq!(chunks[1].start_ms, 2000);
        assert!(chunks.iter().all(|c| c.samples.len() == 2000));
        assert_eq!(w.buffered_ms(), 100);
    }

    #[test]
    fn no_samples_are_lost_or_duplicated() {
        let input = pcm(5321);
        let mut w = ChunkWriter::new(1000, 1).unwrap();
        let mut out = Vec::new();
        for piece in input.chunks(333) {
            for c in w.push(piece).unwrap() {
                out.extend(c.samples);
            }
        }
        if let Some(last) = w.finish() {
            out.extend(last.samples);
        }
        assert_eq!(out, input);
    }

    #[test]
    fn finish_returns_the_partial_tail_with_correct_position() {
        let mut w = ChunkWriter::new(1000, 2).unwrap();
        w.push(&pcm(2500)).unwrap();
        let tail = w.finish().unwrap();
        assert_eq!(tail.index, 1);
        assert_eq!(tail.start_ms, 2000);
        assert_eq!(tail.samples.len(), 500);
        assert_eq!(tail.duration_ms(), 500);
        assert_eq!(tail.end_ms(), 2500);
    }

    #[test]
    fn finish_on_an_empty_or_exact_recording_returns_none() {
        assert!(ChunkWriter::new(1000, 2).unwrap().finish().is_none());
        let mut w = ChunkWriter::new(1000, 2).unwrap();
        assert_eq!(w.push(&pcm(2000)).unwrap().len(), 1);
        assert!(w.finish().is_none());
    }

    #[test]
    fn start_positions_do_not_drift_with_odd_sample_rates() {
        // 44.1kHz で 1 秒チャンクを 1000 個作っても、開始位置は常に index × 1000ms。
        let mut w = ChunkWriter::new(44_100, 1).unwrap();
        let chunks = w.push(&vec![0i16; 44_100 * 50]).unwrap();
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.start_ms, i as u64 * 1000);
        }
    }

    #[test]
    fn an_oversized_push_is_refused_and_leaves_the_writer_untouched() {
        let mut w = ChunkWriter::new(1000, 1).unwrap();
        let too_big = vec![0i16; (1000 * MAX_PUSH_SECONDS) as usize + 1];
        assert!(matches!(
            w.push(&too_big),
            Err(IngestError::InvalidFormat(_))
        ));
        assert_eq!(w.buffered_ms(), 0, "何も取り込まれていない");
        let just_fits = vec![0i16; (1000 * MAX_PUSH_SECONDS) as usize];
        assert_eq!(w.push(&just_fits).unwrap().len(), MAX_PUSH_SECONDS as usize);
    }

    #[test]
    fn empty_push_is_a_no_op() {
        let mut w = ChunkWriter::new(1000, 1).unwrap();
        assert!(w.push(&[]).unwrap().is_empty());
        assert_eq!(w.buffered_ms(), 0);
    }
}
