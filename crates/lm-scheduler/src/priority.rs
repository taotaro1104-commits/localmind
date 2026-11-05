//! ジョブの優先度と種別。録音＞対話＞索引化＞再処理。

/// 優先度（数値が小さいほど先に実行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Recording = 0,
    Chat = 1,
    Indexing = 2,
    Reprocessing = 3,
}

impl Priority {
    /// 充電中・低温など「条件付き」でのみ実行する種別か。
    pub fn requires_favorable_conditions(self) -> bool {
        matches!(self, Priority::Reprocessing)
    }
}

/// スケジューラに投入するジョブ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub priority: Priority,
    /// 重いモデル（LLM/whisper/OCR）を載せるか。true は同時に1つだけ。
    pub heavy: bool,
}

impl Job {
    pub fn new(id: u64, name: impl Into<String>, priority: Priority, heavy: bool) -> Self {
        Self {
            id,
            name: name.into(),
            priority,
            heavy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_recording_chat_indexing_reprocessing() {
        assert!(Priority::Recording < Priority::Chat);
        assert!(Priority::Chat < Priority::Indexing);
        assert!(Priority::Indexing < Priority::Reprocessing);
    }

    #[test]
    fn only_reprocessing_needs_favorable_conditions() {
        assert!(Priority::Reprocessing.requires_favorable_conditions());
        assert!(!Priority::Indexing.requires_favorable_conditions());
        assert!(!Priority::Recording.requires_favorable_conditions());
    }
}
