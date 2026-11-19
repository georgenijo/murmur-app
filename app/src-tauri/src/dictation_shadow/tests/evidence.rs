//! The soak evidence the summary carries (D-S2c-15 S1, S2, S3, S4, S7): who wrote it,
//! what was left open, what was credited at a close, the entry/header books, the
//! persisted-core divergence count, and the reply's `state` and error class.

use std::sync::atomic::AtomicU64;
use std::time::Duration;

use murmur_dictation_core::replies::{ErrText, ProcessAudioRefusal, Reply, ReplyType};
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::summary::{
    ConservationCounts, ConservationReport, Session, Stats, EVENTS_FILE, SUMMARY_FILE,
};
use crate::dictation_shadow::types::{DiffKind, ErrClass, NoReplyReason, ReplyKind, ShadowMsg};
use crate::dictation_shadow::{spawn_thread, Control};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("murmur-evidence-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read_summary(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap()).unwrap()
}

// ------------------------------------------------------------------ S1: the session

/// An armed start writes a session block at once and again, final, at a clean exit; it
/// starts the events file over, and a summary a pass reads names this process.
#[test]
fn an_armed_start_stamps_the_session_and_truncates_the_events_file() {
    let dir = scratch("session");
    // An earlier session's leftovers.
    std::fs::write(dir.join(EVENTS_FILE), "{\"stale\":1}\n").unwrap();
    std::fs::write(dir.join(SUMMARY_FILE), "{\"compared\":999}").unwrap();
    let control: &'static Control = Box::leak(Box::new(Control::new()));
    let unused: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));
    let transport = default_transport();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let thread = spawn_thread(
        move || transport,
        control,
        Duration::from_millis(1),
        unused,
        {
            let dir = dir.clone();
            move || Some(dir)
        },
    )
    .expect("the thread spawns");
    control.request_shutdown();
    assert!(control.wait_finished(Duration::from_secs(10)));
    thread.join().unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join(EVENTS_FILE)).unwrap(),
        "",
        "the events file starts over"
    );
    let summary = read_summary(&dir);
    assert_ne!(summary["compared"], 999, "the old summary was replaced");
    let session = &summary["session"];
    assert_eq!(session["pid"], std::process::id());
    assert_eq!(session["armed"], true);
    assert!(session["started_unix_ms"].as_u64().unwrap() >= before);
    let sha = session["git_sha"].as_str().unwrap();
    assert!(
        sha == "unknown" || (sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit())),
        "git_sha is 40 hex or unknown: {sha}"
    );
    assert!(!session["build_profile"].as_str().unwrap().is_empty());
    assert_eq!(summary["final"], true, "the exit write is the final one");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A disarmed start spawns nothing and so writes nothing: `start` returns before it
/// reaches a file or the session record.
#[test]
fn a_disarmed_start_writes_nothing() {
    assert!(!crate::dictation_shadow::enabled());
    crate::dictation_shadow::start();
    assert!(
        crate::dictation_shadow::STARTED.get().is_none(),
        "a disarmed start must not spawn the comparison thread"
    );
}

#[test]
fn the_build_script_records_the_revision_it_was_built_from() {
    let session = Session::current();
    assert!(session.armed);
    assert_eq!(session.pid, std::process::id());
    // Built from a git checkout (CI and the soak): a full hash. `unknown` is only a
    // checkout with no repository.
    assert!(
        session.git_sha == "unknown"
            || (session.git_sha.len() == 40
                && session.git_sha.bytes().all(|b| b.is_ascii_hexdigit())),
        "{}",
        session.git_sha
    );
}

// ------------------------------------------------------ S2: credit only completed flights

#[test]
fn a_header_is_compared_but_only_the_completed_close_is_credited_as_matched() {
    let mut h = harness();
    let flight = short_idle(1, 1);
    let (site, row) = (flight.header.site_id, flight.header.row);
    h.feed_header(&flight);
    let credit = |h: &Harness| {
        let stats = h
            .stats()
            .per_row
            .get(&(site, row))
            .copied()
            .unwrap_or_default();
        (stats.compared, stats.matched)
    };
    assert_eq!(
        credit(&h),
        (1, 0),
        "compared at the header, matched not yet"
    );
    assert_eq!(h.comparator.open_flights(), 1, "the flight is outstanding");
    h.feed_later(&flight);
    assert_eq!(credit(&h), (1, 1), "matched at the completed close");
    assert_eq!(h.comparator.open_flights(), 0);
    let json = h.stats().to_json();
    assert_eq!(json["per_row"][format!("{site}.{row}")]["matched"], 1);
    assert_eq!(json["incomplete_closes"], serde_json::json!({}));
}

/// A flight that was abandoned (its close says `completed:false`) with effects still
/// outstanding is neither a match nor expected: it is an incomplete close with a reason,
/// and E3 gets no credit.
#[test]
fn an_abandoned_flight_is_an_unexpected_incomplete_close_and_earns_no_credit() {
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    let (site, row) = (flight.header.site_id, flight.header.row);
    // Drop the last effect's witness, then close incomplete.
    let at = flight
        .later
        .iter()
        .rposition(|msg| matches!(msg, ShadowMsg::Witness { .. }))
        .expect("a start has later effects");
    flight.later.truncate(at);
    flight.later.push(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    h.feed(&flight);
    let stats = h.stats();
    assert_eq!(stats.per_row[&(site, row)].matched, 0);
    assert_eq!(stats.abandoned, 1);
    assert_eq!(stats.incomplete_closes.get("abandoned"), Some(&1));
    assert_eq!(stats.named_no_reply, 0);
    assert_eq!(h.comparator.open_flights(), 0);
    assert_eq!(h.stats().to_json()["incomplete_closes"]["abandoned"], 1);
}

/// The named no-reply close: the producer announced it, every decided effect ran, the
/// reply never came and the row's history was witnessed. Expected: counted on its own,
/// never an incomplete close.
#[test]
fn the_announced_no_reply_close_is_expected() {
    let mut h = harness();
    let mut flight = stale_stop(1, 1);
    flight
        .later
        .retain(|msg| !matches!(msg, ShadowMsg::Reply { .. } | ShadowMsg::Close { .. }));
    flight.later.push(ShadowMsg::NoReply {
        rec: flight.header.rec,
        reason: NoReplyReason::Deferred,
    });
    flight.later.push(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    h.feed(&flight);
    let stats = h.stats();
    assert_eq!(stats.named_no_reply, 1, "{stats:?}");
    assert!(stats.incomplete_closes.is_empty(), "{stats:?}");
    assert_eq!(stats.abandoned, 1);
    assert_eq!(h.comparator.open_flights(), 0);
}

/// D-S2c-15 review, major 1: an ordinary abandoned reply is not the named close. The
/// same flight as above (every effect ran, the reply never came) but **dropped**, with no
/// announcement, is an unexpected incomplete close.
#[test]
fn an_ordinary_abandoned_reply_is_not_the_named_no_reply_close() {
    let mut h = harness();
    let mut flight = stale_stop(1, 1);
    flight
        .later
        .retain(|msg| !matches!(msg, ShadowMsg::Reply { .. } | ShadowMsg::Close { .. }));
    flight.later.push(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    h.feed(&flight);
    let stats = h.stats();
    assert_eq!(stats.named_no_reply, 0, "{stats:?}");
    assert_eq!(
        stats.incomplete_closes.get("abandoned"),
        Some(&1),
        "{stats:?}"
    );
    assert_eq!(h.comparator.open_flights(), 0);
}

/// An announcement alone grants nothing: the exemption also needs the row's history
/// witnessed. Two flights announce the same no-reply close after all their effects ran:
/// the pipeline-owned finish that reported both of the owner's retirements is exempt; the
/// one that did not is unexpected.
#[test]
fn an_announced_no_reply_close_needs_its_history_witnesses() {
    let announce = |keep_history: bool| {
        let mut h = harness();
        // The owner's pipeline registers first (its token is what the finish retires).
        h.feed(&pipeline_start(1, 1));
        let mut finish = legacy_parse(2, 2, false);
        assert!(
            finish.decision.reply.is_some(),
            "the finish decides a reply"
        );
        finish
            .later
            .retain(|msg| !matches!(msg, ShadowMsg::Reply { .. } | ShadowMsg::Close { .. }));
        if !keep_history {
            finish
                .later
                .retain(|msg| !matches!(msg, ShadowMsg::History { .. }));
            finish.header.live_ops = Default::default();
        }
        finish.later.push(ShadowMsg::NoReply {
            rec: finish.header.rec,
            reason: NoReplyReason::Deferred,
        });
        finish.later.push(ShadowMsg::Close {
            rec: finish.header.rec,
            completed: false,
        });
        h.feed(&finish);
        h
    };
    let with = announce(true);
    assert_eq!(with.stats().named_no_reply, 1, "{:?}", with.stats());
    let without = announce(false);
    assert_eq!(without.stats().named_no_reply, 0, "{:?}", without.stats());
    assert_eq!(
        without.stats().incomplete_closes.get("abandoned"),
        Some(&1),
        "{:?}",
        without.stats()
    );
}

/// A delegated row's completion is its announced no-reply close, credited to the row as
/// `delegated`: it never has a `matched` of its own.
#[test]
fn a_delegated_row_is_credited_delegated_not_matched() {
    let mut h = harness();
    let mut flight = stale_stop(1, 1);
    let (site, row) = (flight.header.site_id, flight.header.row);
    flight
        .later
        .retain(|msg| !matches!(msg, ShadowMsg::Reply { .. } | ShadowMsg::Close { .. }));
    flight.later.push(ShadowMsg::NoReply {
        rec: flight.header.rec,
        reason: NoReplyReason::Delegated,
    });
    flight.later.push(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    h.feed(&flight);
    let stats = h.stats().per_row[&(site, row)];
    assert_eq!((stats.matched, stats.delegated), (0, 1));
    assert_eq!(
        h.stats().to_json()["per_row"][format!("{site}.{row}")]["delegated"],
        1
    );
}

/// A header with no close is outstanding, and the final summary says so: a clean exit
/// needs 0.
#[test]
fn flights_left_open_at_exit_are_reported() {
    let dir = scratch("outstanding");
    let transport = default_transport();
    let control = Control::new();
    let mut comparator = harness().comparator;
    // One flight's header and nothing else: its close never arrives.
    let flight = short_idle(0, 0);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    control.request_shutdown();
    // The recorder is dropped only after the final summary: it stays outstanding.
    comparator.stats.session = Some(Session::current());
    crate::dictation_shadow::run(
        &transport,
        comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    let summary = read_summary(&dir);
    assert_eq!(summary["outstanding_at_exit"], 1, "{summary}");
    assert_eq!(summary["final"], true);
    drop(recorder);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clean_exit_has_no_outstanding_flights() {
    let dir = scratch("clean-exit");
    let transport = default_transport();
    let control = Control::new();
    let comparator = harness().comparator;
    let flight = short_idle(0, 0);
    let recorder = transport.recorder();
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    control.request_shutdown();
    crate::dictation_shadow::run(
        &transport,
        comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    let summary = read_summary(&dir);
    assert_eq!(summary["outstanding_at_exit"], 0, "{summary}");
    assert_eq!(summary["matched"], 1);
    assert_eq!(summary["incomplete_closes"], serde_json::json!({}));
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ S3: conservation

#[test]
fn conservation_balances_exactly_when_every_entry_ended_in_a_header_a_named_outcome_or_an_unwind() {
    let balanced = ConservationCounts {
        entries: 10,
        headers: 7,
        no_decision: 2,
        unwound: 1,
    };
    assert!(balanced.balanced());
    for broken in [
        ConservationCounts {
            entries: 11,
            ..balanced
        },
        ConservationCounts {
            headers: 6,
            ..balanced
        },
        ConservationCounts {
            no_decision: 3,
            ..balanced
        },
        ConservationCounts {
            unwound: 0,
            ..balanced
        },
    ] {
        assert!(!broken.balanced(), "{broken:?}");
    }
}

#[test]
fn the_summary_writes_the_books_per_site_and_names_the_non_family_acquisitions() {
    let mut h = harness();
    let flight = short_idle(1, 1);
    let site = flight.header.site_id;
    h.feed(&flight);
    let mut stats = h.stats().clone();
    stats.conservation = Some(ConservationReport {
        total: ConservationCounts {
            entries: 1,
            headers: 1,
            no_decision: 0,
            unwound: 0,
        },
        non_family: [("src/commands/recording.rs:6619".to_string(), 3)]
            .into_iter()
            .collect(),
        unclassified: 0,
    });
    let json = stats.to_json();
    let books = &json["conservation"];
    assert_eq!(books["total"]["entries"], 1);
    assert_eq!(books["total"]["balanced"], true);
    assert_eq!(books[site.to_string()]["headers"], 1, "{books}");
    assert_eq!(books["non_family"]["src/commands/recording.rs:6619"], 3);
    assert_eq!(books["unclassified"], 0);
    // No conservation section until the comparison thread read the books.
    assert!(Stats::default().to_json().get("conservation").is_none());
}

/// An imbalance is visible in the summary, not hidden by it.
#[test]
fn an_unbalanced_book_is_written_as_unbalanced() {
    let stats = Stats {
        conservation: Some(ConservationReport {
            total: ConservationCounts {
                entries: 5,
                headers: 4,
                no_decision: 0,
                unwound: 0,
            },
            ..ConservationReport::default()
        }),
        ..Stats::default()
    };
    assert_eq!(stats.to_json()["conservation"]["total"]["balanced"], false);
}

// ---------------------------------------------------------------- S4: core divergences

#[test]
fn the_summary_carries_the_persisted_core_divergence_count() {
    let dir = scratch("divergences");
    let transport = default_transport();
    let control = Control::new();
    control.request_shutdown();
    crate::dictation_shadow::run(
        &transport,
        harness().comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    let summary = read_summary(&dir);
    assert_eq!(
        summary["core_divergences"],
        crate::dictation_executor::adapter::core_divergences(),
        "{summary}"
    );
    assert!(summary["conservation"]["total"]["entries"].is_u64());
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ S7: the reply, in detail

fn reply_slot(flight: &mut Flight) -> &mut ReplyKind {
    flight
        .later
        .iter_mut()
        .find_map(|msg| match msg {
            ShadowMsg::Reply { kind, .. } => Some(kind),
            _ => None,
        })
        .expect("the decision replies")
}

/// R201.1 replies `recording_superseded` with a `state`: the live reply naming the same
/// type with another `state` is `diff_kind 4`.
#[test]
fn a_wrong_reply_state_is_a_reply_mismatch() {
    let mut h = harness();
    let mut flight = stale_stop(1, 1);
    let decided = *reply_slot(&mut flight);
    let ReplyKind::TypedAt(ty, state) = decided else {
        panic!("R201.1 decides a typed reply: {decided:?}");
    };
    assert_eq!(ty, ReplyType::RecordingSuperseded);
    let wrong = Status::ALL
        .into_iter()
        .find(|candidate| *candidate != state)
        .unwrap();
    *reply_slot(&mut flight) = ReplyKind::TypedAt(ty, wrong);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Reply.code());
    assert_eq!(DiffKind::Reply.code(), 4);

    // The decided state is a match.
    let mut h = harness();
    h.feed(&stale_stop(1, 1));
    assert_clean(&h);
}

/// A refusal replies `Err(ErrText::ProcessAudio(MeetingActive))`: the live reply naming
/// another error class is `diff_kind 4`; the same class is a match. (The production
/// wrapper's classification of the returned text is tested in `capture.rs`.)
#[test]
fn a_wrong_err_text_variant_is_a_reply_mismatch() {
    let mut flight_ = process_audio_refused(1, 1);
    assert_eq!(
        flight_.decision.reply,
        Some(Reply::Err(ErrText::ProcessAudio(
            ProcessAudioRefusal::MeetingActive
        )))
    );
    assert_eq!(
        *reply_slot(&mut flight_),
        ReplyKind::ErrAs(ErrClass::ProcessAudio(ProcessAudioRefusal::MeetingActive))
    );

    for wrong in [
        ErrClass::Shell,
        ErrClass::MissingContext,
        ErrClass::SmartAuto,
        ErrClass::Unclassified,
        ErrClass::ProcessAudio(ProcessAudioRefusal::Benchmark),
    ] {
        let mut h = harness();
        let mut damaged = process_audio_refused(1, 1);
        *reply_slot(&mut damaged) = ReplyKind::ErrAs(wrong);
        h.feed(&damaged);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Reply.code(),
            "{wrong:?}"
        );
    }
    // A plain `Err` (no class) agrees with nothing.
    let mut h = harness();
    let mut damaged = process_audio_refused(1, 1);
    *reply_slot(&mut damaged) = ReplyKind::Err;
    h.feed(&damaged);
    assert_eq!(h.only_event().diff_kind, DiffKind::Reply.code());

    let mut h = harness();
    h.feed(&process_audio_refused(1, 1));
    assert_clean(&h);
}

#[test]
fn reply_kinds_carry_closed_content_free_discriminants() {
    // `ErrClass` drops the recording id of `MissingContext` and every error text.
    assert_eq!(
        ErrClass::of(&ErrText::MissingContext(
            murmur_dictation_core::ids::RecordingId(7)
        )),
        ErrClass::MissingContext
    );
    // A typed reply without a state agrees on the kind alone (a producer that could
    // not read it), a detailed one is held to its state.
    let decided = ReplyKind::of(&Reply::typed(ReplyType::NotRecording, Status::Idle));
    assert!(decided.agrees(ReplyKind::Typed(ReplyType::NotRecording)));
    assert!(decided.agrees(ReplyKind::TypedAt(ReplyType::NotRecording, Status::Idle)));
    assert!(!decided.agrees(ReplyKind::TypedAt(
        ReplyType::NotRecording,
        Status::Recording
    )));
    assert!(!decided.agrees(ReplyKind::TypedAt(
        ReplyType::AlreadyRecording,
        Status::Idle
    )));
    assert!(!decided.agrees(ReplyKind::Err));
    assert!(!decided.agrees(ReplyKind::Unrecognized));
    // Shell-owned error text: an unclassifiable live text agrees, and is counted; a fixed
    // class never agrees with it.
    let shell = ReplyKind::ErrAs(ErrClass::Shell);
    let unclassified = ReplyKind::ErrAs(ErrClass::Unclassified);
    assert!(shell.agrees(unclassified) && shell.is_unclassified_agreement(unclassified));
    let fixed = ReplyKind::ErrAs(ErrClass::MissingContext);
    assert!(!fixed.agrees(unclassified) && !fixed.is_unclassified_agreement(unclassified));
}

// ------------------------------------------------- major 2: shutdown quiesces the producers

/// After shutdown begins, a push is refused and counted, never queued behind the final
/// drain; nothing is lost or abandoned by it.
#[test]
fn a_push_after_shutdown_began_is_refused_and_counted() {
    let transport = local_transport(8);
    let recorder = transport.recorder();
    assert!(transport.close_and_quiesce(Duration::from_millis(50)));
    crate::dictation_shadow::transport::mark(
        &recorder,
        crate::dictation_shadow::transport::Mark::Reply(ReplyKind::Ok),
    );
    assert_eq!(transport.unconsumed(), 0, "nothing entered the queue");
    assert_eq!(transport.after_close(), 1);
    assert_eq!(transport.lost(), 0, "a refused push is not a loss");
}

/// The final summary names every message the producers published after shutdown began:
/// a complete flight with a wrong reply that arrives at the boundary is not silently
/// absent from a summary that says it is final and clean.
#[test]
fn a_flight_published_at_the_shutdown_boundary_is_reported_never_silently_absent() {
    let dir = scratch("boundary");
    let transport = default_transport();
    let control = Control::new();
    // Shutdown has begun (the producers see it) ...
    assert!(transport.close_and_quiesce(Duration::from_millis(50)));
    // ... and a whole flight, its reply wrong, is published behind it.
    let recorder = transport.recorder();
    let mut flight = stale_stop(0, 0);
    *flight
        .later
        .iter_mut()
        .find_map(|msg| match msg {
            ShadowMsg::Reply { kind, .. } => Some(kind),
            _ => None,
        })
        .unwrap() = ReplyKind::Err;
    submit_header(&recorder, &flight);
    send_later(&recorder, &flight);
    let sent = transport.after_close();
    assert!(
        sent >= 3,
        "header, effects, reply and close were refused: {sent}"
    );
    control.request_shutdown();
    crate::dictation_shadow::run(
        &transport,
        harness().comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    let summary = read_summary(&dir);
    assert_eq!(summary["final"], true);
    assert_eq!(summary["dropped_after_shutdown"], sent, "{summary}");
    assert!(summary["dropped_after_shutdown"].as_u64().unwrap() > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A push still under way when the deadline passes is reported: `shutdown_quiesced`
/// is false, and the pass is tainted by it.
#[test]
fn a_producer_that_does_not_go_quiet_by_the_deadline_is_reported() {
    let dir = scratch("quiesce-fail");
    let transport = default_transport();
    let control = Control::new();
    transport.hold_inflight_for_test();
    control.request_shutdown();
    crate::dictation_shadow::run(
        &transport,
        harness().comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    transport.release_inflight_for_test();
    let summary = read_summary(&dir);
    assert_eq!(summary["shutdown_quiesced"], false, "{summary}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_quiet_shutdown_says_so() {
    let dir = scratch("quiesce-ok");
    let transport = default_transport();
    let control = Control::new();
    control.request_shutdown();
    crate::dictation_shadow::run(
        &transport,
        harness().comparator,
        Some(&dir),
        &control,
        Duration::from_millis(1),
    );
    let summary = read_summary(&dir);
    assert_eq!(summary["shutdown_quiesced"], true, "{summary}");
    assert_eq!(summary["dropped_after_shutdown"], 0);
    // A periodic summary has no such claim.
    assert!(Stats::default()
        .to_json()
        .get("shutdown_quiesced")
        .is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------------------- minor 1: the build's complete site map

#[track_caller]
fn here() -> &'static std::panic::Location<'static> {
    std::panic::Location::caller()
}

/// The map `build.rs` generated is exactly what the shared rules say of the source it
/// read, and it names the sites that matter: the overlay's `get_status` read owes no
/// header.
#[test]
fn the_generated_site_map_agrees_with_the_rules_and_names_get_status_non_family() {
    use crate::dictation_shadow::sites::{classify_text, site_map, Site};
    let map = site_map();
    assert!(map.len() > 300, "the map is the whole tree: {}", map.len());
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut by_file: std::collections::BTreeMap<&str, String> = Default::default();
    for (file, line, code) in map {
        let text = by_file
            .entry(file)
            .or_insert_with(|| std::fs::read_to_string(manifest.join(file)).unwrap());
        assert_eq!(
            classify_text(file, text, *line as usize).code(),
            *code,
            "{file}:{line}"
        );
    }
    let source = std::fs::read_to_string(manifest.join("src/commands/recording.rs")).unwrap();
    let lines: Vec<&str> = source.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains("pub async fn get_status("))
        .expect("get_status exists");
    let lock = start
        + 1
        + lines[start + 1..]
            .iter()
            .position(|line| line.contains(".lock_or_recover()"))
            .unwrap();
    let class = map
        .iter()
        .find(|(file, line, _)| *file == "src/commands/recording.rs" && *line as usize == lock + 1)
        .map(|(_, _, code)| Site::from_code(*code));
    assert_eq!(class, Some(Site::NonFamily), "get_status's read of D");
    // The decision sections are in the map as family entries.
    assert!(map
        .iter()
        .any(|(file, _, code)| file.ends_with("commands/recording.rs")
            && Site::from_code(*code) == Site::Family));
}

/// A location the map does not name is reported as unclassified, never assumed to be a
/// non-family read.
#[test]
fn a_location_outside_the_map_is_counted_unclassified() {
    use crate::dictation_shadow::sites::{lookup, non_family_counts, note, Site};
    let at = here();
    assert_eq!(lookup(at), Site::Unmapped);
    let (_, before) = non_family_counts();
    note(at);
    note(at);
    let (_, after) = non_family_counts();
    assert_eq!(after - before, 2);
}
