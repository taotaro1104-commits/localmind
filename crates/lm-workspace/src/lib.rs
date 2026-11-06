//! ワークスペース: 文書（テキスト・Markdown）の取り込み、見出し・段落での分割、検索、位置つきの引用。
//!
//! 文書の原本を保存し、検索用のチャンクは原本から決まった手順で作る（`chunker`）。
//! そのため、チャンクの位置（見出し・行番号）は原本を分割し直せば復元でき、別に保存しない。

pub mod chunker;
pub mod docstore;
pub mod error;
pub mod html;
pub mod workspace;

pub use chunker::{split, Chunk, Format, CHUNKER_VERSION};
pub use docstore::{DocStore, StoredDoc};
pub use error::WorkspaceError;
pub use workspace::{Citation, DocInfo, Report, Workspace, E5_RELEVANCE_MARGIN};
