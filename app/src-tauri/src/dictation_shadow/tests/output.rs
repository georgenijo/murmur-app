//! What the shadow writes and where: numeric, file-only, by the comparison thread
//! (D-S2c-1 b and f).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::inputs::{Input, SupervisorEvent};
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::compare::{
    Comparator, EVENT_WINDOW_MS, MAX_EVENTS_PER_SESSION, MAX_HELD_PER_REC, MAX_OPEN_RECORDS,
    MAX_PENDING_HEADERS,
};
use crate::dictation_shadow::summary::{
    write_summary, EventFile, Stats, EVENTS_FILE, EVENT_CODE, SUMMARY_FILE,
};
use crate::dictation_shadow::transport::QUEUE_CAPACITY;
use crate::dictation_shadow::{file_sink, run, Control};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("murmur-shadow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A tracing subscriber layer that counts events, to prove a path is (not) connected.
struct Count(Arc<AtomicUsize>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Count {
    fn on_event(&self, _: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn leaves_are_numbers(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(_) => true,
        serde_json::Value::Object(map) => map.values().all(leaves_are_numbers),
        _ => false,
    }
}

#[test]
fn the_real_emission_path_writes_a_file_and_never_reaches_tracing() {
    // Review fix (Sol 6.2 blocker 2): the mismatch event went through `tracing::warn!`
    // and so through `TauriEmitterLayer`, app state and the event buffer, and the old
    // `shadow_never_executes` test never exercised that path. This one runs the real
    // emission path (the production sink, the production comparator) with a counting
    // subscriber that is proven connected.
    use tracing_subscriber::layer::SubscriberExt;
    let seen = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(Count(Arc::clone(&seen)));
    let dir = scratch("emission");
    tracing::subscriber::with_default(subscriber, || {
        // The counter is connected: an event on this thread is counted.
        tracing::warn!(target: "pipeline", "canary");
        assert_eq!(
            seen.load(Ordering::SeqCst),
            1,
            "the counter is not connected"
        );
        let mut comparator = Comparator::production(file_sink(Some(&dir)));
        let mut flight = short_idle(1, 1);
        flight.header.pre_effects.swap(1, 2);
        comparator.handle_msg(crate::dictation_shadow::types::ShadowMsg::Header(
            flight.header,
        ));
        assert_eq!(comparator.stats.mismatched_root, 1);
    });
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "the shadow emitted a tracing event: only the canary may be counted"
    );
    // The event is in the file, numeric and complete.
    let text = std::fs::read_to_string(dir.join(EVENTS_FILE)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "{text}");
    let event: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(event["event_code"], EVENT_CODE);
    assert_eq!(event["site_id"], 216);
    assert_eq!(event["input_kind"], 1);
    assert_eq!(event["diff_kind"], 2);
    assert_eq!(event.as_object().unwrap().len(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_event_file_validates_and_truncates() {
    let dir = scratch("events");
    let file = EventFile::begin(&dir).unwrap();
    assert!(file.append(216, 1, 2));
    assert!(!file.append(216, 1, 0), "diff_kind 0 is out of range");
    assert!(!file.append(216, 1, 10));
    assert!(!file.append(1, 1, 2), "1 is not a site");
    assert!(!file.append(216, 10_000, 2));
    assert_eq!(
        std::fs::read_to_string(dir.join(EVENTS_FILE))
            .unwrap()
            .lines()
            .count(),
        1
    );
    // A new session starts the file over.
    let _ = EventFile::begin(&dir).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(EVENTS_FILE)).unwrap(), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shadow_summary_is_numeric_complete_and_atomic() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    let mut broken = cancel(2, 2);
    broken.header.live_post_d.status = Status::Recording;
    h.feed(&broken);
    let json = h.stats().to_json();
    assert!(leaves_are_numbers(&json), "numbers only: {json}");
    let object = json.as_object().unwrap();
    for key in [
        "compared",
        "matched",
        "mismatched_root",
        "followup",
        "skipped_tainted",
        "skipped_incomplete",
        "dropped",
        "lost_gap",
        "lost_open",
        "invalidated",
        "late_header_discarded",
        "loss_unattributed",
        "abandoned",
        "abandoned_pre_d",
        "witness_overflow",
        "shadow_panic",
        "events_suppressed",
        "lost_by_site",
        "per_row",
        "pd_witness",
    ] {
        assert!(object.contains_key(key), "the summary lacks `{key}`");
    }
    assert_eq!(object["compared"], 2);
    assert_eq!(object["mismatched_root"], 1);
    let per_row = object["per_row"].as_object().unwrap();
    let row = &per_row["216.1"];
    assert_eq!(row["compared"], 1);
    assert_eq!(row["pd_witness"], 2, "R216.1 witnesses PD-1 and PD-15");
    assert_eq!(object["pd_witness"]["PD-1"], 1);
    // Keys are site.row and PD-n: no id, no content.
    for key in per_row.keys() {
        assert!(key.chars().all(|c| c.is_ascii_digit() || c == '.'), "{key}");
    }

    let dir = scratch("summary");
    write_summary(&dir, h.stats()).unwrap();
    let written = std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(parsed, json);
    assert_eq!(SUMMARY_FILE, "dictation-shadow-summary.json");
    // The temporary file is renamed over the target, never left behind.
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec![SUMMARY_FILE.to_string()]);
    // A second write replaces the first.
    write_summary(&dir, &Stats::default()).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap()).unwrap();
    assert_eq!(parsed["compared"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_summary_writers_never_fail_or_corrupt() {
    // Review fix (Sol 6.2 major 8): every writer used the same temporary name, so two
    // writers corrupted each other or one failed its rename.
    let dir = scratch("concurrent");
    let failures = Arc::new(AtomicU64::new(0));
    let mut workers = Vec::new();
    for n in 0..8u64 {
        let (dir, failures) = (dir.clone(), Arc::clone(&failures));
        workers.push(std::thread::spawn(move || {
            let stats = Stats {
                compared: n,
                ..Stats::default()
            };
            for _ in 0..60 {
                if write_summary(&dir, &stats).is_err() {
                    failures.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
    }
    for w in workers {
        w.join().unwrap();
    }
    assert_eq!(
        failures.load(Ordering::SeqCst),
        0,
        "a writer failed on a shared temporary file"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap()).unwrap();
    assert!(parsed["compared"].is_u64());
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec![SUMMARY_FILE.to_string()], "no stray temporary");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_final_summary_is_written_by_the_thread_after_it_drained_the_queue() {
    // Review fix (Sol 6.2 major 8): the exit hook snapshotted without draining, so a
    // clean exit wrote an incomplete summary. The thread polls slowly here, so the
    // messages are certainly still queued when the shutdown request arrives: the
    // summary must still count every one of them.
    let transport = default_transport();
    let dir = scratch("exit-drain");
    let control = Arc::new(Control::new());
    let comparator = harness().comparator;
    let thread = {
        let (transport, control, dir) = (Arc::clone(&transport), Arc::clone(&control), dir.clone());
        std::thread::spawn(move || {
            run(
                &transport,
                comparator,
                Some(&dir),
                &control,
                Duration::from_millis(300),
            );
        })
    };
    let flights = 20_u64;
    for n in 0..flights {
        let recorder = transport.recorder();
        let flight = idle_for(0, 0, 100 + n);
        submit_header(&recorder, &flight);
        send_later(&recorder, &flight);
    }
    control.request_shutdown();
    assert!(
        control.wait_finished(Duration::from_secs(10)),
        "the thread never finished"
    );
    thread.join().unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(SUMMARY_FILE)).unwrap()).unwrap();
    assert_eq!(parsed["compared"], flights, "{parsed}");
    assert_eq!(parsed["matched"], flights);
    assert_eq!(parsed["mismatched_root"], 0);
    // The thread is the only writer: nothing else is in the directory.
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec![SUMMARY_FILE.to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_summary_file_carries_no_ids() {
    // Recording ids appear in no key or value: run a flight whose recording id is
    // an unmistakable number and look for it in the document.
    let mut h = harness();
    let mut flight = flight(
        1,
        1,
        view(Status::Recovering, RecordingId(987_654_321)),
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
            rid: RecordingId(987_654_321),
        }),
    );
    flight.header.live_post_d.status = Status::Recording;
    h.feed(&flight);
    let text = h.stats().to_json().to_string();
    assert!(!text.contains("987654321"), "{text}");
    assert!(!text.contains("987_654_321"));
}

#[test]
fn shadow_summary_cadence_and_thread_name_are_the_documented_ones() {
    assert_eq!(
        crate::dictation_shadow::SUMMARY_INTERVAL,
        Duration::from_secs(300)
    );
    assert_eq!(
        crate::dictation_shadow::POLL_INTERVAL,
        Duration::from_millis(5)
    );
    assert_eq!(
        crate::dictation_shadow::THREAD_NAME,
        "murmur-dictation-shadow"
    );
    assert_eq!(QUEUE_CAPACITY, 256);
    assert_eq!(MAX_PENDING_HEADERS, 64);
    assert_eq!(MAX_OPEN_RECORDS, 64);
    assert_eq!(MAX_HELD_PER_REC, 32);
    assert_eq!(EVENT_WINDOW_MS, 60_000);
    assert_eq!(MAX_EVENTS_PER_SESSION, 50);
}
