//! Launch-time cleanup of log files an older build left oversized or stale.
//!
//! Before the runtime size caps, a rotated text log could grow without limit
//! (one machine held a 350 MB `app.dev.log.1`), and a release install can still
//! hold `*.dev.*` generations from a dev build. This runs once per launch,
//! synchronously, **before this process opens its log writers**, so it cannot
//! race this process's rotation or Clear logs. It is deliberately narrow:
//!
//! - It trims only this build's rotated text generation (`app.log.1` in release,
//!   `app.dev.log.1` in a dev build). It never touches a current file, and never
//!   touches `events*.jsonl`: the shipper's cursor identifies a JSONL file by
//!   inode and birth time, so replacing one would replay acknowledged events.
//! - It deletes only rotated or dated generations of `*.dev.*` files in a release
//!   install, never a current file, which a running dev app may hold open.
//! - A trim reads at most one cap of bytes, replaces the file only if it is
//!   still the file it read, and a launch deletes at most [`MAX_REMOVALS`] files.
//!
//! Another process rotating the same name in the instant between the read and
//! the replacement is caught by the identity check, which abandons the trim.

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A rotated file may exceed its cap by a line; only more than this is legacy.
const SLACK_BYTES: u64 = 1024 * 1024;
/// A rotated `*.dev.*` generation in a release install is stale once untouched
/// this long.
const STALE_DEV_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);
/// Most files a launch deletes, so a directory full of dated rolling files
/// cannot make this run long.
const MAX_REMOVALS: usize = 16;

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

/// What identifies one incarnation of a file between a read and its replacement.
#[derive(Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i64,
}

impl Identity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime_ns: metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec(),
        }
    }
}

/// Trim this build's oversized rotated text log to its last cap's worth of whole
/// lines and, in a release install, delete stale rotated `*.dev.*` generations.
pub(super) fn trim_legacy_logs(dir: &Path, release: bool, now: SystemTime) -> TrimReport {
    let mut report = TrimReport::default();
    let current = if release { "app.log" } else { "app.dev.log" };
    let rotated = super::generation_path(&dir.join(current), 1);
    let cap = super::APP_LOG_MAX_BYTES;
    if let Ok(metadata) = std::fs::metadata(&rotated) {
        if metadata.is_file() && metadata.len() > cap + SLACK_BYTES {
            if let Ok(Some(kept)) = trim_to_tail(&rotated, cap) {
                report.trimmed_files += 1;
                report.reclaimed_bytes += metadata.len().saturating_sub(kept);
            }
        }
    }
    if release {
        remove_stale_dev_generations(dir, now, &mut report);
    }
    report
}

/// Delete rotated and dated generations (`app.dev.log.1`, `events.dev.jsonl.2`,
/// `app.dev.log.2026-03-02`) untouched for [`STALE_DEV_AFTER`]. The current
/// files themselves are never candidates.
fn remove_stale_dev_generations(dir: &Path, now: SystemTime, report: &mut TrimReport) {
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
                    .is_some_and(|rest| rest.len() > 1 && rest.starts_with('.'))
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

/// The last `keep` bytes of `path` from a line boundary, with the identity of
/// the file they were read from.
fn read_tail(path: &Path, keep: u64) -> std::io::Result<(Vec<u8>, Identity)> {
    let mut file = std::fs::File::open(path)?;
    let identity = Identity::of(&file.metadata()?);
    file.seek(SeekFrom::Start(identity.len.saturating_sub(keep)))?;
    let mut tail = Vec::with_capacity(keep as usize);
    file.take(keep).read_to_end(&mut tail)?;
    let start = tail
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(tail.len(), |newline| newline + 1);
    tail.drain(..start);
    Ok((tail, identity))
}

/// Replace `path` with `data` unless it is no longer the file `expected` was
/// read from (rotated, cleared or rewritten meanwhile). Written beside the file
/// and renamed over it, so a crash leaves the old file or the trimmed one.
/// `Ok(false)` means the file changed and nothing was replaced.
fn replace_if_unchanged(path: &Path, expected: &Identity, data: &[u8]) -> std::io::Result<bool> {
    let mut temp = path.as_os_str().to_os_string();
    temp.push(".trim");
    let temp = PathBuf::from(temp);
    let mut out = std::fs::File::create(&temp)?;
    let written = out.write_all(data).and_then(|()| out.flush());
    let unchanged = matches!(
        std::fs::metadata(path),
        Ok(current) if Identity::of(&current) == *expected
    );
    let result = match written {
        Ok(()) if unchanged => std::fs::rename(&temp, path).map(|()| true),
        Ok(()) => Ok(false),
        Err(error) => Err(error),
    };
    if !matches!(result, Ok(true)) {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Trim `path` to its last `keep` bytes. Returns the new size, or `None` when
/// the file changed under the trim and was left alone.
fn trim_to_tail(path: &Path, keep: u64) -> std::io::Result<Option<u64>> {
    let (tail, identity) = read_tail(path, keep)?;
    Ok(replace_if_unchanged(path, &identity, &tail)?.then_some(tail.len() as u64))
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

    const OLD: Duration = Duration::from_secs(10 * 24 * 60 * 60);

    #[test]
    fn an_oversized_rotated_text_log_is_trimmed_to_its_cap_on_a_line_boundary() {
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
    fn current_files_jsonl_and_the_other_builds_files_are_never_touched() {
        let dir = scratch("untouched");
        let over = super::super::APP_LOG_MAX_BYTES * 2;
        // Current files rotate themselves; a rotated JSONL generation carries
        // the shipper's cursor; the other build's text log is not this build's.
        write_lines(&dir.join("app.log"), over);
        write_lines(&dir.join("events.jsonl"), over);
        write_lines(&dir.join("events.jsonl.1"), over);
        write_lines(&dir.join("events.jsonl.2"), over);
        write_lines(&dir.join("app.dev.log.1"), over);
        // Just over the cap but within the slack is normal.
        write_lines(
            &dir.join("app.log.1"),
            super::super::APP_LOG_MAX_BYTES + 1024,
        );
        let names = [
            "app.log",
            "events.jsonl",
            "events.jsonl.1",
            "events.jsonl.2",
            "app.dev.log.1",
            "app.log.1",
        ];
        let sizes = |dir: &Path| -> Vec<u64> {
            names
                .iter()
                .map(|name| std::fs::metadata(dir.join(name)).unwrap().len())
                .collect()
        };
        let before = sizes(&dir);
        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        // The release pass would delete a stale dev generation, but this one is
        // fresh; nothing else qualifies.
        assert_eq!(report, TrimReport::default());
        assert_eq!(before, sizes(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dev_build_trims_its_own_rotated_log_only() {
        let dir = scratch("dev");
        let over = super::super::APP_LOG_MAX_BYTES * 2;
        write_lines(&dir.join("app.dev.log.1"), over);
        write_lines(&dir.join("app.log.1"), over);
        let report = trim_legacy_logs(&dir, false, SystemTime::now());
        assert_eq!(report.trimmed_files, 1);
        assert!(
            std::fs::metadata(dir.join("app.dev.log.1")).unwrap().len()
                <= super::super::APP_LOG_MAX_BYTES
        );
        assert_eq!(
            std::fs::metadata(dir.join("app.log.1")).unwrap().len(),
            over.div_ceil(49) * 49
        );
        // A dev build never deletes anything.
        age(&dir.join("app.dev.log.1"), OLD);
        assert_eq!(
            trim_legacy_logs(&dir, false, SystemTime::now()).removed_files,
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_release_install_removes_only_stale_rotated_dev_generations() {
        let dir = scratch("stale");
        let stale = [
            "app.dev.log.1",
            "events.dev.jsonl.2",
            "app.dev.log.2026-03-02",
            "frontend.dev.log.1",
        ];
        for name in stale {
            write_lines(&dir.join(name), 2048);
            age(&dir.join(name), OLD);
        }
        // Current files, however old, may be held open by a running dev app.
        for name in [
            "app.dev.log",
            "events.dev.jsonl",
            "frontend.dev.log",
            "transcriptions.dev.jsonl",
        ] {
            write_lines(&dir.join(name), 2048);
            age(&dir.join(name), OLD);
        }
        // A fresh generation, a release file and an unrelated name stay.
        write_lines(&dir.join("events.dev.jsonl.1"), 2048);
        write_lines(&dir.join("app.log"), 2048);
        write_lines(&dir.join("app.dev.logger.txt"), 2048);
        age(&dir.join("app.dev.logger.txt"), OLD);

        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        assert_eq!(report.removed_files, 4);
        for name in stale {
            assert!(!dir.join(name).exists(), "{name}");
        }
        for name in [
            "app.dev.log",
            "events.dev.jsonl",
            "frontend.dev.log",
            "transcriptions.dev.jsonl",
            "events.dev.jsonl.1",
            "app.log",
            "app.dev.logger.txt",
        ] {
            assert!(dir.join(name).exists(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removals_per_launch_are_bounded() {
        let dir = scratch("bounded");
        for n in 0..(MAX_REMOVALS + 10) {
            let path = dir.join(format!("app.dev.log.{n:03}"));
            write_lines(&path, 100);
            age(&path, OLD);
        }
        let report = trim_legacy_logs(&dir, true, SystemTime::now());
        assert_eq!(report.removed_files as usize, MAX_REMOVALS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_rotated_or_cleared_during_the_trim_is_not_overwritten() {
        let dir = scratch("race");
        let path = dir.join("app.log.1");
        write_lines(&path, 4096);
        let (tail, identity) = read_tail(&path, 1024).unwrap();

        // Another process rotates newer data into the name after the read.
        let newer = dir.join("newer");
        std::fs::write(&newer, b"newer data\n").unwrap();
        std::fs::rename(&newer, &path).unwrap();
        assert!(!replace_if_unchanged(&path, &identity, &tail).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"newer data\n");
        assert!(!dir.join("app.log.1.trim").exists());

        // The same file appended to after the read is also not the file read.
        let (tail, identity) = read_tail(&path, 1024).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"more\n")
            .unwrap();
        assert!(!replace_if_unchanged(&path, &identity, &tail).unwrap());

        // A clear that deleted the file must not be undone by recreating it.
        let (tail, identity) = read_tail(&path, 1024).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(!replace_if_unchanged(&path, &identity, &tail).unwrap());
        assert!(!path.exists(), "deleted history was recreated");
        assert!(!dir.join("app.log.1.trim").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
