//! 用途ごとの実行経路（Nano / GGUF / PC / whisper）の決定と縮退。

pub mod profile;
pub mod route;

pub use profile::DeviceProfile;
pub use route::{decide, Route, RouteError, Task};

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_with_ram(ram_mb: u32) -> DeviceProfile {
        DeviceProfile {
            ram_mb,
            nano_available: false,
            pc_reachable: false,
            gguf_installed: true,
        }
    }

    #[test]
    fn low_ram_boundary_is_exactly_six_gib() {
        assert_eq!(DeviceProfile::LOW_RAM_MB, 6144);
        assert!(profile_with_ram(DeviceProfile::LOW_RAM_MB - 1).is_low_ram());
        assert!(!profile_with_ram(DeviceProfile::LOW_RAM_MB).is_low_ram());
    }

    #[test]
    fn low_ram_does_not_change_the_route() {
        assert_eq!(
            decide(Task::DocumentRag, &profile_with_ram(2048)),
            decide(Task::DocumentRag, &profile_with_ram(16384))
        );
    }

    #[test]
    fn low_ram_threshold() {
        let mut p = DeviceProfile {
            ram_mb: 4096,
            nano_available: false,
            pc_reachable: false,
            gguf_installed: true,
        };
        assert!(p.is_low_ram());
        p.ram_mb = 8192;
        assert!(!p.is_low_ram());
    }
}
