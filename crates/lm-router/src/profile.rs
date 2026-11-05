//! 端末プロファイル（診断結果）。

/// 初回診断・実行時に得られる端末の能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    pub ram_mb: u32,
    /// Gemini Nano（AICore）が利用可能か。ブートローダー解除端末などでは false。
    pub nano_available: bool,
    /// LAN 内に PC コンパニオンが到達可能か。
    pub pc_reachable: bool,
    /// ローカルの GGUF モデルが導入済みか。
    pub gguf_installed: bool,
}

impl DeviceProfile {
    /// 軽量設定（小さい GGUF、並列度の抑制）を促す RAM の下限。
    /// UI・モデル選定用の情報で、経路決定（`route::decide`）には影響しない。
    pub const LOW_RAM_MB: u32 = 6 * 1024;

    pub fn is_low_ram(&self) -> bool {
        self.ram_mb < Self::LOW_RAM_MB
    }
}
