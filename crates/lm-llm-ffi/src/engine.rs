//! 議事録の抽出エンジン（Kotlin に公開する）。

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path};
use std::sync::Arc;

use lm_ingest::minutes::{extract_minutes, render_for_llm};
use lm_ingest::{
    FsChunkStore, IngestError, Minutes, MinutesItem, Stage, Transcript, TranscriptSegment,
};
use lm_llm::llama::{LlamaConfig, LlamaGenerator, OpenError};
use lm_llm::{Language, LlmMinutesExtractor, CANCELLED_MESSAGE, MAX_INPUT_CHARS, TOO_LONG_MESSAGE};

use crate::error::LlmError;

/// 渡せる発話の数と、本文の合計バイト数の上限（巨大な入力でメモリを使い切らない）。
const MAX_LINES: usize = 5_000;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_THREADS: u32 = 8;

/// 文字起こしの 1 区間（`lm-ffi` の `SpokenSegment` に対応）。時刻は録音の開始からのミリ秒。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SpokenLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// 議事録の 1 項目。`citations` は根拠の発話の開始位置（ミリ秒）で、必ず 1 つ以上ある。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MinutesEntry {
    pub text: String,
    /// 担当者（やることだけ）。
    pub assignee: Option<String>,
    /// 期限（やることだけ。自由記述）。
    pub due: Option<String>,
    pub citations: Vec<u64>,
}

/// 抽出した議事録。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MinutesReport {
    pub summary: Vec<MinutesEntry>,
    pub decisions: Vec<MinutesEntry>,
    pub action_items: Vec<MinutesEntry>,
    pub open_issues: Vec<MinutesEntry>,
    /// 根拠が無い・不正などの理由で、採用しなかった項目の数。
    pub rejected: u32,
    /// 使ったモデルの識別子。
    pub model_id: String,
}

fn entry(item: &MinutesItem) -> MinutesEntry {
    MinutesEntry {
        text: item.text.clone(),
        assignee: None,
        due: None,
        citations: item.citations.clone(),
    }
}

fn report(m: &Minutes, model_id: String) -> MinutesReport {
    MinutesReport {
        summary: m.summary.iter().map(entry).collect(),
        decisions: m.decisions.iter().map(entry).collect(),
        action_items: m
            .action_items
            .iter()
            .map(|a| MinutesEntry {
                text: a.text.clone(),
                assignee: a.assignee.clone(),
                due: a.due.clone(),
                citations: a.citations.clone(),
            })
            .collect(),
        open_issues: m.open_issues.iter().map(entry).collect(),
        rejected: u32::try_from(m.rejected).unwrap_or(u32::MAX),
        model_id,
    }
}

fn invalid(msg: &str) -> LlmError {
    LlmError::InvalidInput { msg: msg.into() }
}

/// アプリ専用領域の絶対パス（`..` なし）だけを受け付ける。
fn check_abs_path(path: &str, what: &str) -> Result<(), LlmError> {
    let p = Path::new(path);
    if !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(invalid(&format!(
            "{what}は `..` を含まない絶対パスで指定してください"
        )));
    }
    Ok(())
}

/// 発話の一覧から文字起こしを作る（時刻の順で、重ならないこと）。
fn transcript_from(lines: &[SpokenLine]) -> Result<Transcript, LlmError> {
    if lines.len() > MAX_LINES {
        return Err(invalid("発話の数が多すぎます"));
    }
    if lines.iter().map(|l| l.text.len()).sum::<usize>() > MAX_TEXT_BYTES {
        return Err(invalid("文字起こしが大きすぎます"));
    }
    let segments = lines
        .iter()
        .map(|l| TranscriptSegment {
            chunk_index: 0,
            start_ms: l.start_ms,
            end_ms: l.end_ms,
            speaker: None,
            text: l.text.clone(),
            confidence: 1.0,
            stage: Stage::Draft,
        })
        .collect();
    let mut transcript = Transcript::new();
    transcript
        .set_chunk(0, segments)
        .map_err(|_| invalid("発話の時刻が不正です（時刻の順で、重ならないこと）"))?;
    Ok(transcript)
}

#[derive(uniffi::Object)]
pub struct MinutesEngine {
    extractor: LlmMinutesExtractor,
}

impl MinutesEngine {
    pub(crate) fn extractor(&self) -> &LlmMinutesExtractor {
        &self.extractor
    }

    pub(crate) fn model_id(&self) -> String {
        self.extractor_id()
    }

    pub(crate) fn from_extractor(extractor: LlmMinutesExtractor) -> Self {
        Self { extractor }
    }

    fn run(&self, lines: &[SpokenLine]) -> Result<Minutes, LlmError> {
        let transcript = transcript_from(lines)?;
        if render_for_llm(&transcript).chars().count() > MAX_INPUT_CHARS {
            return Err(invalid(
                "文字起こしが長すぎます。短い区間に分けて抽出してください",
            ));
        }
        // 開始前に届いた中止は有効（エンジンが、終了時に中止の要求を戻す）。
        // ネイティブの推論は、パニックでも FFI の外へ出さない。
        let result = catch_unwind(AssertUnwindSafe(|| {
            extract_minutes(&transcript, &self.extractor, MAX_INPUT_CHARS)
        }));
        let result = result.map_err(|_| LlmError::Engine {
            msg: "議事録の抽出の途中で失敗しました".into(),
        })?;
        // 種類は、エンジンが返した理由で決める（失敗のあとに中止が届いても、本当の失敗を隠さない）。
        result.map_err(|e| match e {
            IngestError::Extract(m) if m == CANCELLED_MESSAGE => LlmError::Cancelled,
            IngestError::Extract(m) if m == TOO_LONG_MESSAGE => {
                invalid("文字起こしが長すぎます。短い区間に分けて抽出してください")
            }
            IngestError::Extract(msg) => LlmError::Engine { msg },
            other => LlmError::Engine {
                msg: other.to_string(),
            },
        })
    }
}

#[uniffi::export]
impl MinutesEngine {
    /// モデル（GGUF）を検証して読み込む。`sha256` は小文字の 16 進 64 文字で、内容が一致しないモデルは
    /// 読み込まない（`Integrity`。入れ直せば直る）。`threads` は 1〜8。時間とメモリがかかるので、UI スレッドから
    /// 呼ばないこと。同時に 2 つ開かないこと。
    #[uniffi::constructor]
    pub fn open(model_path: String, sha256: String, threads: u32) -> Result<Arc<Self>, LlmError> {
        check_abs_path(&model_path, "モデルのパス")?;
        if !(1..=MAX_THREADS).contains(&threads) {
            return Err(invalid("スレッド数は 1〜8 でなければなりません"));
        }
        let config = LlamaConfig {
            threads,
            ..LlamaConfig::default()
        };
        let opened = catch_unwind(|| LlamaGenerator::open(Path::new(&model_path), &sha256, config))
            .map_err(|_| LlmError::Engine {
                msg: "LLM エンジンの初期化に失敗しました".into(),
            })?;
        let generator = opened.map_err(|e| match e {
            OpenError::Integrity(msg) => LlmError::Integrity { msg },
            OpenError::InvalidConfig(msg) => LlmError::InvalidInput { msg },
            OpenError::Engine(msg) => LlmError::Engine { msg },
        })?;
        Ok(Arc::new(Self::from_extractor(LlmMinutesExtractor::new(
            Box::new(generator),
        ))))
    }

    /// 議事録を書く言語を決める（`ja` か `en`。既定は `ja`）。次の抽出から効く。抽出は 1 つずつ行うこと。
    pub fn set_language(&self, language: String) -> Result<(), LlmError> {
        let lang = Language::from_code(&language)
            .ok_or_else(|| invalid("言語は ja か en で指定してください"))?;
        self.extractor.set_language(lang);
        Ok(())
    }

    /// 実行中の抽出を止める（別のスレッドから呼べる）。止めた抽出は `Cancelled` を返す。
    pub fn cancel(&self) {
        self.extractor.cancel();
    }

    /// 残っている中止の要求を取り消す。生成が走っていないときに `cancel` した場合に、次の質問の前に呼ぶ。
    pub fn reset_cancel(&self) {
        self.extractor.reset_cancel();
    }

    /// 渡せる文字起こしの最大文字数の目安（時刻の印を含めた、LLM への入力の文字数）。
    pub fn max_input_chars(&self) -> u32 {
        MAX_INPUT_CHARS as u32
    }

    /// 発話の一覧から議事録を抽出する（保存はしない）。発話は時刻の順で、重ならないこと。空なら空の議事録。
    pub fn extract(&self, lines: Vec<SpokenLine>) -> Result<MinutesReport, LlmError> {
        let minutes = self.run(&lines)?;
        Ok(report(&minutes, self.extractor_id()))
    }

    /// 抽出して、録音 `recording_id` の議事録として保存する（`recordings_root` は録音の置き場所の絶対パス）。
    /// 録音が無ければエラー（空のディレクトリを作らない）。保存した議事録は、`lm-ffi` の
    /// `RecordingStore.export_minutes` で書き出せる。
    pub fn extract_and_save(
        &self,
        recordings_root: String,
        recording_id: String,
        lines: Vec<SpokenLine>,
    ) -> Result<MinutesReport, LlmError> {
        check_abs_path(&recordings_root, "録音の置き場所")?;
        let store = FsChunkStore::new(&recordings_root);
        store
            .load_meta(&recording_id)
            .map_err(|_| invalid("その録音がありません"))?;
        let minutes = self.run(&lines)?;
        store
            .save_minutes(&recording_id, &minutes)
            .map_err(|e| LlmError::Engine { msg: e.to_string() })?;
        Ok(report(&minutes, self.extractor_id()))
    }
}

impl MinutesEngine {
    fn extractor_id(&self) -> String {
        lm_ingest::MinutesExtractor::id(&self.extractor)
    }
}
