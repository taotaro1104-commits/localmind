//! 用途 → 実行経路の決定。

use crate::profile::DeviceProfile;

/// 処理の用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Task {
    /// 短い要約・言い換え
    ShortText,
    /// 文書 RAG（長い文脈）
    DocumentRag,
    /// 文字起こし
    Transcription,
    /// PC の大型モデルへのオフロードを明示指定した質問
    Offload,
}

/// 実行経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    Nano,
    Gguf,
    Pc,
    Whisper,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouteError {
    #[error("実行可能な経路がありません（GGUF モデルが未導入）")]
    NoRouteAvailable,
}

/// 経路を決める。常に GGUF へ縮退でき、GGUF も無い場合のみエラー。
///
/// - 短い要約・言い換え: Nano → GGUF
/// - 文書 RAG: GGUF（Nano は文脈が足りない）
/// - 文字起こし: whisper.cpp（モデルは別管理のため常に可）
/// - オフロード: PC → GGUF
pub fn decide(task: Task, profile: &DeviceProfile) -> Result<Route, RouteError> {
    match task {
        Task::Transcription => Ok(Route::Whisper),
        Task::ShortText if profile.nano_available => Ok(Route::Nano),
        Task::Offload if profile.pc_reachable => Ok(Route::Pc),
        Task::ShortText | Task::DocumentRag | Task::Offload => {
            if profile.gguf_installed {
                Ok(Route::Gguf)
            } else {
                Err(RouteError::NoRouteAvailable)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(nano: bool, pc: bool, gguf: bool) -> DeviceProfile {
        DeviceProfile {
            ram_mb: 8192,
            nano_available: nano,
            pc_reachable: pc,
            gguf_installed: gguf,
        }
    }

    #[test]
    fn short_text_prefers_nano() {
        assert_eq!(
            decide(Task::ShortText, &profile(true, false, true)),
            Ok(Route::Nano)
        );
    }

    #[test]
    fn short_text_falls_back_to_gguf_without_nano() {
        assert_eq!(
            decide(Task::ShortText, &profile(false, false, true)),
            Ok(Route::Gguf)
        );
    }

    #[test]
    fn document_rag_always_uses_gguf_even_with_nano() {
        assert_eq!(
            decide(Task::DocumentRag, &profile(true, true, true)),
            Ok(Route::Gguf)
        );
    }

    #[test]
    fn transcription_uses_whisper() {
        assert_eq!(
            decide(Task::Transcription, &profile(false, false, false)),
            Ok(Route::Whisper)
        );
    }

    #[test]
    fn offload_uses_pc_when_reachable_else_gguf() {
        assert_eq!(
            decide(Task::Offload, &profile(false, true, true)),
            Ok(Route::Pc)
        );
        assert_eq!(
            decide(Task::Offload, &profile(false, false, true)),
            Ok(Route::Gguf)
        );
    }

    #[test]
    fn no_nano_means_every_text_task_degrades_to_gguf() {
        let p = profile(false, false, true);
        for t in [Task::ShortText, Task::DocumentRag, Task::Offload] {
            assert_eq!(decide(t, &p), Ok(Route::Gguf), "{t:?}");
        }
    }

    #[test]
    fn errors_when_no_gguf_and_no_other_route() {
        assert_eq!(
            decide(Task::DocumentRag, &profile(true, true, false)),
            Err(RouteError::NoRouteAvailable)
        );
        assert_eq!(
            decide(Task::ShortText, &profile(false, false, false)),
            Err(RouteError::NoRouteAvailable)
        );
    }
}
