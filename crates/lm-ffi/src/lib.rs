//! LocalMind コアの FFI 層（UniFFI、proc マクロ方式）。Kotlin から呼ぶ入口。
//!
//! 公開面は最小限: 経路決定（`decide_route`）と検索（`SearchEngine`）。
//! 埋め込みモデルは `ForeignEmbedder` として Kotlin 側から注入する。

#[cfg(feature = "onnx")]
mod e5;
mod error;
mod router;
mod search;

pub use error::FfiError;
pub use router::{decide_route, DeviceProfile, Route, Task};
pub use search::{
    e5_search_config, ForeignEmbedder, SearchConfig, SearchEngine, SearchHit, SearchResult,
};

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests;
