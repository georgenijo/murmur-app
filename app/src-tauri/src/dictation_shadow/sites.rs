//! The dictation lock D's acquisition sites (D-S2c-9 a, D-S2c-15 S3).
//!
//! The live shell counts every acquisition of D inside the guard function
//! (`MutexExt::lock_or_recover`), while armed. Some sites take D without taking a
//! decision section (a read of three fields, a fence held across a paste): they owe no
//! header and are **named** here, by file and the text of their line and of the next, so
//! an edit elsewhere in the file does not shift them.
//!
//! * [`classify`] says what a site is. Test builds apply it in the guard (a named or test
//!   site is not counted at all); a native build cannot read source files on the live
//!   thread, so the guard instead notes each acquisition's location in a fixed lock-free
//!   table ([`note`]: a few atomics, no allocation, no lock, no wait) and the comparison
//!   thread classifies the table when it writes the summary ([`snapshot`]). The
//!   summary's `conservation.non_family` is that table's named part, by `file:line`, and
//!   the entries it reports are the raw count minus those (an acquisition at a site whose
//!   source cannot be read stays in the entries, so it unbalances the books instead of
//!   hiding).

use std::collections::BTreeMap;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

/// Production sites that take the dictation lock D without taking a decision section: not
/// a family entry, so no header is owed for them (D-S2c-9 a). The guard function counts
/// **every** other acquisition while armed, and the replay requires each one to end in a
/// header, a named no-decision outcome or an unwind. A site is named by its file and the
/// text of its line and of the line after it (so an edit elsewhere does not shift it),
/// with the reason.
pub(crate) const NON_FAMILY: &[(&str, &str, &str, &str)] = &[
    (
        "src/commands/recording.rs",
        "let dictation = app_state.dictation.lock_or_recover();",
        "(",
        "the code-vocabulary prompt's fast-path read of three fields",
    ),
    (
        "src/commands/recording.rs",
        "let dictation = app_state.dictation.lock_or_recover();",
        "let Some(current_code_vocab) = cached_code_vocab_prompt(&dictation, bundle_id) else {",
        "the context snapshot's code-vocabulary revalidation",
    ),
    (
        "src/commands/recording.rs",
        "let dictation = state.app_state.dictation.lock_or_recover();",
        "project_lifecycle_view(&state.app_state, &dictation)",
        "query 402, the short read after a smart-destination capture (D-S2-3)",
    ),
    (
        "src/commands/recording.rs",
        "let dictation = app_state.dictation.lock_or_recover();",
        "if !owns_delivery(app_state, &dictation, recording_id) {",
        "the delivery ownership fence (query 403): held across a paste, not a decision",
    ),
    (
        "src/commands/recording.rs",
        "let dictation = app_state.dictation.lock_or_recover();",
        "drop(wait);",
        "the timed delivery ownership fence (query 403)",
    ),
    (
        "src/commands/recording.rs",
        "let _dictation = app_state.dictation.lock_or_recover();",
        "if app_state.recording_id.load(Ordering::SeqCst) != recording_id",
        "the generation fence around an emit: held across it, not a decision",
    ),
    (
        "src/commands/recording/lifecycle_ports.rs",
        "let dictation = state.app_state.dictation.lock_or_recover();",
        "register_partial_ticker_locked",
        "the ticker registration `SD` makes inside its own section's spawn",
    ),
    (
        "src/dictation_admission.rs",
        "s.read(|o| facts::status(app_state.dictation.lock_or_recover().status, o))",
        "});",
        "the admission probe's read of the status",
    ),
];

/// What an acquisition site is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// Test code: never a decision section, never counted in a test build.
    Test,
    /// A named non-family site ([`NON_FAMILY`]).
    NonFamily,
    /// Anything else: a decision section, owed a header.
    Family,
    /// The caller's source file cannot be read here, so the site cannot be told from a
    /// family one. Treated as family.
    Unreadable,
}

/// What the acquisition at `at` is. Reads the source file (call it off the live path).
pub(crate) fn classify(at: &Location<'_>) -> Site {
    let file = at.file();
    if file.ends_with("_tests.rs")
        || file.ends_with("_harness.rs")
        || file.contains("/tests/")
        || file.contains("dictation_shadow/")
    {
        return Site::Test;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let name = std::path::Path::new(file);
    let Some(text) = [name.to_path_buf(), manifest.join(name)]
        .into_iter()
        .chain(
            name.strip_prefix("app/src-tauri")
                .ok()
                .map(|rest| manifest.join(rest)),
        )
        .find_map(|path| std::fs::read_to_string(path).ok())
    else {
        return Site::Unreadable;
    };
    let lines: Vec<&str> = text.lines().collect();
    let line = lines
        .get((at.line() as usize).wrapping_sub(1))
        .map(|line| line.trim())
        .unwrap_or_default();
    // An inline `#[cfg(test)] mod x {`: its lines, up to the `}` at column 0 that rustfmt
    // puts at its end, are test code.
    let in_test_module = lines
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| {
            pair[0].trim() == "#[cfg(test)]"
                && pair[1].trim_start().starts_with("mod ")
                && pair[1].trim_end().ends_with('{')
        })
        .any(|(start, _)| {
            let end = lines[start + 2..]
                .iter()
                .position(|line| *line == "}")
                .map_or(lines.len(), |offset| start + 2 + offset + 1);
            (start + 2..=end).contains(&(at.line() as usize))
        });
    if in_test_module {
        return Site::Test;
    }
    let next = lines
        .get(at.line() as usize)
        .map(|line| line.trim())
        .unwrap_or_default();
    let named = NON_FAMILY.iter().any(|(path, text, after, _)| {
        file.ends_with(path)
            && line == *text
            && (next == *after || (after.len() > 1 && next.contains(*after)))
    });
    if named {
        Site::NonFamily
    } else {
        Site::Family
    }
}

// ------------------------------------------------------ the lock-free acquisition table

/// Distinct acquisition sites the table can hold. The app has a few dozen; a full table
/// counts the excess in [`overflow`], which the summary reports as unclassified.
const SLOTS: usize = 128;

struct Slot {
    /// A non-zero hash of the site (file and line); 0 is a free slot.
    key: AtomicU64,
    count: AtomicU64,
    /// Written once, by the thread that claimed the slot; read by the comparison thread.
    at: OnceLock<&'static Location<'static>>,
}

#[allow(clippy::declare_interior_mutable_const)]
const FREE: Slot = Slot {
    key: AtomicU64::new(0),
    count: AtomicU64::new(0),
    at: OnceLock::new(),
};
static TABLE: [Slot; SLOTS] = [FREE; SLOTS];
static OVERFLOW: AtomicU64 = AtomicU64::new(0);

fn key_of(at: &Location<'_>) -> u64 {
    // FNV-1a over the file and the line: stable within a process, never 0.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in at.file().bytes().chain(at.line().to_le_bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash | 1
}

/// Note one acquisition of D at `at`: bounded probing over atomics, no allocation, no
/// lock, no wait. The live thread's whole cost.
pub(crate) fn note(at: &'static Location<'static>) {
    let key = key_of(at);
    let start = (key as usize) % SLOTS;
    for step in 0..SLOTS {
        let slot = &TABLE[(start + step) % SLOTS];
        match slot
            .key
            .compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                let _ = slot.at.set(at);
                slot.count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(found) if found == key => {
                slot.count.fetch_add(1, Ordering::Relaxed);
                return;
            }
            Err(_) => {}
        }
    }
    OVERFLOW.fetch_add(1, Ordering::Relaxed);
}

/// Acquisitions the table had no slot for.
pub(crate) fn overflow() -> u64 {
    OVERFLOW.load(Ordering::Relaxed)
}

/// Every site noted so far with its count. A slot claimed a moment ago whose location
/// is not published yet is skipped: its count appears at the next write.
pub(crate) fn snapshot() -> Vec<(&'static Location<'static>, u64)> {
    TABLE
        .iter()
        .filter_map(|slot| {
            let at = slot.at.get()?;
            Some((*at, slot.count.load(Ordering::Relaxed)))
        })
        .collect()
}

/// The named non-family acquisitions noted so far, by `file:line`, and the count of
/// those that could not be classified. Classifies each site once (the comparison thread
/// calls this; it reads source files).
pub(crate) fn non_family_counts() -> (BTreeMap<String, u64>, u64) {
    static CACHE: Mutex<BTreeMap<(&'static str, u32), Site>> = Mutex::new(BTreeMap::new());
    let mut named = BTreeMap::new();
    let mut unclassified = overflow();
    for (at, count) in snapshot() {
        let key = (at.file(), at.line());
        let cached = CACHE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .copied();
        let site = cached.unwrap_or_else(|| {
            let site = classify(at);
            CACHE
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, site);
            site
        });
        match site {
            Site::NonFamily => {
                *named
                    .entry(format!("{}:{}", at.file(), at.line()))
                    .or_default() += count;
            }
            Site::Unreadable => unclassified += count,
            Site::Test | Site::Family => {}
        }
    }
    (named, unclassified)
}
