//! Compile-time-selected platform behavior.
//!
//! Keep target selection in this module. Callers should depend on the stable
//! functions exported here instead of carrying their own `#[cfg]` branches.

#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux")),
    path = "unsupported.rs"
)]
mod current;

pub(crate) fn cpu_percent() -> f32 {
    current::cpu_percent()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CpuTicks {
    active: u64,
    idle: u64,
}

impl CpuTicks {
    pub(super) const fn new(active: u64, idle: u64) -> Self {
        Self { active, idle }
    }
}

pub(super) fn cpu_percent_between(previous: CpuTicks, current: CpuTicks) -> f32 {
    let active = current.active.wrapping_sub(previous.active);
    let idle = current.idle.wrapping_sub(previous.idle);
    let total = active + idle;
    if total == 0 {
        0.0
    } else {
        (active as f64 / total as f64 * 100.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_uses_delta_between_samples() {
        let previous = CpuTicks::new(1_000, 4_000);
        let current = CpuTicks::new(1_075, 4_025);

        assert_eq!(cpu_percent_between(previous, current), 75.0);
    }

    #[test]
    fn cpu_percent_is_zero_without_elapsed_ticks() {
        let snapshot = CpuTicks::new(1_000, 4_000);

        assert_eq!(cpu_percent_between(snapshot, snapshot), 0.0);
    }
}
