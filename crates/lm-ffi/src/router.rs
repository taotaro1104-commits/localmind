//! 経路決定（lm-router）の公開面。

use crate::error::FfiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Task {
    ShortText,
    DocumentRag,
    Transcription,
    Offload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Route {
    Nano,
    Gguf,
    Pc,
    Whisper,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DeviceProfile {
    pub ram_mb: u32,
    pub nano_available: bool,
    pub pc_reachable: bool,
    pub gguf_installed: bool,
}

/// 用途と端末の状態から実行経路を決める。GGUF も無ければエラー。
#[uniffi::export]
pub fn decide_route(task: Task, profile: DeviceProfile) -> Result<Route, FfiError> {
    let t = match task {
        Task::ShortText => lm_router::Task::ShortText,
        Task::DocumentRag => lm_router::Task::DocumentRag,
        Task::Transcription => lm_router::Task::Transcription,
        Task::Offload => lm_router::Task::Offload,
    };
    let p = lm_router::DeviceProfile {
        ram_mb: profile.ram_mb,
        nano_available: profile.nano_available,
        pc_reachable: profile.pc_reachable,
        gguf_installed: profile.gguf_installed,
    };
    Ok(match lm_router::decide(t, &p)? {
        lm_router::Route::Nano => Route::Nano,
        lm_router::Route::Gguf => Route::Gguf,
        lm_router::Route::Pc => Route::Pc,
        lm_router::Route::Whisper => Route::Whisper,
    })
}
