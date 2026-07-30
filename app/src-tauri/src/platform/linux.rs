use super::{update_cpu_percent, CpuTicks};
use crate::MutexExt;
use std::sync::Mutex;

fn parse_snapshot(contents: &str) -> Option<CpuTicks> {
    let line = contents.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }

    let user: u64 = parts.next()?.parse().ok()?;
    let nice: u64 = parts.next()?.parse().ok()?;
    let system: u64 = parts.next()?.parse().ok()?;
    let idle: u64 = parts.next()?.parse().ok()?;
    let iowait: u64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let irq: u64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let softirq: u64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let steal: u64 = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    Some(CpuTicks::new(
        user + nice + system + irq + softirq + steal,
        idle + iowait,
    ))
}

fn snapshot() -> Option<CpuTicks> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    parse_snapshot(&contents)
}

static PREVIOUS: Mutex<Option<CpuTicks>> = Mutex::new(None);

pub(super) fn cpu_percent() -> Option<f32> {
    update_cpu_percent(&mut PREVIOUS.lock_or_recover(), snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_cpu_totals() {
        let snapshot = parse_snapshot("cpu  10 2 3 20 4 5 6 7 0 0\n").unwrap();

        assert_eq!(snapshot, CpuTicks::new(33, 24));
    }

    #[test]
    fn rejects_non_cpu_input() {
        assert_eq!(parse_snapshot("intr 1 2 3\n"), None);
    }
}
