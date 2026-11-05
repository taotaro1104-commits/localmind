//! 端末状態（充電・温度）による実行ゲート。

/// 熱状態（Android の Thermal status に対応する簡略版）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Thermal {
    Normal,
    Warm,
    Hot,
}

/// 端末状態の読み取り。Kotlin 側から注入される想定。
pub trait DeviceState {
    fn is_charging(&self) -> bool;
    fn thermal(&self) -> Thermal;
}

/// テスト・単純用途向けの固定状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedDeviceState {
    pub charging: bool,
    pub thermal: Thermal,
}

impl DeviceState for FixedDeviceState {
    fn is_charging(&self) -> bool {
        self.charging
    }
    fn thermal(&self) -> Thermal {
        self.thermal
    }
}

/// 条件付きジョブ（再処理）を実行してよいか: 充電中かつ Normal。
pub fn favorable(state: &dyn DeviceState) -> bool {
    state.is_charging() && state.thermal() == Thermal::Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(charging: bool, thermal: Thermal) -> FixedDeviceState {
        FixedDeviceState { charging, thermal }
    }

    #[test]
    fn favorable_only_when_charging_and_cool() {
        assert!(favorable(&st(true, Thermal::Normal)));
        assert!(!favorable(&st(false, Thermal::Normal)));
        assert!(!favorable(&st(true, Thermal::Warm)));
        assert!(!favorable(&st(true, Thermal::Hot)));
    }
}
