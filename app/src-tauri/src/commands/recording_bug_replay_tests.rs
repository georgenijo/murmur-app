//! Stage 4 historical bug replays, shell half (appendix C13.3, plan §6 Stage 4).
//!
//! One test per bug named in the C13.3 matrix. Each drives today's commands
//! through the Stage 0a `DictationHost` seam on the Stage 0b harness (fakes, no
//! clock, causal barriers only) and asserts the row's oracle. The core half of
//! the same matrix (`admit` rows and `decide_*` rows through the public API) is
//! `crates/dictation-core/tests/c13_3_bug_replays.rs`.
//!
//! These tests add nothing to the golden fixtures and change no golden
//! assertion. Rows 1, 2, 4 and 5 are also Stage 0b goldens (`golden_s01`,
//! `golden_s05`, `golden_s03`/`s04`/`s05`/`s12`); the tests here run their own
//! bodies, with the extra assertions C13.3 requires (the #216 late-publish
//! `RuntimeFailure` witness, the #384 assertion made before the guard
//! completes).
//!
//! The revert and mutation each test must fail under is recorded in the PR body
//! and, in short, on each test.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, started, stopped};
use super::*;

/// A named scenario body, with the terminal (outcome/code) its golden records.
type TerminalScenario = (&'static str, fn(&Arc<World>), &'static str);

/// A named scenario body.
type Scenario = (&'static str, fn(&Arc<World>));

const IDLE_EMIT: &str = "emit recording-status-changed \"idle\"";

fn status(world: &Arc<World>) -> DictationStatus {
    world.state().app_state.dictation.lock_or_recover().status
}

/// The payloads of every `recording-status-changed` emit in `trace`, in order.
fn status_emits(trace: &[String]) -> Vec<String> {
    const PREFIX: &str = "emit recording-status-changed ";
    trace
        .iter()
        .filter_map(|line| line.strip_prefix(PREFIX))
        .map(|payload| payload.trim_matches('"').to_string())
        .collect()
}

fn position(trace: &[String], line: &str) -> usize {
    trace
        .iter()
        .position(|entry| entry == line)
        .unwrap_or_else(|| panic!("`{line}` is missing from the trace:\n{}", trace.join("\n")))
}

fn sub(title: &'static str, body: fn(&Arc<World>)) -> SubRun {
    SubRun {
        title,
        setup: Setup::idle(),
        body,
    }
}

// ------------------------------------------------------------------ #216

/// #216 / PR #219 (`06296f79`): a hold-down stop reached `stop_native_recording`
/// before the start finished, and the late start emitted `recording` after
/// `processing`. The stop queues behind the start on the recording transition.
/// "Queued" is observed: the stop future is pending and none of it has run.
///
/// Oracle: the emitted status sequence is `starting, recording, processing`
/// (SP1, SP3: no `recording` after `processing`) and the stop's reply is the
/// completed-pipeline `transcription` reply with `state: idle` (the fake supplies
/// 16,000 samples, at least 4,800), not `not_recording`.
///
/// Revert / mutation: delete the stop command's `recording_transition.lock().await`
/// (`stop_native_recording_admitted`, the `let transition = ...` binding).
#[test]
fn bug216_stop_queued_behind_start() {
    for _ in 0..repeat_count() {
        let lines = run_sub_run(&sub("216 stop queued behind start", |w| {
            w.knobs().stop_result = Ok(16_000);
            w.close_gate("smart_capture");
            let mut start = w.start();
            assert!(poll_once(&mut start).is_pending());
            let mut stop = w.stop(None);
            assert!(
                poll_once(&mut stop).is_pending(),
                "the stop must queue behind the start on the recording transition"
            );
            assert!(
                !w.trace()
                    .iter()
                    .any(|line| line.starts_with("supervisor.stop")),
                "nothing of the stop has run while the start holds the transition"
            );
            assert_eq!(status(w), DictationStatus::Starting);
            w.open_gate("smart_capture");
            let reply = complete(&mut start);
            w.reply("start", &reply);
            let reply = complete(&mut stop);
            w.reply("stop", &reply);
            let reply = reply.expect("the stop completes the pipeline");
            assert_eq!(reply["type"], "transcription", "not `not_recording`");
            assert_eq!(reply["state"], "idle");
        }));
        let emits = status_emits(&lines);
        assert_eq!(
            emits.get(..3),
            Some(
                &[
                    "starting".to_string(),
                    "recording".into(),
                    "processing".into()
                ][..]
            ),
            "{emits:?}"
        );
        let processing = emits.iter().position(|s| s == "processing").unwrap();
        assert!(
            !emits[processing..].iter().any(|s| s == "recording"),
            "no `recording` after `processing`: {emits:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("not_recording")),
            "the stop must not reply not_recording"
        );
    }
}

/// #216, the publish half (`06296f79` did not touch the publish guard): a late
/// ready publish needs status `Starting` (R104.5).
///
/// The mutation witness pauses the start after `is_dictation_recording(rid) ==
/// true` is read and before D is taken (the fake supervisor's phase read), and
/// delivers `Recovering{RuntimeFailure}` for the current id. That arm writes
/// `Recovering` and **preserves** the installed context, so the `active_context`
/// check does not refuse first: only the `status == Starting` guard stands
/// between the publish and an erroneous Recovering→Recording write. The publish
/// must emit no `recording`, write no status and spawn no `SD`.
///
/// A supervisor `Idle` (or a non-`RuntimeFailure` `Recovering`) in the same
/// window clears the context, so those runs verify refusal but do not kill the
/// R104.5 mutation (C13.3); the second half of this test runs the `Idle` one for
/// that reason, and for completeness only.
///
/// Mutation: drop R104.5's `status == Starting` guard (`decide_ready_publish`).
#[test]
fn bug216_late_publish_requires_starting() {
    for _ in 0..repeat_count() {
        let lines = run_sub_run(&sub("216 late publish, RuntimeFailure", |w| {
            w.knobs().cold_start = ColdStart::ReadyBeforeContext;
            w.on("supervisor.is_dictation_recording", {
                let world = Arc::clone(w);
                move || world.runtime_failure(1)
            });
            let mut start = w.start();
            let reply = complete(&mut start);
            w.reply("start", &reply);
            assert!(reply.is_ok(), "the start flight ends with its own reply");
            assert_eq!(
                status(w),
                DictationStatus::Recovering,
                "Recovering is not overwritten with Recording"
            );
            assert!(
                w.state().app_state.active_context(1).is_some(),
                "the RuntimeFailure arm preserves the context: this run is a mutation witness"
            );
            assert_eq!(
                w.side_task_spawns(),
                0,
                "no side task starts for a publish that did not happen"
            );
        }));
        assert!(
            lines
                .iter()
                .any(|l| l == "publish_recording_ready 1 -> false"),
            "{lines:#?}"
        );
        assert!(
            !status_emits(&lines).iter().any(|s| s == "recording"),
            "no `recording` emit: {lines:#?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("-> Recording")),
            "no status write to Recording: {lines:#?}"
        );
    }
    // Refusal verified, not a mutation witness: `Idle` clears the context first.
    let lines = run_sub_run(&sub("216 late publish, Idle", |w| {
        w.knobs().cold_start = ColdStart::ReadyBeforeContext;
        w.on("supervisor.is_dictation_recording", {
            let world = Arc::clone(w);
            move || world.notify(1, AudioLifecycleEvent::Idle)
        });
        let mut start = w.start();
        let reply = complete(&mut start);
        w.reply("start", &reply);
        assert_eq!(w.side_task_spawns(), 0);
        assert_eq!(status(w), DictationStatus::Idle);
    }));
    assert!(
        !status_emits(&lines).iter().any(|s| s == "recording"),
        "no `recording` emit after a supervisor Idle: {lines:#?}"
    );
}

// ------------------------------------------------------------------ #384

/// #384 (`26ceb195`): status lingered at `Processing` after a 12 s timed-out
/// Stopping teardown. The recording is Processing, the stop times out with its
/// `IdleGuard` still live (the stop is parked inside the supervisor's stop call,
/// the causal barrier), the supervisor `Idle` for the current id is delivered, and
/// the assertions run **immediately after that event and before the guard
/// completes**: the later guard re-writes `Idle` on an id match (PD-6), so a
/// final-state assertion alone passes even with the fix reverted.
///
/// Intermediate oracle: status `Idle`; `keyboard::is_processing()` false; exactly
/// one `idle` emit, and it comes from the supervisor arm; no later `processing`.
/// After the guard: still `Idle` (a second `idle` emit is allowed, PD-6).
///
/// Revert / mutation: the bridge's `Idle` arm no longer forces `Idle` over
/// `Processing` (R211.3 a no-op; the historical one-line change is the added
/// `| DictationStatus::Processing` that `decide_sup_idle` now owns).
#[test]
fn bug384_idle_after_timed_out_stop() {
    for _ in 0..repeat_count() {
        let lines = run_sub_run(&sub("384 idle after a timed-out stop", |w| {
            recording(w);
            w.knobs().stop_result = Err("stop timed out");
            w.on("supervisor.stop", {
                let world = Arc::clone(w);
                move || {
                    assert_eq!(status(&world), DictationStatus::Processing);
                    world.notify(1, AudioLifecycleEvent::Idle);
                    // Before the stop's guard completes.
                    assert_eq!(
                        status(&world),
                        DictationStatus::Idle,
                        "the supervisor Idle must clear Processing"
                    );
                    assert!(
                        !keyboard::is_processing(),
                        "keyboard processing must be cleared by the supervisor Idle"
                    );
                    let trace = world.trace();
                    let idles = trace.iter().filter(|l| *l == IDLE_EMIT).count();
                    assert_eq!(
                        idles,
                        1,
                        "exactly one idle emit so far:\n{}",
                        trace.join("\n")
                    );
                    assert!(
                        position(&trace, IDLE_EMIT) > position(&trace, "supervisor.notify 1 Idle"),
                        "the idle emit is the supervisor arm's"
                    );
                    assert_eq!(
                        status_emits(&trace).last().map(String::as_str),
                        Some("idle"),
                        "no processing emit after the supervisor's idle"
                    );
                }
            });
            stopped(w, "stop", None);
            assert_eq!(status(w), DictationStatus::Idle);
            assert!(!keyboard::is_processing());
        }));
        let emits = status_emits(&lines);
        let first_idle = emits.iter().position(|s| s == "idle").unwrap();
        assert!(
            !emits[first_idle..].iter().any(|s| s == "processing"),
            "no processing emit after the first idle: {emits:?}"
        );
        assert!(
            emits.iter().filter(|s| *s == "idle").count() <= 2,
            "at most the supervisor's and the guard's idle (PD-6): {emits:?}"
        );
    }
}

// ------------------------------------------------------------------ #388

fn s12a(world: &Arc<World>) {
    world.knobs().start_ack = Some(AudioStartError::SpawnFailed("scripted".to_string()));
    started(world);
    world.notify(
        1,
        AudioLifecycleEvent::InitializationFailed {
            error: "scripted initialization failure".to_string(),
            kind: audio::AudioFailureKind::BackendError,
            recovery_reason: None,
        },
    );
    world.notify(1, AudioLifecycleEvent::Idle);
}

fn s12b(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SpawnFailed("scripted".to_string()));
        knobs.before_ack = vec![
            AudioLifecycleEventScript::InitializationFailed,
            AudioLifecycleEventScript::Idle,
        ];
    }
    started(world);
}

fn s12c(world: &Arc<World>) {
    {
        let mut knobs = world.knobs();
        knobs.start_ack = Some(AudioStartError::SupervisorUnavailable);
        knobs.before_ack = vec![AudioLifecycleEventScript::Recovering(
            AudioCancelReason::User,
        )];
    }
    started(world);
}

/// (d) A recording whose worker failed (`Recovering{RuntimeFailure}`) and then
/// receives the supervisor's `Idle` (R211.6): the status is Idle at once, with no
/// later write to rely on.
fn s12d(world: &Arc<World>) {
    recording(world);
    world.runtime_failure(1);
    assert_eq!(status(world), DictationStatus::Recovering);
    world.notify(1, AudioLifecycleEvent::Idle);
    assert_eq!(
        status(world),
        DictationStatus::Idle,
        "the supervisor Idle over Recovering must restore Idle"
    );
}

/// #388 / PR #392, the core-owned part (`66dd78d7`, supervisor-only): any
/// grammar-legal sequence over a start ends in `Idle`. Scenario 12(a)(b)(c)
/// plus (d): `InitializationFailed` then `Idle` after a failed ack, the same
/// before the ack, the 2 s start timeout over a supervisor `Cancel{User}`, and a
/// supervisor `Idle` over a `Recovering` attempt. After each,
/// status is `Idle` with the keyboard gate and the context cleared and exactly
/// one terminal claimed for the id.
///
/// No revert applies (the fix changes only `audio_lifecycle.rs`; C13.3, plan §2
/// criterion 2). Mutation: R211.4 and R211.6 and R102.3 made no-ops.
#[test]
fn bug388_init_failure_reaches_idle() {
    let bodies: [Scenario; 4] = [
        ("12a failure ack, then InitializationFailed and Idle", s12a),
        (
            "12b InitializationFailed and Idle, then the failure ack",
            s12b,
        ),
        ("12c 2 s start timeout with a user cancel", s12c),
        ("12d Idle over a Recovering attempt", s12d),
    ];
    for _ in 0..repeat_count() {
        for (title, body) in bodies {
            let lines = run_sub_run(&sub(title, body));
            let last = lines.last().expect("a final line");
            assert_eq!(
                last,
                "= final status=Idle processing=false context=none cancelled_through=0",
                "{title}: the attempt must end Idle\n{}",
                lines.join("\n")
            );
            let terminals = lines
                .iter()
                .filter(|l| l.starts_with("~ terminal 1 "))
                .count();
            assert_eq!(
                terminals,
                1,
                "{title}: exactly one terminal\n{}",
                lines.join("\n")
            );
            if title.starts_with("12b") {
                // The supervisor's `Idle` over `Starting` (R211.4) restores Idle by
                // itself, before the failure ack: the later failure path writes Idle
                // regardless, so only this ordering shows R211.4 working.
                assert!(
                    position(&lines, "~ status Starting -> Idle")
                        < lines
                            .iter()
                            .position(|l| l.starts_with("supervisor.start_ack 1 Err("))
                            .expect("the failed ack"),
                    "{title}: R211.4 must write Idle before the ack\n{}",
                    lines.join("\n")
                );
            }
            if title.starts_with("12d") {
                assert!(
                    lines.iter().any(|l| l == "~ status Recovering -> Idle"),
                    "{title}: R211.6 must write Idle over Recovering\n{}",
                    lines.join("\n")
                );
            }
            assert!(
                !lines.iter().any(|l| l.starts_with("~ status")
                    && l.ends_with("-> Recording")
                    && !title.starts_with("12d")),
                "{title}: the attempt never reaches Recording"
            );
        }
    }
}

// ------------------------------------------------------------------ #535

/// C7.4 `M(T)` at class level: which performance classes a terminal's outcome
/// is compatible with. `None`: no check (`Superseded`, PD-4 and PD-5 are exempt).
fn performance_classes(terminal: &str) -> Option<&'static [&'static str]> {
    Some(match terminal {
        "Success" => &["success"],
        "NoSpeech" | "TooShort" => &["noSpeech"],
        "UserCancelledStarting" | "UserCancelledRecording" | "UserCancelledProcessing" => {
            &["cancelled"]
        }
        "CaptureInitFailure" | "StopFailure" | "PipelineFailure" => &["failed"],
        "RuntimeInterruption" => &["failed", "success", "noSpeech", "cancelled", "interrupted"],
        _ => return None,
    })
}

fn interrupted_recording(world: &Arc<World>, samples: usize) {
    recording(world);
    world.runtime_failure(1);
    world.finish_interrupted(1, samples);
}

fn s03a(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    for recording_id in world.auto_stop_requests() {
        stopped(world, "auto-stop", Some(recording_id));
    }
    world.release_owner_clear();
}

fn s03b(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    world.close_gate("pipeline");
    let mut user = world.stop(None);
    assert!(poll_once(&mut user).is_pending());
    for recording_id in world.auto_stop_requests() {
        stopped(world, "auto-stop", Some(recording_id));
    }
    world.open_gate("pipeline");
    let reply = complete(&mut user);
    world.reply("user stop", &reply);
    world.release_owner_clear();
}

fn s04a(world: &Arc<World>) {
    interrupted_recording(world, 6_000);
    world.release_owner_clear();
}

/// 3c: the auto-stop takes the capture first; the user stop then finds Processing.
fn s03c(world: &Arc<World>) {
    interrupted_recording(world, 9_000);
    world.close_gate("pipeline");
    let requests = world.auto_stop_requests();
    let mut auto = world.stop(Some(requests[0]));
    assert!(poll_once(&mut auto).is_pending());
    world.note("auto-stop took the capture and is in the pipeline");
    stopped(world, "user stop", None);
    world.open_gate("pipeline");
    let reply = complete(&mut auto);
    world.reply("auto-stop", &reply);
    world.release_owner_clear();
}

/// 4b (PD-1): a user stop runs after the bridge read "current" and before its Idle
/// write; the hook runs at the bridge's `recording-interrupted` emit.
fn s04b(world: &Arc<World>) {
    type StopSlot = Arc<std::sync::Mutex<Option<Fut<Result<serde_json::Value, String>>>>>;
    recording(world);
    world.runtime_failure(1);
    world.close_gate("pipeline");
    let slot: StopSlot = Arc::new(std::sync::Mutex::new(None));
    let (between, parked) = (Arc::clone(world), Arc::clone(&slot));
    world.on("emit:recording-interrupted", move || {
        let mut stop = between.stop(None);
        assert!(poll_once(&mut stop).is_pending());
        *parked.lock().expect("slot") = Some(stop);
    });
    world.finish_interrupted(1, 6_000);
    world.note("the bridge has returned; the user stop is still in the pipeline");
    world.open_gate("pipeline");
    let mut stop = slot.lock().expect("slot").take().expect("a parked stop");
    let reply = complete(&mut stop);
    world.reply("user stop", &reply);
    world.release_owner_clear();
}

fn s05(world: &Arc<World>) {
    recording(world);
    world.knobs().stop_result = Err("stop timed out");
    let idle = Arc::clone(world);
    world.on("supervisor.stop", move || {
        idle.notify(1, AudioLifecycleEvent::Idle)
    });
    stopped(world, "stop", None);
}

/// 4d (PD-18 a): a start takes the dictation lock after S12 wrote Idle and before its
/// tail, and again after the owner is cleared; id 1 still ends with one terminal.
fn s04d(world: &Arc<World>) {
    recording(world);
    world.runtime_failure(1);
    let between = Arc::clone(world);
    world.on_s12_tail(move || {
        between.note("S12 wrote Idle and released D; its tail has not run; a start takes D");
        let mut start = between.start();
        let reply = complete(&mut start);
        between.reply("start", &reply);
    });
    world.finish_interrupted(1, 6_000);
    world.note("the bridge returned; the supervisor is held before clearing its owner");
    world.release_owner_clear();
    world.note("owner cleared; the supervisor has nothing more to notify");
    let mut again = world.start();
    let reply = complete(&mut again);
    world.reply("start again", &reply);
}

/// #535 / PR #563 (`ddf2b6a1`, `80747cd4`): every dictation attempt reaches a
/// terminal exactly once and correlatably. Runs scenarios 3 (a, b, c), 4 (a, b, d), 5
/// and 12 (a, b, c) and asserts the **terminal claims**, not the ledger: a test-only
/// observer in `DictationTelemetry::claim_terminal` records every *successful*
/// claim per id (it adds no trace line, so no golden changes), and each scenario
/// must have exactly one for id 1, with the exact outcome and code its golden
/// records (D-S2b-1). The same-outcome case the ledger cannot see is forced after
/// each scenario: the replay re-claims id 1 with the first claim's own outcome and
/// code and asserts the claim is refused and the count stays one. The ledger lines
/// (`~ terminal 1`) are still checked, and where a performance class was claimed it
/// is in `M(T)` (C7.4; `Superseded`, PD-4 and PD-5 are exempt and none of these
/// scenarios produces one).
///
/// Revert: remove the once-only claim in `DictationTelemetry::claim_terminal`
/// (`if attempt.terminal.is_some() { return false; }`): the forced re-claim wins and
/// the count is two, with the same outcome and code. Mutation: drop `TERM` from
/// R102.2 or make R211.3's `TERM` conditional.
#[test]
fn bug535_exactly_one_terminal() {
    // (title, body, the terminal the golden trace records for id 1: outcome/code)
    let scenarios: [TerminalScenario; 10] = [
        (
            "3a interruption, auto-stop transcribes",
            s03a,
            "RuntimeInterruption/StreamInvalidated",
        ),
        (
            "3b interruption, the user stop wins",
            s03b,
            "RuntimeInterruption/StreamInvalidated",
        ),
        (
            "3c interruption, the auto-stop wins",
            s03c,
            "RuntimeInterruption/StreamInvalidated",
        ),
        (
            "4a short interruption",
            s04a,
            "RuntimeInterruption/RuntimeFailure",
        ),
        (
            "4b short interruption, a user stop between the check and the idle write",
            s04b,
            "RuntimeInterruption/RuntimeFailure",
        ),
        (
            "4d start before the owner is cleared",
            s04d,
            "RuntimeInterruption/RuntimeFailure",
        ),
        (
            "5 timed-out stop, supervisor Idle",
            s05,
            "PipelineFailure/RuntimeFailure",
        ),
        (
            "12a failure ack first",
            s12a,
            "CaptureInitFailure/RuntimeFailure",
        ),
        (
            "12b supervisor failure first",
            s12b,
            "CaptureInitFailure/BackendError",
        ),
        (
            "12c start timeout",
            s12c,
            "UserCancelledStarting/CancelledStarting",
        ),
    ];
    for _ in 0..repeat_count() {
        for (title, body, expected) in scenarios {
            // Every scenario accepts id 1.
            let id = 1;
            let claims = std::cell::RefCell::new(Vec::new());
            let refused_again = std::cell::Cell::new(None);
            let lines = run_sub_run_observed(&sub(title, body), &|w| {
                let telemetry = &w.state().app_state.dictation_telemetry;
                let successful = telemetry.successful_claims_for_test(id);
                // Force a second claim with the first claim's own outcome and code.
                if let Some(&(outcome, code)) = successful.first() {
                    refused_again.set(Some(!telemetry.emit_terminal(id, outcome, code)));
                }
                *claims.borrow_mut() = telemetry
                    .successful_claims_for_test(id)
                    .into_iter()
                    .map(|(outcome, code)| format!("{outcome:?}/{code:?}"))
                    .collect::<Vec<_>>();
            });
            let claims = claims.into_inner();
            assert_eq!(
                claims,
                [expected],
                "{title}: id {id} must have exactly one successful terminal claim, with the golden's outcome/code\n{}",
                lines.join("\n")
            );
            assert_eq!(
                refused_again.get(),
                Some(true),
                "{title}: a second claim of id {id} with the same outcome and code must be refused"
            );
            // The ledger agrees: one `~ terminal` line, the same terminal.
            let outcomes: Vec<&str> = lines
                .iter()
                .filter_map(|l| l.strip_prefix(&format!("~ terminal {id} ")))
                .collect();
            assert_eq!(
                outcomes,
                [expected],
                "{title}: id {id} ledger\n{}",
                lines.join("\n")
            );
            // M(T): the performance class the winning terminal is compatible with.
            let terminal = outcomes[0].split('/').next().unwrap_or_default();
            let perf = lines
                .iter()
                .find_map(|l| l.strip_prefix(&format!("~ perf-claim {id} ")));
            if let (Some(classes), Some(perf)) = (performance_classes(terminal), perf) {
                let class = serde_json::from_str::<serde_json::Value>(perf)
                    .expect("perf claim is JSON")["status"]
                    .as_str()
                    .expect("a status")
                    .to_string();
                assert!(
                    classes.contains(&class.as_str()),
                    "{title}: id {id}: terminal {terminal} with performance class {class} is not in M(T)"
                );
            }
        }
    }
}

// ------------------------------------------------------------------ #117

/// The production items of `commands/models.rs` as token text: parsed with `syn`
/// and every top-level item carrying a `#[cfg(test)]`-style attribute dropped
/// (the file has a test module in the middle and production code after it).
fn models_rs_production_source() -> String {
    use quote::ToTokens;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/models.rs");
    let text = std::fs::read_to_string(&path).expect("read models.rs");
    let file = syn::parse_file(&text).expect("parse models.rs");
    let is_test_cfg = |attr: &syn::Attribute| {
        attr.path().is_ident("cfg") && attr.to_token_stream().to_string().contains("test")
    };
    let items: Vec<String> = file
        .items
        .iter()
        .filter(|item| {
            let attrs: &[syn::Attribute] = match item {
                syn::Item::Fn(item) => &item.attrs,
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Const(item) => &item.attrs,
                syn::Item::Static(item) => &item.attrs,
                syn::Item::Struct(item) => &item.attrs,
                syn::Item::Enum(item) => &item.attrs,
                syn::Item::Use(item) => &item.attrs,
                _ => &[],
            };
            !attrs.iter().any(is_test_cfg)
        })
        .map(|item| item.to_token_stream().to_string())
        .collect();
    assert!(
        items.iter().any(|item| item.contains("ensure_vad_model")),
        "the scan must reach ensure_vad_model"
    );
    items.join("\n")
}

/// SP5, source half, for the VAD download path: no `recording-status-changed`
/// emit in `commands/models.rs`. `ensure_vad_model` is called as a fire-and-forget
/// background task from the pipeline; a status emit there stomps `Idle`.
#[test]
fn sp5_models_rs_emits_no_recording_status() {
    let source = models_rs_production_source();
    assert!(
        !source.contains("recording-status-changed"),
        "commands/models.rs must not emit recording-status-changed (#117)"
    );
    assert!(
        !source.contains("downloading-vad"),
        "commands/models.rs must not emit downloading-vad (#117)"
    );
}

/// The real `ensure_vad_model` on a worker thread (the pipeline starts it as a
/// fire-and-forget task), against a fake download (`vad::test_seam`) parked until
/// released. The worker has an explicit cancellation path and every join is bounded
/// (D-S2b-10 g): a worker that cannot make progress (here: one waiting on the install
/// lock another installer holds) is cancelled, and a worker that still does not stop
/// fails the test instead of hanging it. Dropping releases the download, cancels and
/// joins within the bound, so a failing assertion never leaves a thread behind.
struct VadInstall {
    seam: crate::vad::test_seam::Installed,
    cancel: Arc<tokio::sync::Notify>,
    worker: Option<std::thread::JoinHandle<Result<(), String>>>,
}

/// The longest a replay waits for the install to park before it fails.
const VAD_PARK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The longest the worker may take to stop after the download is released.
const VAD_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The longest the worker may take to stop after it is cancelled.
const VAD_CANCEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The error a cancelled install returns.
const VAD_INSTALL_CANCELLED: &str = "the VAD install was cancelled";

impl VadInstall {
    fn start(world: &Arc<World>, models_dir: &std::path::Path) -> Self {
        let seam = crate::vad::test_seam::install(models_dir);
        let handle = world.app_handle();
        let cancel = Arc::new(tokio::sync::Notify::new());
        let worker_cancel = Arc::clone(&cancel);
        let worker = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(async {
                    // Every wait inside the install is async, so dropping its future
                    // (the lock wait, the parked download) is a real cancellation.
                    tokio::select! {
                        result = super::super::models::ensure_vad_model(&handle) => result,
                        () = worker_cancel.notified() => Err(VAD_INSTALL_CANCELLED.to_string()),
                    }
                })
        });
        Self {
            seam,
            cancel,
            worker: Some(worker),
        }
    }

    /// `Ok` when the install parked in its download; `Err` when it finished first (its
    /// result) or did not park in time.
    fn wait_parked(&mut self) -> Result<(), String> {
        self.wait_parked_within(VAD_PARK_TIMEOUT)
    }

    fn wait_parked_within(&mut self, timeout: std::time::Duration) -> Result<(), String> {
        use crate::vad::test_seam::Parked;
        let worker = self.worker.as_ref().expect("the worker");
        match self.seam.wait_parked(|| worker.is_finished(), timeout) {
            Parked::Arrived => Ok(()),
            Parked::WorkerFinished => Err(format!(
                "the VAD install finished before it parked: {:?}",
                self.finish()
            )),
            Parked::TimedOut => Err("the VAD install never parked in its download".to_string()),
        }
    }

    /// Wait up to `timeout` for the worker, then collect its result; `None` when it
    /// is still running.
    fn join_within(&mut self, timeout: std::time::Duration) -> Option<Result<(), String>> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
            {
                return Some(match self.worker.take() {
                    Some(worker) => worker.join().expect("the VAD install thread"),
                    None => Err("the VAD install was already collected".to_string()),
                });
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Cancel the worker and wait for it within `VAD_CANCEL_TIMEOUT`.
    fn cancel(&mut self) -> Result<Result<(), String>, String> {
        self.cancel.notify_one();
        self.join_within(VAD_CANCEL_TIMEOUT).ok_or_else(|| {
            format!(
                "the VAD install did not stop within {VAD_CANCEL_TIMEOUT:?} of its cancellation"
            )
        })
    }

    /// Release the download and return the install's result. A worker that does not
    /// finish within `VAD_JOIN_TIMEOUT` is cancelled; one that still does not stop
    /// fails the test.
    fn finish(&mut self) -> Result<(), String> {
        self.seam.release();
        match self.join_within(VAD_JOIN_TIMEOUT) {
            Some(result) => result,
            None => match self.cancel() {
                Ok(_) => panic!(
                    "the VAD install did not finish within {VAD_JOIN_TIMEOUT:?} of its release \
                     (it was cancelled)"
                ),
                Err(error) => panic!("{error}"),
            },
        }
    }
}

impl Drop for VadInstall {
    fn drop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        self.seam.release();
        if let Err(error) = self.cancel() {
            // The worker is detached (its handle is dropped, never joined): the test
            // fails, it does not hang.
            self.worker = None;
            if std::thread::panicking() {
                eprintln!("{error}");
            } else {
                panic!("{error}");
            }
        }
    }
}

/// A regular file standing where the models directory's parent should be.
fn bug117_blocker_file() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("murmur-bug117-blocker-{}", std::process::id()))
}

/// The #117 replay must not hang when the install fails before it parks (a models
/// directory that cannot be created: here its parent is a regular file, as with
/// `TMPDIR` pointing at a file). The wait reports the early completion, promptly and
/// with the install's own error.
#[test]
fn bug117_a_failed_install_returns_promptly() {
    let blocker = bug117_blocker_file();
    std::fs::write(&blocker, b"not a directory").expect("write the blocker file");
    let started = std::time::Instant::now();
    let reports = std::sync::Mutex::new(None);
    run_sub_run_observed(&sub("117 install fails before parking", |_| {}), &|w| {
        let mut install = VadInstall::start(w, &bug117_blocker_file().join("models"));
        *reports.lock().unwrap() = Some(install.wait_parked());
    });
    let _ = std::fs::remove_file(&blocker);
    let report = reports.lock().unwrap().take().expect("the observer ran");
    let error = report.expect_err("an install that cannot create its directory never parks");
    assert!(
        error.contains("finished before it parked")
            && error.contains("Failed to create models directory"),
        "{error}"
    );
    assert!(
        started.elapsed() < VAD_PARK_TIMEOUT / 2,
        "the failure path returned in {:?}, not by timeout",
        started.elapsed()
    );
}

/// D-S2b-10 (g): a replay bounds the installer's shutdown as well as its arrival. The
/// installer here waits on the install lock another installer holds, so it never parks
/// (the shape that hung the replay: the wait timed out, then `Drop` joined the blocked
/// worker for good). The wait reports it, the cancellation stops the worker promptly
/// with the cancellation error, and dropping the handle returns instead of hanging.
#[test]
fn bug117_a_blocked_installer_is_cancelled_not_awaited() {
    let lock = super::super::models::hold_vad_install_lock_for_test();
    let started = std::time::Instant::now();
    let reports = std::sync::Mutex::new(None);
    run_sub_run_observed(
        &sub("117 installer blocked on the install lock", |_| {}),
        &|w| {
            let mut install = VadInstall::start(w, &bug117_models_dir());
            let parked = install.wait_parked_within(std::time::Duration::from_millis(300));
            let cancelled = install.cancel();
            *reports.lock().unwrap() = Some((parked, cancelled));
            // Dropped here with its worker already collected: nothing left to join.
        },
    );
    drop(lock);
    let (parked, cancelled) = reports.lock().unwrap().take().expect("the observer ran");
    assert_eq!(
        parked,
        Err("the VAD install never parked in its download".to_string())
    );
    assert_eq!(
        cancelled,
        Ok(Err(VAD_INSTALL_CANCELLED.to_string())),
        "the cancellation stops an installer that is blocked on the lock"
    );
    assert!(
        started.elapsed() < VAD_CANCEL_TIMEOUT,
        "shutdown was bounded, took {:?}",
        started.elapsed()
    );
}

/// The scratch models directory of the #117 replay (runs are serialized).
fn bug117_models_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("murmur-bug117-{}", std::process::id()))
}

/// #117 (`25677eca`): the first-run VAD download re-emitted `processing` after
/// `IdleGuard` had written Idle and left the app stuck. The pipeline starts
/// `ensure_vad_model` as a fire-and-forget task when the model is missing; here the
/// real `ensure_vad_model` runs against the world's app handle with a fake download
/// (`vad::test_seam`, test builds only) parked in `stream_download`. The stop
/// completes, `IdleGuard` writes Idle, the supervisor finishes the sealed stop, and
/// only then is the download released: the real completion path (validate, rename,
/// cleanup) runs after the final idle. Nothing may emit a status after that idle, and
/// the final status is Idle.
///
/// Revert: `git revert 25677eca` (the `downloading-vad` and `processing` emits in
/// `ensure_vad_model`; they sat in the completion path). Mutation: any status emit in
/// the completion path, however spelled (`concat!("recording-status-", "changed")`).
#[test]
fn bug117_no_status_emit_after_final_idle() {
    let source = models_rs_production_source();
    assert!(
        !source.contains("recording-status-changed"),
        "commands/models.rs must not emit recording-status-changed (#117 revert)"
    );
    for _ in 0..repeat_count() {
        let _ = std::fs::remove_dir_all(bug117_models_dir());
        let lines = run_sub_run(&sub("117 no status emit after the final idle", |w| {
            recording(w);
            // The pipeline's fire-and-forget VAD install, on the world's handle.
            let mut install = VadInstall::start(w, &bug117_models_dir());
            install
                .wait_parked()
                .expect("the install parks in its download");
            w.note("the VAD install is parked in its download");
            w.knobs().stop_result = Ok(16_000);
            stopped(w, "stop", None);
            assert_eq!(status(w), DictationStatus::Idle);
            w.finish_sealed_stop(1);
            w.note("IdleGuard wrote Idle and the supervisor finished the stop: the download completes now");
            let installed = install.finish();
            assert_eq!(installed, Ok(()), "the real completion path succeeds");
            assert!(
                crate::vad::vad_model_exists(),
                "the completion path installed the model"
            );
            w.note("the VAD install completed");
            assert_eq!(status(w), DictationStatus::Idle);
        }));
        let _ = std::fs::remove_dir_all(bug117_models_dir());
        let completed = position(&lines, "# the VAD install completed");
        let last_idle = lines
            .iter()
            .rposition(|l| l == IDLE_EMIT)
            .expect("the run ends with an idle emit");
        assert!(
            last_idle < completed,
            "the download completes after the final idle:\n{}",
            lines.join("\n")
        );
        let later: Vec<&String> = lines[last_idle + 1..]
            .iter()
            .filter(|l| l.starts_with("emit recording-status-changed "))
            .collect();
        assert!(
            later.is_empty(),
            "no recording-status-changed emit after the final idle: {later:?}"
        );
        assert_eq!(
            status_emits(&lines),
            ["starting", "recording", "processing", "idle", "idle"],
            "the legal sequence: the stop's idle, then the cleanup's"
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("= final status=Idle processing=false context=none cancelled_through=0")
        );
    }
}

// ----------------------------------------- 0b798457, bb66238b, 3c44d9e4

/// `0b798457`: a queued preparation ran while a preview was connecting. The
/// admission (`dictation_prepare_admitted`, i.e. `admit(DictationPrepare)`) refuses
/// while a preview is active, and the queued preparation behind the transition is
/// not admitted. (The commit's own `connecting_preview_blocks_queued_preparation_
/// before_supervisor_startup` stays in `recording.rs`; this is its C13.3 name.)
///
/// Revert: remove the `preview_active` check from the prepare gate (historically
/// `&& !...microphone_preview.is_active()` in `dictation_prepare_admitted`).
/// Mutation: delete `preview_active` from the `DictationPrepare` order.
#[tokio::test]
async fn bug_prep_admission_preview() {
    let state = Arc::new(State::for_test());
    let transition = state.app_state.recording_transition.lock().await;
    assert!(dictation_prepare_admitted(&state));
    let preview = state.app_state.microphone_preview.claim(0).unwrap();
    assert!(state.app_state.microphone_preview.is_connecting(preview));
    assert!(!audio_lifecycle::is_audio_active());
    assert_eq!(dictation_prepare_refusal(&state), Some("preview_active"));
    let queued_state = Arc::clone(&state);
    let queued = tokio::spawn(async move {
        crate::audio::prepare::admission_transition(
            &queued_state.app_state.recording_transition,
            || dictation_prepare_admitted(&queued_state),
        )
        .await
        .is_some()
    });
    drop(transition);
    assert!(
        !queued.await.unwrap(),
        "the queued preparation is not admitted"
    );
}

/// `bb66238b`, the dictation half: a committed permission relaunch refuses a
/// dictation start with `busy_restarting`. (The meeting half, the post-claim
/// `relaunch_committed` re-read that undoes the claim, is
/// `meeting::admission_tests::bug_relaunch_vs_meeting_claim`: `claim_meeting_ownership`
/// is private to `meeting.rs`.)
///
/// Revert: remove the committed check from `dictation_start_blocker`.
/// Mutation: remove R101.3's `relaunch_committed` check.
#[test]
fn bug_relaunch_vs_dictation_start() {
    let state = State::for_test();
    assert_eq!(dictation_start_blocker(&state, DictationStatus::Idle), None);
    let _committed = crate::commands::permissions::CommittedRelaunchForTest::raise();
    assert_eq!(
        dictation_start_blocker(&state, DictationStatus::Idle),
        Some(DictationStartBlocker {
            kind: "busy_restarting",
            state: "idle"
        })
    );
}

/// `3c44d9e4` (PR #1128): an open Settings mic test delayed dictation and refused
/// the Fn key-down. A key-down over an open test takes the preview over, and the
/// start hands the kept preview to the supervisor: the fake supervisor records
/// `start_after_preview` and **zero** cold device opens (`supervisor.start`).
///
/// Revert: make `key_down_admission_from`'s `preview_active` arm return
/// `Err("preview_active")` and restore `transition_after_stopping_preview` in
/// `start_native_recording`. Mutation: KeyDownCapture x preview-with-candidate K to
/// R, DictationStart x preview K to stop-and-retry.
#[test]
fn bug_preview_handoff() {
    // Admission: the key-down takes over a preview that is `Active` (a handoff
    // candidate) and still refuses one that is only connecting.
    let state = State::for_test();
    assert_eq!(key_down_capture_admission(&state), Ok(None));
    let id = state.app_state.microphone_preview.claim(50).unwrap();
    assert_eq!(key_down_capture_admission(&state), Err("preview_active"));
    state
        .app_state
        .microphone_preview
        .set_phase_if(id, crate::microphone_preview::PreviewPhase::Active);
    assert_eq!(key_down_capture_admission(&state), Ok(Some(id)));
    // The start hands the kept preview over.
    for _ in 0..repeat_count() {
        let lines = run_sub_run(&sub("8 kept preview handed to the start", |w| {
            w.state()
                .app_state
                .microphone_preview
                .claim(50)
                .expect("a mic test is kept");
            let mut start = w.start();
            let reply = complete_joining(&mut start);
            w.reply("start", &reply);
            assert!(reply.is_ok());
        }));
        assert!(
            lines
                .iter()
                .any(|l| l == "supervisor.start_after_preview 1 toggle"),
            "the start must carry the kept preview to the supervisor:\n{}",
            lines.join("\n")
        );
        assert!(
            !lines.iter().any(|l| l.starts_with("supervisor.start 1 ")),
            "zero cold device opens:\n{}",
            lines.join("\n")
        );
    }
}
