//! チャンクファイルの形式（ヘッダ・CRC-32・エンコード・デコード）。
//!
//! `[マジック 4][版 4][サンプルレート 4][連番 4][開始位置 8][サンプル数 8][CRC-32 4][PCM 本体]`（リトルエンディアン）。
//! CRC-32 はヘッダ（CRC 自身を除く先頭 32 バイト）と本体の両方を対象にする（版 3）。
//! デコードはヘッダの値を実ファイルの長さと突き合わせてから確保する（巨大確保・途中で切れたデータの防止）。

use super::MAX_START_MS;
use crate::audio::{validate_sample_rate, MAX_CHUNK_SECONDS, MAX_SAMPLE_RATE};
use crate::chunk::Chunk;
use crate::error::IngestError;

const MAGIC: &[u8; 4] = b"LMCH";
/// 版 3。旧版（1・2）のファイルは読まない（製品版の前に形式を確定させたので、移行処理は持たない）。
const VERSION: u32 = 3;
/// マジック 4 + 版 4 + サンプルレート 4 + 連番 4 + 開始位置 8 + サンプル数 8 + CRC-32 4。
pub(super) const HEADER_LEN: usize = 36;
/// 正当なチャンクファイルの最大サイズ（ヘッダ + 最大レート × 最大長 × 2 バイト）。
pub(super) const MAX_FILE_BYTES: u64 =
    HEADER_LEN as u64 + MAX_SAMPLE_RATE as u64 * MAX_CHUNK_SECONDS as u64 * 2;
/// CRC-32（IEEE 802.3）。ビット腐敗の検出用で、改ざん対策ではない。
#[cfg(test)]
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    crc32_parts(&[bytes])
}

/// 複数の断片を連結したものの CRC-32（連結のための複製をしない）。
fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for &b in parts.iter().flat_map(|p| p.iter()) {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// CRC の対象にするヘッダの長さ（CRC フィールドより前の全て）。
const CRC_COVERED_HEADER: usize = HEADER_LEN - 4;

pub(super) fn encode(chunk: &Chunk) -> Vec<u8> {
    let body: Vec<u8> = chunk.samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&chunk.sample_rate.to_le_bytes());
    out.extend_from_slice(&chunk.index.to_le_bytes());
    out.extend_from_slice(&chunk.start_ms.to_le_bytes());
    out.extend_from_slice(&(chunk.samples.len() as u64).to_le_bytes());
    // CRC はヘッダ（CRC 自身を除く）と本体の両方を守る。サンプルレートや開始位置の腐敗も検出する。
    let crc = crc32_parts(&[&out, &body]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap_or([0; 4]))
}

fn le_u64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))
}

pub(super) fn corrupt(msg: impl Into<String>) -> IngestError {
    IngestError::Corrupt(msg.into())
}

/// ヘッダを検証してチャンクを組み立てる前の、数値の取り出しと検証。
struct Header {
    sample_rate: u32,
    index: u32,
    start_ms: u64,
    count: u64,
    crc: u32,
}

fn parse_header(bytes: &[u8], expected_index: u32) -> Result<Header, IngestError> {
    if bytes.len() < HEADER_LEN {
        return Err(corrupt("ヘッダが途中で切れています"));
    }
    if &bytes[0..4] != MAGIC {
        return Err(corrupt("マジックナンバーが一致しません"));
    }
    if le_u32(&bytes[4..8]) != VERSION {
        return Err(corrupt("未対応の版です"));
    }
    let header = Header {
        sample_rate: le_u32(&bytes[8..12]),
        index: le_u32(&bytes[12..16]),
        start_ms: le_u64(&bytes[16..24]),
        count: le_u64(&bytes[24..32]),
        crc: le_u32(&bytes[32..36]),
    };
    validate_sample_rate(header.sample_rate).map_err(|e| corrupt(e.to_string()))?;
    if header.index != expected_index {
        return Err(corrupt(format!(
            "連番が一致しません: ファイル名 {expected_index} / ヘッダ {}",
            header.index
        )));
    }
    if header.start_ms > MAX_START_MS {
        return Err(corrupt(format!(
            "開始位置が範囲外です: {} ms",
            header.start_ms
        )));
    }
    Ok(header)
}

/// ファイル内容をチャンクへ。ヘッダの件数が実サイズと一致しなければ拒否し、CRC も照合する。
pub(super) fn decode(bytes: &[u8], expected_index: u32) -> Result<Chunk, IngestError> {
    let h = parse_header(bytes, expected_index)?;
    let max_count = u64::from(h.sample_rate) * u64::from(MAX_CHUNK_SECONDS);
    let body = &bytes[HEADER_LEN..];
    if h.count > max_count || body.len() as u64 != h.count * 2 {
        return Err(corrupt(format!(
            "サンプル数 {} が本体 {} バイトと一致しません（途中で切れた・余分なデータがある可能性）",
            h.count,
            body.len()
        )));
    }
    if crc32_parts(&[&bytes[..CRC_COVERED_HEADER], body]) != h.crc {
        return Err(corrupt(
            "チェックサムが一致しません（データが壊れています）",
        ));
    }
    let (pairs, _) = body.as_chunks::<2>();
    Ok(Chunk {
        index: h.index,
        start_ms: h.start_ms,
        sample_rate: h.sample_rate,
        samples: pairs.iter().map(|&b| i16::from_le_bytes(b)).collect(),
    })
}
