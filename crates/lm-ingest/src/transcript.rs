//! 文字起こし結果の集合（議事録の土台）。
//!
//! - チャンク単位で登録・再登録できる（失敗したチャンクの再実行で重複しない）。
//! - 仮（Draft）→ 高精度（Refined）の再処理は、差分だけを返して更新する。話者名は引き継ぐ。
//! - 話者は後から手動で命名する（自動分離は実験機能で、ここでは扱わない）。
//! - 各セグメントは時刻で引ける（タップで該当音声を再生）。検索索引用の文書にも変換できる。
//!
//! 不変条件: セグメントは開始時刻の昇順で、互いに時間が重ならない（`start_ms < end_ms`）。
//! 登録時に検証し、違反する入力は状態を変えずに `InvalidFormat` で拒否する。

use crate::audio::timestamp_label;
use crate::error::IngestError;
use crate::store::validate_recording_id;
use crate::transcribe::{TranscriptSegment, MAX_SEGMENT_CHARS};

/// 再処理の差分。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefineDiff {
    /// 文面が変わったセグメント数。
    pub changed: usize,
    /// 旧結果に対応が無い、または対応先がすでに別の新セグメントに使われた新しいセグメント数
    /// （1 つの発話が複数に分割された場合の 2 つ目以降を含む）。
    pub added: usize,
    /// 新結果に対応が無くなった旧セグメント数。
    pub removed: usize,
    /// 文面が同じセグメント数。
    pub unchanged: usize,
}

impl RefineDiff {
    /// 何か更新があったか（通知するかの判断に使う）。
    pub fn has_changes(&self) -> bool {
        self.changed + self.added + self.removed > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    /// 開始時刻の昇順で、時間が重ならない。
    segments: Vec<TranscriptSegment>,
}

fn overlap_ms(a: &TranscriptSegment, b: &TranscriptSegment) -> u64 {
    a.end_ms
        .min(b.end_ms)
        .saturating_sub(a.start_ms.max(b.start_ms))
}

/// 登録するセグメントを検証して、開始時刻順に並べて返す。
/// 同じチャンクの番号、`start_ms < end_ms`、セグメント同士が重ならないこと。
fn prepare(
    chunk_index: u32,
    mut segments: Vec<TranscriptSegment>,
) -> Result<Vec<TranscriptSegment>, IngestError> {
    if segments.len() > MAX_SEGMENTS_PER_CHUNK {
        return Err(IngestError::InvalidFormat(format!(
            "1 チャンクのセグメント数が上限（{MAX_SEGMENTS_PER_CHUNK}）を超えています: {}",
            segments.len()
        )));
    }
    for s in &mut segments {
        // 本文・話者名は登録時に無害化する（索引・出力・引用 UI のどれも同じ内容になる）。
        s.text = one_line(&s.text);
        s.speaker = s.speaker.take().and_then(|name| normalize_speaker(&name));
        if s.chunk_index != chunk_index {
            return Err(IngestError::InvalidFormat(format!(
                "チャンク {chunk_index} に別チャンク {} のセグメントが含まれています",
                s.chunk_index
            )));
        }
        if s.text.trim().is_empty() || s.text.chars().count() > MAX_SEGMENT_CHARS {
            return Err(IngestError::InvalidFormat(format!(
                "セグメントの本文が空、または {MAX_SEGMENT_CHARS} 文字を超えています（{}〜{} ms）",
                s.start_ms, s.end_ms
            )));
        }
        if s.start_ms >= s.end_ms {
            return Err(IngestError::InvalidFormat(format!(
                "セグメントの時刻が不正です: {}〜{}",
                s.start_ms, s.end_ms
            )));
        }
    }
    segments.sort_by_key(|s| s.start_ms);
    if let Some(w) = segments.windows(2).find(|w| w[0].end_ms > w[1].start_ms) {
        return Err(IngestError::InvalidFormat(format!(
            "セグメントの時間が重なっています: {}〜{} と {}〜{}",
            w[0].start_ms, w[0].end_ms, w[1].start_ms, w[1].end_ms
        )));
    }
    Ok(segments)
}

/// 改行を空白にする。出力に 1 セグメント = 1 行を保たせ、本文による行の偽造を防ぐ。
/// 改行に限らず、全ての制御文字（ESC・VT・FF・NEL など）と行区切り（U+2028/2029）を空白にする。
pub(crate) fn one_line(text: &str) -> String {
    text.chars()
        .filter(|c| !is_invisible_format(*c))
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// 表示を偽装できる、または何も表示されない書式文字（ゼロ幅文字・双方向制御・BOM）。
pub(crate) fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{061C}'
            | '\u{00AD}'
            | '\u{034F}'
            | '\u{180E}'
            | '\u{115F}'..='\u{1160}'
            | '\u{3164}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// 1 チャンクに登録できるセグメント数の上限（照合が二乗になる入力を防ぐ）。
pub const MAX_SEGMENTS_PER_CHUNK: usize = 5000;

/// 話者名の最大文字数。
pub const MAX_SPEAKER_CHARS: usize = 64;

/// 話者名を整える（制御文字は空白に、長すぎる分は切り捨て）。
pub(crate) fn clean_speaker(name: &str) -> String {
    one_line(name).chars().take(MAX_SPEAKER_CHARS).collect()
}

/// 話者名を整える。整えた結果が空（空白のみ）なら `None`。登録・付け替え・設定の全経路で使う。
fn normalize_speaker(name: &str) -> Option<String> {
    Some(clean_speaker(name)).filter(|n| !n.trim().is_empty())
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// 最後のセグメントの終了位置（ミリ秒）。空なら 0。
    pub fn duration_ms(&self) -> u64 {
        self.segments.last().map_or(0, |s| s.end_ms)
    }

    /// チャンクのセグメントを取り除いて返す。
    fn remove_chunk(&mut self, chunk_index: u32) -> Vec<TranscriptSegment> {
        let old: Vec<_> = self
            .segments
            .iter()
            .filter(|s| s.chunk_index == chunk_index)
            .cloned()
            .collect();
        if !old.is_empty() {
            self.segments.retain(|s| s.chunk_index != chunk_index);
        }
        old
    }

    /// 検証済み・整列済みのセグメントを、時間順の位置に差し込む。
    /// 前後の既存セグメントと重なる場合は拒否する（状態は変えない）。
    fn insert_block(&mut self, block: Vec<TranscriptSegment>) -> Result<(), IngestError> {
        let (Some(first), Some(last)) = (block.first(), block.last()) else {
            return Ok(());
        };
        let at = self
            .segments
            .partition_point(|s| s.start_ms < first.start_ms);
        let prev_ok = at
            .checked_sub(1)
            .is_none_or(|i| self.segments[i].end_ms <= first.start_ms);
        let next_ok = self
            .segments
            .get(at)
            .is_none_or(|n| n.start_ms >= last.end_ms);
        if !(prev_ok && next_ok) {
            return Err(IngestError::InvalidFormat(
                "既存のセグメントと時間が重なっています".into(),
            ));
        }
        self.segments.splice(at..at, block);
        Ok(())
    }

    /// 取り除いたセグメントを元に戻す（登録が拒否されたとき）。
    fn restore(&mut self, old: Vec<TranscriptSegment>) {
        self.segments.extend(old);
        self.segments.sort_by_key(|s| s.start_ms);
    }

    /// チャンクの結果を登録する。同じチャンクの既存結果は置き換える（再実行しても重複しない）。
    /// 拒否した場合、状態は変わらない。
    pub fn set_chunk(
        &mut self,
        chunk_index: u32,
        segments: Vec<TranscriptSegment>,
    ) -> Result<(), IngestError> {
        let block = prepare(chunk_index, segments)?;
        let old = self.remove_chunk(chunk_index);
        self.insert_block(block).inspect_err(|_| self.restore(old))
    }

    /// 高精度の結果でチャンクを置き換え、差分を返す。拒否した場合、状態は変わらない。
    /// 新しいセグメントは、時間が最も重なる旧セグメントの話者名を引き継ぐ。
    pub fn apply_refined(
        &mut self,
        chunk_index: u32,
        refined: Vec<TranscriptSegment>,
    ) -> Result<RefineDiff, IngestError> {
        let refined = prepare(chunk_index, refined)?;
        let old = self.remove_chunk(chunk_index);
        let (merged, diff) = merge_refined(&old, refined);
        match self.insert_block(merged) {
            Ok(()) => Ok(diff),
            Err(e) => {
                self.restore(old);
                Err(e)
            }
        }
    }

    /// 話者ラベルを付け替える（「話者A」→「田中」）。付け替えたセグメント数を返す。
    pub fn rename_speaker(&mut self, from: &str, to: &str) -> usize {
        // 保存済みの名前は整形済みなので、`from` も同じ整形をかけて比較する。
        let (Some(from), to) = (normalize_speaker(from), normalize_speaker(to)) else {
            return 0;
        };
        let mut n = 0;
        for s in self
            .segments
            .iter_mut()
            .filter(|s| s.speaker.as_deref() == Some(from.as_str()))
        {
            s.speaker = to.clone();
            n += 1;
        }
        n
    }

    /// `index` 番目のセグメントの話者を設定する。範囲外なら false。
    pub fn set_speaker(&mut self, index: usize, speaker: Option<String>) -> bool {
        match self.segments.get_mut(index) {
            Some(s) => {
                s.speaker = speaker.and_then(|name| normalize_speaker(&name));
                true
            }
            None => false,
        }
    }

    /// `ms` を含むセグメント（開始 ≤ ms < 終了）。
    pub fn segment_at(&self, ms: u64) -> Option<&TranscriptSegment> {
        let next = self.segments.partition_point(|s| s.start_ms <= ms);
        next.checked_sub(1)
            .map(|i| &self.segments[i])
            .filter(|s| ms < s.end_ms)
    }

    /// 人が読む形式: `[00:01:23] 話者A: 本文`（話者が無ければ `[00:01:23] 本文`）。
    /// 本文・話者名の改行は空白にし、1 セグメント = 1 行を保つ。
    pub fn to_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                let ts = timestamp_label(s.start_ms);
                let text = one_line(&s.text);
                match &s.speaker {
                    Some(sp) => format!("[{ts}] {}: {text}\n", one_line(sp)),
                    None => format!("[{ts}] {text}\n"),
                }
            })
            .collect()
    }

    /// 検索索引用の `(参照 id, 本文)`。セグメントごとに 1 文書とし、参照 id から時刻を復元できる
    /// （引用をタップして該当音声を再生する）。セグメントは重ならないので、参照 id は一意。
    pub fn index_documents(
        &self,
        recording_id: &str,
    ) -> Result<Vec<(String, String)>, IngestError> {
        validate_recording_id(recording_id)?;
        Ok(self
            .segments
            .iter()
            .map(|s| (segment_ref(recording_id, s.start_ms), s.text.clone()))
            .collect())
    }
}

/// 新旧のセグメントを対応づけて、差分を数える。新しいセグメントは、時間が最も重なる旧セグメントと
/// 対応する。すでに別の新セグメントに使われた旧セグメントに当たった場合は `added`（分割された発話）。
fn merge_refined(
    old: &[TranscriptSegment],
    refined: Vec<TranscriptSegment>,
) -> (Vec<TranscriptSegment>, RefineDiff) {
    let mut matched = vec![false; old.len()];
    let mut diff = RefineDiff::default();
    let mut merged = Vec::with_capacity(refined.len());

    for mut seg in refined {
        let best = old
            .iter()
            .enumerate()
            .map(|(i, o)| (i, overlap_ms(&seg, o)))
            .filter(|&(_, ov)| ov > 0)
            .max_by_key(|&(_, ov)| ov);
        match best {
            Some((i, _)) => {
                if matched[i] {
                    diff.added += 1;
                } else if old[i].text == seg.text {
                    matched[i] = true;
                    diff.unchanged += 1;
                } else {
                    matched[i] = true;
                    diff.changed += 1;
                }
                if seg.speaker.is_none() {
                    seg.speaker = old[i].speaker.clone();
                }
            }
            None => diff.added += 1,
        }
        merged.push(seg);
    }
    diff.removed = matched.iter().filter(|m| !**m).count();
    (merged, diff)
}

/// 検索結果の参照 id（`録音id@開始ms`）。
pub fn segment_ref(recording_id: &str, start_ms: u64) -> String {
    format!("{recording_id}@{start_ms}")
}

/// 参照 id を `(録音 id, 開始ms)` に戻す。形式が不正、または `segment_ref` が作らない形
/// （`+5`、`007`）なら None。
pub fn parse_segment_ref(reference: &str) -> Option<(String, u64)> {
    let (id, ms_text) = reference.rsplit_once('@')?;
    validate_recording_id(id).ok()?;
    let ms: u64 = ms_text.parse().ok()?;
    (ms.to_string() == ms_text).then(|| (id.to_string(), ms))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_checks;
