//! Launch-time cleanup of log files an older build left oversized or stale.
//!
//! Before the runtime size caps, rotated logs could grow without limit (one
//! machine held a 350 MB `app.dev.log.1`), and a release install can still hold
//! `*.dev.*` files from a dev build. This runs once per launch on a background
//! thread, touches a fixed set of names, and reads at most one cap's worth of
//! bytes per file. Current files are never touched: those rotate themselves.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A rotated file may exceed its cap by a batch or a line; only more than this
/// is legacy.
const SLACK_BYTES: u64 = 1024 * 1024;
/// A `*.dev.*` file in a release install is stale once untouched this long. A
/// dev build running beside it keeps its own files fresh.
const STALE_DEV_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// Most files a launch deletes, so a directory full of dated rolling files
/// cannot make this run long.
const MAX_REMOVALS: usize = 16;

/// Log families: file name, rotated generations, byte cap.
const FAMILIES: &[(&str, u8, u64)] = &[
    ("app.log", 1, super::APP_LOG_MAX_BYTES),
    ("app.dev.log", 1, super::APP_LOG_MAX_BYTES),
    ("events.jsonl", 2, super::EVENT_LOG_MAX_BYTES),
    ("events.dev.jsonl", 2, super::EVENT_LOG_MAX_BYTES),
];
const DEV_PREFIXES: &[&str] = &[
    "app.dev.log",
    "events.dev.jsonl",
    "frontend.dev.log",
    "transcriptions.dev.jsonl",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TrimReport {
    pub(super) trimmed_files: u32,
    pub(super) removed_files: u32,
    pub(super) reclaimed_bytes: u64,
}

/// Trim oversized rotated generations to their last `cap` bytes (from a line
/// boundary), and, when `release` is set, delete `*.dev.*` leftovers untouched
/// for [`STALE_DEV_AFTER`].
pub(super) fn trim_legacy_logs(dir: &Path, release: bool, now: SystemTime) -> TrimReport {
    let mut report = TrimReport::default();
    for (name, generations, cap) in FAMILIES {
        if release && name.contains(".dev.") {
            continue; // Handled by the stale-leftover pass below.
        }
        for path in super::rotated_paths(&dir.join(name), *generations) {
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.is_file() && metadata.len() > cap + SLACK_BYTES {
                if let Ok(kept) = trim_to_tail(&path, *cap) {
                    report.trimmed_files += 1;
                    report.reclaimed_bytes += metadata.len().saturating_sub(kept);
                }
            }
        }
    }
    if release {
        remove_stale_dev_files(dir, now, &mut report);
    }
    report
}

fn remove_stale_dev_files(dir: &Path, now: SystemTime, report: &mut TrimReport) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut stale: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            DEV_PREFIXES.iter().any(|prefix| {
                name.strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            })
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let age = now.duration_since(metadata.modified().ok()?).ok()?;
            (metadata.is_file() && age >= STALE_DEV_AFTER).then(|| (entry.path(), metadata.len()))
        })
        .collect();
    stale.sort();
    for (path, len) in stale.into_iter().take(MAX_REMOVALS) {
        if std::fs::remove_file(&path).is_ok() {
            report.removed_files += 1;
            report.reclaimed_bytes += len;
        }
    }
}

/// Replace `path` with its last `keep` bytes, starting at a line boundary.
/// Written beside the file and renamed over it, so a crash leaves either the
/// old file or the trimmed one. Returns the new size.
fn trim_to_tail(path: &Path, keep: u64) -> std::io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(keep)))?;
    let mut tail = Vec::with_capacity(keep as usize);
    file.take(keep).read_to_end(&mut tail)?;
    let start = tail
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(tail.len(), |newline| newline + 1);
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".trim");
    let temp = PathBuf::from(temp);
    let mut out = std::fs::File::create(&temp)?;
    let written = out.write_all(&tail[start..]).and_then(|()| out.flush());
    if let Err(error) = written.and_then(|()| std::fs::rename(&temp, path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok((tail.len() - start) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur-legacy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_lines(path: &Path, bytes: u64) {
        let line = "0123456789abcdef0123456789abcdef0123456789abcdef\n";
        let mut data = String::new();
        while (data.len() as u64) < bytes {
            data.push_str(line);
        }
        std::fs::write(path, data).unwrap();
    }

    fn age(path: &Path, by: Duration) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    #[test]
    fn an_oversized_rotated_file_is_trimmed_to_its_cap_on_a_line_boundary() {
        let dir = scratch("trim");
        let rotated = dir.join("app.log.1");
        write_lines(&rotated, super::super::APP_LOG_MAX_BYTES * 3);
        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        let trimmed = std::fs::read(&rotated).unwrap();
        assert_eq!(report.trimmed_files, 1);
        assert!(trimmed.len() as u64 <= super::super::APP_LOG_MAX_BYTES);
        assert!(trimmed.len() as u64 > super::super::APP_LOG_MAX_BYTES - 100);
        assert!(trimmed.starts_with(b"0123456789abcdef"), "starts mid-line");
        assert!(trimmed.ends_with(b"\n"));
        assert!(!dir.join("app.log.1.trim").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_within_the_cap_and_current_files_are_never_touched() {
        let dir = scratch("untouched");
        // A current file far over any cap is the runtime writer's to rotate.
        write_lines(&dir.join("app.log"), super::super::APP_LOG_MAX_BYTES * 2);
        write_lines(
            &dir.join("events.jsonl"),
            super::super::EVENT_LOG_MAX_BYTES * 2,
        );
        // A rotated file just over the cap, within the slack, is normal.
        write_lines(
            &dir.join("app.log.1"),
            super::super::APP_LOG_MAX_BYTES + 1024,
        );
        let before: Vec<u64> = ["app.log", "events.jsonl", "app.log.1"]
            .iter()
            .map(|name| std::fs::metadata(dir.join(name)).unwrap().len())
            .collect();
        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        let after: Vec<u64> = ["app.log", "events.jsonl", "app.log.1"]
            .iter()
            .map(|name| std::fs::metadata(dir.join(name)).unwrap().len())
            .collect();
        assert_eq!(report, TrimReport::default());
        assert_eq!(before, after);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_install_removes_only_stale_dev_leftovers() {
        let dir = scratch("stale");
        let stale = [
            "app.dev.log",
            "app.dev.log.1",
            "events.dev.jsonl.2",
            "app.dev.log.2026-03-02",
        ];
        for name in stale {
            write_lines(&dir.join(name), 2048);
            age(&dir.join(name), Duration::from_secs(10 * 24 * 60 * 60));
        }
        // A dev build running beside this install keeps its files fresh.
        write_lines(&dir.join("events.dev.jsonl"), 2048);
        write_lines(&dir.join("app.log"), 2048);
        // Unrelated names that merely share a prefix stay.
        write_lines(&dir.join("app.dev.logger.txt"), 2048);
        age(
            &dir.join("app.dev.logger.txt"),
            Duration::from_secs(10 * 24 * 60 * 60),
        );

        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        assert_eq!(report.removed_files, 4);
        for name in stale {
            assert!(!dir.join(name).exists(), "{name}");
        }
        for name in ["events.dev.jsonl", "app.log", "app.dev.logger.txt"] {
            assert!(dir.join(name).exists(), "{name}");
        }

        // A dev build never removes its own files.
        write_lines(&dir.join("app.dev.log.1"), 2048);
        age(
            &dir.join("app.dev.log.1"),
            Duration::from_secs(10 * 24 * 60 * 60),
        );
        let report = trim_legacy_logs(&dir, false, SystemTime::now());
        assert_eq!(report.removed_files, 0);
        assert!(dir.join("app.dev.log.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removals_per_launch_are_bounded() {
        let dir = scratch("bounded");
        for n in 0..(MAX_REMOVALS + 10) {
            let path = dir.join(format!("app.dev.log.{n:03}"));
            write_lines(&path, 100);
            age(&path, Duration::from_secs(10 * 24 * 60 * 60));
        }
        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        assert_eq!(report.removed_files as usize, MAX_REMOVALS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
