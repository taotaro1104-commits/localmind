//! LocalMind コアの FFI 層（UniFFI、proc マクロ方式）。Kotlin から呼ぶ入口。
//!
//! 公開面は最小限: 経路決定（`decide_route`）、検索（`SearchEngine`）、文書の取り込みと出典つきの検索（`WorkspaceEngine`）。
//! 埋め込みモデルは `ForeignEmbedder` として Kotlin 側から注入する。

#[cfg(feature = "onnx")]
mod e5;
mod error;
mod recording;
mod router;
mod search;
mod workspace;

pub use error::FfiError;
pub use recording::{
    ExportFormat, ExportedFile, PurgeSummary, Recorder, RecordingInfo, RecordingStore,
};
pub use router::{decide_route, DeviceProfile, Route, Task};
pub use search::{
    e5_search_config, ForeignEmbedder, SearchConfig, SearchEngine, SearchHit, SearchResult,
};
pub use workspace::{Citation, DocFormat, DocumentInfo, WorkspaceEngine, WorkspaceReport};

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_recording;
#[cfg(test)]
mod tests_workspace;
