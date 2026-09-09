//! Shared low-level SQLite helpers for Murmur's local stores
//! (`knowledge_store`, `query_history`, `meeting_store`, `performance_metrics`).
//!
//! This module intentionally stays at the raw `rusqlite`/`std::io` error
//! level: every store maps these results into its own error type at the call
//! site, so each store keeps its own messages, recovery policy, and
//! `InitializationOutcome`/migration logic. Nothing here changes store
//! behavior — it only removes copy-pasted plumbing.

use rusqlite::Connection;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The pragma configuration applied to a store's primary connection.
///
/// Each store preserves its own historical values exactly; this struct only
/// removes the duplicated `pragma_update`/`busy_timeout` call sequence.
#[derive(Debug, Clone, Copy)]
pub struct PragmaOptions {
    pub journal_mode: &'static str,
    pub synchronous: &'static str,
    pub foreign_keys: bool,
    /// `None` means the store never sets `secure_delete` (leaves the SQLite
    /// default), matching `performance_metrics` today.
    pub secure_delete: Option<bool>,
    pub busy_timeout: Duration,
}

impl PragmaOptions {
    /// WAL journaling, FULL durability, foreign keys and secure delete on,
    /// a 2s busy timeout. Shared today by `knowledge_store`, `query_history`,
    /// and `meeting_store`'s primary connections.
    pub const STANDARD: Self = Self {
        journal_mode: "WAL",
        synchronous: "FULL",
        foreign_keys: true,
        secure_delete: Some(true),
        busy_timeout: Duration::from_secs(2),
    };
}

/// Apply `options` to `connection`. Order: busy timeout first (so any
/// subsequent pragma that needs to wait on a lock already has it), then
/// foreign keys, journal mode, synchronous, and (if set) secure delete.
pub fn configure_connection(
    connection: &Connection,
    options: &PragmaOptions,
) -> rusqlite::Result<()> {
    connection.busy_timeout(options.busy_timeout)?;
    connection.pragma_update(
        None,
        "foreign_keys",
        if options.foreign_keys { "ON" } else { "OFF" },
    )?;
    connection.pragma_update(None, "journal_mode", options.journal_mode)?;
    connection.pragma_update(None, "synchronous", options.synchronous)?;
    if let Some(secure_delete) = options.secure_delete {
        connection.pragma_update(
            None,
            "secure_delete",
            if secure_delete { "ON" } else { "OFF" },
        )?;
    }
    Ok(())
}

/// Run `PRAGMA quick_check` and report whether it returned `ok`.
pub fn quick_check(connection: &Connection) -> rusqlite::Result<bool> {
    let result: String = connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
    Ok(result == "ok")
}

/// Read `PRAGMA user_version`.
pub fn schema_version(connection: &Connection) -> rusqlite::Result<u32> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Open a connection with no pragmas applied yet.
pub fn open_raw(db_path: &Path) -> rusqlite::Result<Connection> {
    Connection::open(db_path)
}

/// Build the sidecar (or main-file) path for `suffix` (`"-wal"`, `"-shm"`,
/// or `""` for the main database file itself).
pub fn sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut path = db_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

/// Best-effort delete of a database's `-wal`/`-shm` sidecars. Missing files
/// are not an error. This is for intentional purges (e.g. trimming rotated
/// backups) — use [`quarantine_with_sidecars`] when the sidecars are
/// forensic evidence that must be preserved, not discarded.
pub fn remove_sidecars_best_effort(db_path: &Path) {
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(sidecar_path(db_path, suffix));
    }
}

/// Move a database file and any `-wal`/`-shm` sidecars it has into
/// quarantine, preserving them as separate evidence instead of deleting
/// them. `quarantine_path` is the destination for the main file; sidecars
/// land alongside it with matching suffixes.
///
/// Sidecars are moved first so that an interrupted quarantine still leaves
/// the main database file present (retried on the next recovery attempt).
pub fn quarantine_with_sidecars(db_path: &Path, quarantine_path: &Path) -> io::Result<()> {
    for suffix in ["-wal", "-shm", ""] {
        let source = sidecar_path(db_path, suffix);
        if source.exists() {
            let target = sidecar_path(quarantine_path, suffix);
            fs::rename(source, target)?;
        }
    }
    Ok(())
}

/// List the files directly inside `directory`, newest first. Ties (equal
/// modification time, or a modification time that could not be read) break
/// on descending filename, so ordering is deterministic even on filesystems
/// with coarse mtime resolution.
pub fn newest_first(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .map(|path| {
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (modified, path)
        })
        .collect();
    entries.sort_by(|(left_time, left_path), (right_time, right_path)| {
        right_time
            .cmp(left_time)
            .then_with(|| right_path.file_name().cmp(&left_path.file_name()))
    });
    Ok(entries.into_iter().map(|(_, path)| path).collect())
}

/// Milliseconds since the Unix epoch, for backup/quarantine file stamps and
/// record timestamps.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
