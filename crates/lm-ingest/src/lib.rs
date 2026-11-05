//! 録音の取り込み: チャンク分割・永続化・VAD・文字起こしパイプライン・議事録の土台。
//!
//! ネイティブの文字起こしエンジン（whisper.cpp）は [`transcribe::Transcriber`] の実装として
//! 呼び出し側が差し込む。このクレートは純 Rust で、音声の入出力そのもの（マイク、再生）は扱わない。
//!
//! 録音中は録音と VAD だけを行い、文字起こしなどの重い処理は停止後か充電中に
//! `lm-scheduler` の再処理ジョブとして順番に実行する（同時に重いモデルは 1 つだけ）。
//!
//! 処理の流れ: [`ChunkWriter`] で 30 秒ごとに分割 → [`FsChunkStore`] へ逐次保存（クラッシュ復元可）
//! → 停止後・充電中に [`transcribe::transcribe_chunk`]（VAD で発話区間だけを処理）→
//! [`Transcript`] に集約（仮→高精度の差分更新、話者の手動命名）→ 検索索引へ
//! （[`Transcript::index_documents`] の参照 id から時刻へ戻せる）。

pub mod audio;
pub mod chunk;
pub mod error;
pub mod minutes;
pub mod store;
pub mod transcribe;
pub mod transcript;
pub mod vad;

pub use chunk::{Chunk, ChunkWriter};
pub use error::IngestError;
pub use minutes::{ActionItem, Minutes, MinutesExtractor, MinutesItem, RawItem, RawMinutes};

pub use store::{is_expired, FsChunkStore, PurgeReport, RecordingMeta, Recovery};
pub use transcribe::{Stage, Transcriber, TranscriptSegment, Utterance};
pub use transcript::{RefineDiff, Transcript};
pub use vad::{EnergyVad, SpeechSpan, VadConfig};
