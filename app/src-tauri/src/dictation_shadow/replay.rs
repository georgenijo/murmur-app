//! Test builds: the **synchronous replay** of the live capture sites (appendix C8.5
//! E7, E8).
//!
//! A replay arms the shadow without `MURMUR_SHADOW` and compares **on the calling
//! thread** (`Transport::set_sync`): no queue, no comparison thread, fully
//! deterministic. The golden traces and the bug replays run the real shell, so every
//! live capture site feeds the real comparator; a replay ends by checking that every
//! flight was closed exactly once, and returns what the comparison counted.
//!
//! Replays are process-global and are serialized by the caller (the golden harness
//! holds `GOLDEN_LOCK`); a test that arms one must not run beside tests that drive
//! the shell (`cargo test --lib -- --test-threads=1`, or nextest's process per test).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::compare::{Comparator, MismatchEvent};
use super::summary::Stats;
use super::transport::{Transport, QUEUE_CAPACITY};

/// One armed replay. [`Replay::end`] disarms it.
pub(crate) struct Replay {
    events: Arc<Mutex<Vec<MismatchEvent>>>,
    transport: Arc<Transport>,
    ended: bool,
}

/// The live side's entry/header counters (D-S2c-8 c), as a difference: the decision
/// sections the shell entered while armed (counted where it acquires D, or, for a
/// D-less entry, where it decides), the headers its flights submitted, the sections that
/// ended with a **named** no-decision outcome, the completed flights whose reply no
/// command return witnessed, and the flights an unwind ended. The harness that drives
/// the shell reads them (the replay may not name the live side) and hands the delta to
/// [`Report::conserved`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Conservation {
    pub entries: u64,
    pub headers: u64,
    pub no_decision: u64,
    pub unrendered: u64,
    /// Flights that ended as the named no-reply close (each is one `Close{completed:false}`
    /// the shell itself accounts for).
    pub no_reply: u64,
    /// Decisions dropped by an unwind before their header (each recorded as a loss).
    pub unwound: u64,
}

impl Conservation {
    pub(crate) fn since(self, before: Self) -> Self {
        Self {
            entries: self.entries - before.entries,
            headers: self.headers - before.headers,
            no_decision: self.no_decision - before.no_decision,
            unrendered: self.unrendered - before.unrendered,
            no_reply: self.no_reply - before.no_reply,
            unwound: self.unwound - before.unwound,
        }
    }
}

/// What a test that **provokes** a loss expects its sub-run to report (D-S2c-8 d). The
/// E7 gate requires none: `lost == 0`, `unwound == 0`, `unverifiable == 0`. A test that
/// drives a scenario in which the comparator cannot verify a finish, or an effect
/// unwinds, states that here, in the test, and the gate then requires exactly it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Expect {
    pub unverifiable: u64,
    pub unwound: u64,
    /// Flights whose header went out but whose close never arrived, because the unwind
    /// ended them (an incomplete close the test names; the gate otherwise requires 0).
    pub open: u64,
    /// Flights that closed before their decision was taken (a `Pre` effect unwound).
    pub abandoned_pre_d: u64,
    /// Flights that closed as the named no-reply close (incomplete, no `Reply` credited): a
    /// finish a deferred drain ran after the command that queued it returned, the import's
    /// own pipeline error, a flight with no command return to witness it.
    pub incomplete: u64,
    /// Flights that closed with `Close{completed:false}` after their header: a flight that
    /// never ran to its end (a dropped future, a named no-reply close). A completed flight
    /// whose `Close{completed:true}` was lost is one of these, so the gate requires 0 unless
    /// the test names them (D-S2c-9 b).
    pub abandoned: u64,
}

thread_local! {
    static EXPECT: std::cell::Cell<Expect> = const { std::cell::Cell::new(Expect { unverifiable: 0, unwound: 0, open: 0, abandoned_pre_d: 0, incomplete: 0, abandoned: 0 }) };
}

/// Called by the test that provokes the loss, before it drives the sub-run on this
/// thread. Consumed (reset) by the next gate.
pub(crate) fn expect(expectation: Expect) {
    EXPECT.with(|slot| slot.set(expectation));
}

/// What a replay's comparison counted.
pub(crate) struct Report {
    pub stats: Stats,
    pub events: Vec<MismatchEvent>,
    /// The live side's counters over this replay, once the harness handed them over.
    pub conservation: Option<Conservation>,
    /// Messages the live side could not hand over (a full ring, or a ticket dropped
    /// unsent by an unwind).
    pub lost: u64,
    /// Recorders still open when the replay ended: comparison is credited at close, so
    /// one still open was never compared to its end (D-S2c-9 b).
    pub open: usize,
}

/// Counters summed over every replay of this process (E3 and E8 evidence).
static TOTALS: Mutex<Option<Totals>> = Mutex::new(None);

#[derive(Clone, Debug, Default)]
pub(crate) struct Totals {
    pub replays: u64,
    pub compared: u64,
    pub mismatched_root: u64,
    pub abandoned: u64,
    pub unverifiable: u64,
    pub per_row: BTreeMap<(u16, u16), u64>,
    pub pd_witness: BTreeMap<u8, u64>,
    /// Ticker skips the comparator was told of, by reason (`TickerSkipReason::index`).
    pub ticker_skipped: [u64; super::types::TickerSkipReason::ALL.len()],
}

/// The totals so far.
pub(crate) fn totals() -> Totals {
    TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// Forget the totals (a test that measures from zero).
pub(crate) fn reset_totals() {
    *TOTALS.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

impl Replay {
    /// Arm the shadow in synchronous mode with the production comparator.
    pub(crate) fn begin() -> Self {
        let events: Arc<Mutex<Vec<MismatchEvent>>> = Arc::default();
        let sink_events = Arc::clone(&events);
        let comparator = Comparator::production(Box::new(move |event| {
            sink_events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event);
        }));
        // A transport of its own: `seq` starts at 1, in step with the comparator.
        let transport = Transport::new(QUEUE_CAPACITY);
        transport.set_sync(Some(comparator));
        super::set_test_transport(Some(Arc::clone(&transport)));
        Self {
            events,
            transport,
            ended: false,
        }
    }

    /// Disarm, check that every recorder opened was closed exactly once, and report.
    /// Call it after everything the replay drove has been dropped, so every flight
    /// that did not finish has sent its `Close{completed:false}`.
    pub(crate) fn end(mut self) -> Report {
        self.ended = true;
        super::set_test_transport(None);
        let (stats, open) = self.transport.with_sync(|comparator| {
            comparator.assert_conserved();
            (comparator.stats.clone(), comparator.open_flights())
        });
        self.transport.set_sync(None);
        let events = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut totals = TOTALS.lock().unwrap_or_else(PoisonError::into_inner);
        let totals = totals.get_or_insert_with(Totals::default);
        totals.replays += 1;
        totals.compared += stats.compared;
        totals.mismatched_root += stats.mismatched_root;
        totals.abandoned += stats.abandoned;
        totals.unverifiable += stats.unverifiable;
        for (row, row_stats) in &stats.per_row {
            *totals.per_row.entry(*row).or_default() += row_stats.compared;
            for (total, noted) in totals
                .ticker_skipped
                .iter_mut()
                .zip(row_stats.ticker_skipped)
            {
                *total += noted;
            }
        }
        for (pd, count) in &stats.pd_witness {
            *totals.pd_witness.entry(*pd).or_default() += count;
        }
        Report {
            stats,
            events,
            conservation: None,
            lost: self.transport.lost(),
            open,
        }
    }
}

// ------------------------------------------------------------ the site probe

/// The (site, row) pairs the armed replay has compared so far.
pub(crate) fn compared_rows_now() -> Vec<(u16, u16)> {
    super::test_transport()
        .map(|transport| {
            transport.with_sync(|comparator| {
                comparator
                    .stats
                    .per_row
                    .iter()
                    .filter(|(_, stats)| stats.compared > 0)
                    .map(|(row, _)| *row)
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// What the site probe saw over the process (C8.7 site-level pins).
#[derive(Clone, Debug, Default)]
pub(crate) struct SiteTotals {
    /// Header submissions, each checked for a free dictation lock D.
    pub submits: u64,
    /// Blocking points reached (`stop.perf_enter_vad`, `supervisor.start`), each
    /// checked for a header already compared.
    pub blocking_points: BTreeMap<&'static str, u64>,
    /// Descriptions of every violation, in order.
    pub violations: Vec<String>,
}

static SITE_TOTALS: Mutex<Option<SiteTotals>> = Mutex::new(None);

/// The site probe's totals so far.
pub(crate) fn site_totals() -> SiteTotals {
    SITE_TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

/// Forget the site probe's totals.
pub(crate) fn reset_site_totals() {
    *SITE_TOTALS.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

fn note_site(f: impl FnOnce(&mut SiteTotals)) {
    f(SITE_TOTALS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(SiteTotals::default));
}

/// A blocking effect is about to run (`point`): the flight's header must already have
/// been compared (D-S2c-1 / C8.7 `shadow_header_precedes_blocking_effect`, at the
/// site). `required` is the row the header belongs to; the probe passes when the
/// replay has compared it.
pub(crate) fn blocking_point(point: &'static str) {
    let required: &[(u16, u16)] = match point {
        // R203.9: `PDT:PE(Vad), PS` after the header.
        "stop.perf_enter_vad" => &[(203, 9)],
        // The cold start: the start decision that issues `CS` is compared already.
        "supervisor.start" => &[(101, 15)],
        _ => return,
    };
    let seen = compared_rows_now();
    note_site(|totals| {
        *totals.blocking_points.entry(point).or_default() += 1;
        if !required.iter().all(|row| seen.contains(row)) {
            totals.violations.push(format!(
                "`{point}` ran before its header was compared: needs {required:?}, compared {seen:?}"
            ));
        }
    });
}

/// Test builds: observe every header submission (D must be free at it) and the
/// blocking points of the barrier hook, on this thread, until dropped.
pub(crate) struct SiteProbe {
    _hook: crate::barrier_hook::Installed,
}

impl SiteProbe {
    /// `d_is_free` is true when the dictation lock D is not held.
    pub(crate) fn install(d_is_free: impl Fn() -> bool + 'static) -> Self {
        let hook = crate::barrier_hook::install(move |point| match point {
            "shadow.submit" => {
                let free = d_is_free();
                note_site(|totals| {
                    totals.submits += 1;
                    if !free {
                        totals
                            .violations
                            .push("a header was submitted while D was held".to_string());
                    }
                });
            }
            "stop.perf_enter_vad" => blocking_point("stop.perf_enter_vad"),
            _ => {}
        });
        Self { _hook: hook }
    }
}

impl Drop for Replay {
    fn drop(&mut self) {
        // A replay abandoned by a panic must not leave the shadow armed for the next
        // test.
        if !self.ended {
            super::set_test_transport(None);
            self.transport.set_sync(None);
        }
    }
}

// ------------------------------------------------ the D guard's non-family acquisitions

/// Production sites that take the dictation lock D without taking a decision section: not
/// a family entry, so no header is owed for them (D-S2c-9 a). The guard function counts
/// **every** other acquisition while armed, and the replay requires each one to end in a
/// header, a named no-decision outcome or an unwind. A site is named by its file and the
/// text of its line and of the line after it (so an edit elsewhere does not shift it),
/// with the reason.
const NON_FAMILY: &[(&str, &str, &str, &str)] = &[
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

/// Whether the acquisition at `at` is test code or a named non-family site: not counted
/// as a decision section. Cached per site; the first look reads the source file.
pub(crate) fn not_a_decision_section(at: &'static std::panic::Location<'static>) -> bool {
    static CACHE: Mutex<BTreeMap<(&'static str, u32), bool>> = Mutex::new(BTreeMap::new());
    let key = (at.file(), at.line());
    if let Some(known) = CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return *known;
    }
    let excluded = classify_site(at);
    CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, excluded);
    excluded
}

fn classify_site(at: &std::panic::Location<'_>) -> bool {
    let file = at.file();
    if file.ends_with("_tests.rs")
        || file.ends_with("_harness.rs")
        || file.contains("/tests/")
        || file.contains("dictation_shadow/")
    {
        return true;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let name = std::path::Path::new(file);
    let text = [name.to_path_buf(), manifest.join(name)]
        .into_iter()
        .chain(
            name.strip_prefix("app/src-tauri")
                .ok()
                .map(|rest| manifest.join(rest)),
        )
        .find_map(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_else(|| panic!("the D guard's caller file {file} cannot be read"));
    let lines: Vec<&str> = text.lines().collect();
    let line = lines
        .get(at.line() as usize - 1)
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
        return true;
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
    if !named && std::env::var_os("MURMUR_E7_SITES").is_some() {
        eprintln!("E7SITE {file}:{} {line}", at.line());
    }
    named
}

/// Sub-runs that by design drive no family entry (they exercise a ledger directly), so
/// `at least one decision compared` does not apply to them. Each is held to taking
/// none.
const NO_SHELL_ENTRY: &[&str] = &[
    // Ledger / telemetry scenarios that call the ledger or a host port directly.
    "PD-10 eviction after 128",
    "PD-11",
    // The model installer parked on its lock: the scenario drives the installer, not a
    // family entry.
    "117 installer blocked on the install lock",
    "117 install fails before parking",
    "117 dropping an installer blocked on the install lock",
];

impl Report {
    /// Hand over the live side's counters for this replay.
    pub(crate) fn conserved(mut self, conservation: Conservation) -> Self {
        self.conservation = Some(conservation);
        self
    }

    /// The E7 gate for one replay: no mismatch, no loss, no panic, nothing skipped.
    pub(crate) fn assert_clean(&self, what: &str) {
        let s = &self.stats;
        // Findings mode (D-S2c-6 e): record every divergence (row, site, diff kind) and
        // go on, so one run reports all of them together. The gate itself never runs in
        // this mode.
        if std::env::var_os("MURMUR_E7_COLLECT").is_some() {
            for event in &self.events {
                let rows: Vec<_> = s
                    .per_row
                    .iter()
                    .filter(|(row, stats)| row.0 == event.site_id && stats.mismatched > 0)
                    .map(|(row, _)| row.1)
                    .collect();
                eprintln!(
                    "E7FINDING sub_run={what:?} site={} rows={rows:?} input_kind={} diff_kind={}",
                    event.site_id, event.input_kind, event.diff_kind
                );
            }
            eprintln!(
                "E7ROWS sub_run={what:?} {:?}",
                s.per_row.keys().collect::<Vec<_>>()
            );
            return;
        }
        assert!(
            self.events.is_empty()
                && s.mismatched_root == 0
                && s.followup == 0
                && s.shadow_panic == 0
                && s.dropped == 0
                && s.lost_open == 0
                && s.abandoned_by_loss == 0
                && s.abandoned_by_panic == 0
                && s.witness_overflow == 0,
            "{what}: the shadow reported a divergence: events {:?}, stats {s:?}",
            self.events
        );
        // Entry/header conservation (D-S2c-8 c, d): a sub-run that compared nothing, took
        // a decision section without a header or a named no-decision outcome, lost a
        // message or closed a flight the comparator could not verify proves nothing,
        // unless the test that provoked the loss said so itself.
        let expected = EXPECT.with(|slot| slot.replace(Expect::default()));
        assert_eq!(
            s.unverifiable, expected.unverifiable,
            "{what}: {} flights closed unverifiable ({} expected by the test): {s:?}",
            s.unverifiable, expected.unverifiable
        );
        let c = self.conservation.unwrap_or_else(|| {
            panic!("{what}: the harness handed the replay no entry/header counters")
        });
        assert_eq!(
            c.entries,
            c.headers + c.no_decision + c.unwound,
            "{what}: the live shell entered {} decision sections but its flights submitted {} \
             headers, {} ended with a named no-decision outcome and {} were lost to an unwind \
             (a family entry returned without a capture)",
            c.entries,
            c.headers,
            c.no_decision,
            c.unwound
        );
        assert_eq!(
            c.unwound, expected.unwound,
            "{what}: {} decisions were lost to an unwind ({} expected by the test)",
            c.unwound, expected.unwound
        );
        if NO_SHELL_ENTRY.contains(&what) {
            // A ledger-only scenario: it must still enter no decision section (if it ever
            // drives the shell, it leaves this list and is held to the rule below).
            assert_eq!(
                (c.entries, s.compared),
                (0, 0),
                "{what}: listed as driving no family entry, but the shell entered sections"
            );
        } else {
            assert!(
                s.matched >= 1 && c.entries >= 1,
                "{what}: the sub-run matched {} decisions at close (entered {}): it proves \
                 nothing",
                s.matched,
                c.entries
            );
        }
        // The loss counter is the live side's own account: nothing is lost unless the
        // test provoked an unwind.
        assert_eq!(
            self.lost, expected.unwound,
            "{what}: the live side lost {} messages ({} expected by the test)",
            self.lost, expected.unwound
        );
        // Comparison is credited at close (D-S2c-9 b): nothing is left open, every header
        // ended in exactly one close, and the clean ones matched. A flight whose header
        // arrived but whose close never did (a missing reply, a lost tail) is open here,
        // whatever the comparison thought of its header. A test that provoked an unwind
        // states how many decisions it ended (`Expect::unwound`); each of those is a flight
        // whose header was never sent, so it is open or closed incomplete, and nothing else is.
        assert_eq!(
            s.abandoned_pre_d, expected.abandoned_pre_d,
            "{what}: {} flights closed before their decision ({} expected by the test)",
            s.abandoned_pre_d, expected.abandoned_pre_d
        );
        assert_eq!(
            self.open as u64, expected.open,
            "{what}: {} flights were still open at the end of the sub-run ({} expected by the \
             test): {s:?}",
            self.open, expected.open
        );
        assert_eq!(
            s.closed_total() - s.abandoned_pre_d + self.open as u64,
            c.headers,
            "{what}: {} headers were submitted, but {} closes were credited and {} flights \
             are still open ({s:?})",
            c.headers,
            s.closed_total(),
            self.open
        );
        assert_eq!(
            s.abandoned,
            c.no_reply + expected.abandoned,
            "{what}: {} flights closed without completing ({} named no-reply closes by the \
             shell, {} named by the test): {s:?}",
            s.abandoned,
            c.no_reply,
            expected.abandoned
        );
        assert_eq!(
            s.skipped_incomplete, expected.incomplete,
            "{what}: {} flights closed incomplete ({} named by the test): {s:?}",
            s.skipped_incomplete, expected.incomplete
        );
        assert_eq!(
            s.matched + s.unverifiable + s.abandoned_pre_d + s.skipped_incomplete,
            s.closed_total(),
            "{what}: a close was neither a match nor expected ({s:?})"
        );
        assert_eq!(
            c.unrendered, 0,
            "{what}: {} completed flights never rendered their reply at a command return",
            c.unrendered
        );
    }
}

/// The E7 gate itself (D-S2c-9 b): a replay in which a flight's close never arrived, a loss
/// was recorded or a flight was unverifiable fails, unless the test that provoked it said
/// so; and what it said is exactly what is required.
#[cfg(test)]
mod gate_tests {
    use super::*;

    /// One matched flight, one header, one entry: a clean replay.
    fn clean() -> Report {
        Report {
            stats: Stats {
                compared: 1,
                matched: 1,
                ..Stats::default()
            },
            events: Vec::new(),
            conservation: Some(Conservation {
                entries: 1,
                headers: 1,
                no_decision: 0,
                unrendered: 0,
                no_reply: 0,
                unwound: 0,
            }),
            lost: 0,
            open: 0,
        }
    }

    #[test]
    fn a_clean_replay_passes() {
        clean().assert_clean("clean");
    }

    #[test]
    #[should_panic(expected = "still open at the end")]
    fn a_flight_whose_close_never_arrived_fails_the_gate() {
        let mut report = clean();
        report.open = 1;
        report.conservation.as_mut().unwrap().headers = 2;
        report.conservation.as_mut().unwrap().entries = 2;
        report.assert_clean("never closed");
    }

    #[test]
    #[should_panic(expected = "headers were submitted")]
    fn a_header_without_a_close_fails_the_gate() {
        let mut report = clean();
        report.conservation.as_mut().unwrap().headers = 2;
        report.conservation.as_mut().unwrap().entries = 2;
        report.assert_clean("header without a close");
    }

    #[test]
    #[should_panic(expected = "closed unverifiable")]
    fn an_unverifiable_flight_fails_the_gate() {
        let mut report = clean();
        report.stats.unverifiable = 1;
        report.assert_clean("unverifiable");
    }

    #[test]
    #[should_panic(expected = "lost")]
    fn a_lost_message_fails_the_gate() {
        let mut report = clean();
        report.lost = 1;
        report.assert_clean("lost");
    }

    #[test]
    #[should_panic(expected = "closed incomplete")]
    fn an_incomplete_close_fails_the_gate_unless_named() {
        let mut report = clean();
        report.stats.skipped_incomplete = 1;
        report.conservation.as_mut().unwrap().headers = 2;
        report.conservation.as_mut().unwrap().entries = 2;
        report.assert_clean("incomplete");
    }

    #[test]
    #[should_panic(expected = "closed without completing")]
    fn a_flight_that_never_completed_fails_the_gate_unless_named() {
        // Every `Close{completed:true}` dropped degrades to `Close{completed:false}`: the
        // comparator calls the clean record matched, so only this counter shows it.
        let mut report = clean();
        report.stats.abandoned = 1;
        report.assert_clean("abandoned");
    }

    #[test]
    fn what_a_test_names_is_exactly_what_the_gate_accepts() {
        // A named incomplete close passes; the same replay unnamed fails (above).
        let mut report = clean();
        report.stats.matched = 1;
        report.stats.skipped_incomplete = 1;
        report.conservation.as_mut().unwrap().headers = 2;
        report.conservation.as_mut().unwrap().entries = 2;
        expect(Expect {
            incomplete: 1,
            ..Expect::default()
        });
        report.assert_clean("named incomplete");
        // A named expectation that does not happen fails too.
        let result = std::panic::catch_unwind(|| {
            expect(Expect {
                open: 1,
                ..Expect::default()
            });
            clean().assert_clean("named but absent");
        });
        assert!(result.is_err(), "an expectation nothing met must fail");
        // The panic above left the expectation consumed: nothing leaks to the next gate.
        clean().assert_clean("clean again");
    }
}
