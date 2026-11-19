//! The scheduler: one deterministic thread of control that sequences the actors
//! (T-holder flights, D-section steps, the supervisor thread, its bridge arms,
//! pipelines, tickers) one atomic step at a time, and the run loop that checks
//! every prefix and the end of the run.
//!
//! Two scheduling rules from the README (Stage 2b): a dictation future is never
//! polled inline inside a span and never dropped inside one (a step is the whole
//! span; a start can only be dropped *between* steps, at its await), and a future
//! is polled once per step and the self-wake is not followed (a blocked step
//! changes nothing and the scheduler moves on to another actor).

use super::checks::{Checker, RunHits};
use super::flights::{CancelFlight, ImportFlight, StartFlight, StopFlight};
use super::gen::{self, Bias, Cmd, StartCaller, StopCaller};
use super::model::*;
use super::rng::Rng;
use super::sup_task::{SupTask, WorkerTask};
use super::task::*;
use crate::status::Status;

const MAX_STEPS: u32 = 6_000;
const REROLL_EVERY: u32 = 12;

struct Slot {
    tid: TaskId,
    task: Box<dyn Task>,
    daemon: bool,
}

fn weight(bias: &Bias, name: &str) -> u64 {
    match name {
        "supervisor" | "worker" => bias.supervisor,
        "bridge" => bias.bridge,
        "pipeline" => bias.pipeline,
        "ticker" => bias.ticker,
        _ => bias.clients,
    }
}

/// Fields are read through `Debug` when a seed fails.
#[allow(dead_code)]
#[derive(Debug)]
pub struct RunReport {
    pub seed: u64,
    pub violations: Vec<String>,
    pub hits: RunHits,
    pub steps: u32,
}

pub fn run_seed(seed: u64) -> RunReport {
    run(seed, false, false).0
}

pub fn run_seed_with(seed: u64, trace: bool) -> (RunReport, Vec<String>) {
    run(seed, trace, false)
}

/// Like [`run_seed`], and every blocked poll must leave the whole world as it was.
pub fn run_seed_strict(seed: u64) -> RunReport {
    run(seed, false, true).0
}

/// Something is waiting for a window: a queued status event it has not received, or a start
/// reply the main window has not processed.
fn window_ready(w: &World) -> bool {
    !w.start_replies.is_empty() || w.windows.iter().any(|window| !window.inbox.is_empty())
}

fn window_name(index: usize) -> &'static str {
    if index == MAIN_WINDOW {
        "main"
    } else {
        "overlay"
    }
}

/// What the scheduler boundary a delivery ran at was about to issue, as the trace names it:
/// a due stop is named with its caller, so a test can tell whose stop the delivery preceded.
fn due_label(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Stop {
            caller: StopCaller::Window(index),
            ..
        } => format!("stop from the {} window", window_name(*index)),
        Cmd::Stop {
            caller: StopCaller::Ipc,
            ..
        } => "stop from the IPC".to_string(),
        Cmd::Start { .. } => "start".to_string(),
        Cmd::Import(_) => "import".to_string(),
        Cmd::Cancel { .. } => "cancel".to_string(),
    }
}

/// What a delivery changed that the run follows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivered {
    Other,
    /// The main window received `Idle`: its guard and its local status are gone.
    MainIdle,
    /// The main window's start reply put `Recording` over the `Processing` of a stop in flight.
    ReplyOverProcessing,
}

/// One ready delivery happens: a window receives its next status event, or the main window
/// processes its next start reply (#1162, a step of its own like a status delivery). A delivery
/// is a step of the schedule that changes no shell state: no model time, no actor weight.
/// `before_command` names the due command it ran ahead of, when it ran at a due-command
/// boundary.
fn deliver_next(
    w: &mut World,
    rng: &mut Rng,
    checker: &mut Checker,
    clock: u32,
    before_command: Option<String>,
) -> Delivered {
    let mut delivered = Delivered::Other;
    // The status deliveries first (window order), the start reply last, so a world with no
    // pending reply draws exactly what it drew before the reply existed.
    let mut ready: Vec<Option<usize>> = (0..w.windows.len())
        .filter(|window| !w.windows[*window].inbox.is_empty())
        .map(Some)
        .collect();
    if !w.start_replies.is_empty() {
        ready.push(None);
    }
    let before_command_text = match &before_command {
        Some(label) => format!(" (before the due {label})"),
        None => String::new(),
    };
    match ready[rng.index(ready.len())] {
        Some(window) => {
            let status = w.deliver_status(window);
            if window == MAIN_WINDOW && status == Status::Idle {
                delivered = Delivered::MainIdle;
            }
            if let Some(trace) = checker.trace.as_mut() {
                trace.push(format!(
                    "{clock:>4} {} window receives {status:?}{before_command_text}",
                    window_name(window)
                ));
            }
        }
        None => {
            let before = w.windows[MAIN_WINDOW].observed;
            let stop_in_flight = w.windows[MAIN_WINDOW].stop_in_flight;
            let (reply, write) = w.deliver_start_reply().expect("a start reply was pending");
            let after = w.windows[MAIN_WINDOW].observed;
            // Read off the window, not off the write the model reports: a reply that put
            // `Recording` over the `Processing` of a stop still in flight (#1162).
            if before == Status::Processing && after == Status::Recording && stop_in_flight {
                checker.hits.reply_over_processing += 1;
                delivered = Delivered::ReplyOverProcessing;
            }
            if write == ReplyWrite::EndedGeneration {
                checker.hits.reply_ended_generation += 1;
            }
            if let Some(trace) = checker.trace.as_mut() {
                let (ty, state, generation) = match reply.response {
                    StartResponse::Typed {
                        ty,
                        state,
                        recording_id,
                    } => (
                        ty.as_str().to_string(),
                        format!("{state:?}"),
                        recording_id.map_or(String::new(), |id| format!(", recording {id}")),
                    ),
                    StartResponse::Error => {
                        ("error".to_string(), "no state".to_string(), String::new())
                    }
                };
                trace.push(format!(
                    "{clock:>4} main window receives the reply of its start t{} ({ty}, state {state}{generation}): {}; the window shows {before:?} -> {after:?}{before_command_text}",
                    reply.tid,
                    match write {
                        ReplyWrite::Wrote(status) => format!("writes {status:?}"),
                        ReplyWrite::NotOwner => "not the owner of the state".to_string(),
                        ReplyWrite::StaleStarting => "stale starting, ignored".to_string(),
                        ReplyWrite::EndedGeneration =>
                            "starting for a generation that already ended, ignored".to_string(),
                        ReplyWrite::NoState => "no state to write".to_string(),
                    }
                ));
            }
        }
    }
    delivered
}

fn run(seed: u64, trace: bool, strict: bool) -> (RunReport, Vec<String>) {
    let mut rng = Rng::new(seed);
    let mut scenario_rng = rng.fork();
    // A second stream, apart from the scheduler's and the scenario's: what it draws (the
    // microphone of a start, the legacy imports) moves none of their draws.
    let mut extra_rng = Rng::new(seed ^ 0x1A7E_5EED_0F00_BA11);
    let scenario = gen::scenario(&mut scenario_rng, &mut extra_rng);
    let mut w = World::new();
    let mut out = StepOut::default();
    let mut checker = Checker::default();
    if trace {
        checker.trace = Some(Vec::new());
    }
    let mut mailbox: Vec<(TaskId, PipelineReply)> = Vec::new();
    let mut slots: Vec<Slot> = vec![
        Slot {
            tid: 1,
            task: Box::new(SupTask),
            daemon: true,
        },
        Slot {
            tid: 2,
            task: Box::new(WorkerTask),
            daemon: true,
        },
    ];
    let mut next_tid: TaskId = 3;
    let mut pending: std::collections::VecDeque<(u32, Cmd)> = scenario.cmds.into();
    let mut bias = Bias::roll(&mut rng);
    let mut clock = 0u32;
    let mut steps = 0u32;
    // A start reply put `Recording` over a stop's `Processing`, and the main window has received
    // no `Idle` since: its next stop gesture is the second one the reply made possible (#1162).
    let mut late_reply = false;

    loop {
        if steps >= MAX_STEPS {
            checker
                .violations
                .push(format!("liveness: no quiescence after {MAX_STEPS} steps"));
            break;
        }
        if steps.is_multiple_of(REROLL_EVERY) {
            bias = Bias::roll(&mut rng);
        }
        // Issue the commands that are due.
        while pending.front().is_some_and(|(at, _)| *at <= clock) {
            // A due command and a ready delivery compete at this boundary (#1159, D-S2b-19
            // (c)): production can deliver a status to a window and only then send that
            // window's stop, or send the stop first. The RNG chooses (one draw, only when both
            // are ready, so a run with no ready delivery draws nothing new). The delivery
            // takes no model time and carries no actor weight; the command stays due and
            // competes again, so the loop ends: each delivery consumes a queued event and
            // only actors add events.
            if window_ready(&w) && rng.chance(1, 2) {
                let label = due_label(&pending.front().expect("checked above").1);
                match deliver_next(&mut w, &mut rng, &mut checker, clock, Some(label)) {
                    Delivered::ReplyOverProcessing => late_reply = true,
                    Delivered::MainIdle => late_reply = false,
                    Delivered::Other => {}
                }
                continue;
            }
            let (_, cmd) = pending.pop_front().expect("checked above");
            // Stops follow their caller (D-S2b-15 (c), D-S2b-17 (b), D-S2b-18). A window
            // caller (the main window with the hold and toggle hotkeys and the silence
            // auto-stop, or the overlay) acts on the status *its own window* last
            // received, and sends the stop IPC only when that is `Recording`
            // (`useRecordingState.ts` `handleStop`; `useRecordingControls.ts` click and
            // double-click). On `Starting` the window cancels instead (the `Cancel`
            // commands model that call), on `Recovering` it only presents "Microphone
            // cleanup is still in progress", on anything else it does nothing. The
            // window can still show `Recording` after the shell entered `Recovering`,
            // because the emit reaches it in a delivery step of its own, and the
            // two windows can disagree. The `stop_native_recording` IPC has no such gate
            // and keeps every schedule, as does the supervisor's `Interrupted` auto-stop
            // (a flight, not a command). A stop issued earlier and still in flight when
            // the status turns `Recovering` is already a flight. The main window also
            // keeps `handleStop`'s in-flight guard and local `Processing` write (#1167):
            // once it has sent a stop it sends no second one until the IPC returns or it
            // receives `Idle`, and it shows `Processing` until an event moves it. The
            // overlay has neither (`useRecordingControls.ts`), so it can send twice.
            if let Cmd::Stop { caller, .. } = &cmd {
                match *caller {
                    StopCaller::Window(index) => {
                        // The main window's `handleStop` returns first when its own stop is
                        // still in flight (`isStoppingRef`, #1167); the overlay has no guard.
                        if w.windows[index].stop_in_flight {
                            if let Some(trace) = checker.trace.as_mut() {
                                trace.push(format!(
                                    "{clock:>4} frontend stop not sent ({} window has a stop in flight)",
                                    window_name(index)
                                ));
                            }
                            continue;
                        }
                        let observed = w.windows[index].observed;
                        if observed != Status::Recording {
                            if let Some(trace) = checker.trace.as_mut() {
                                trace.push(if observed == Status::Recovering {
                                    format!(
                                        "{clock:>4} frontend stop suppressed ({} window received Recovering)",
                                        window_name(index)
                                    )
                                } else {
                                    format!(
                                        "{clock:>4} frontend stop not sent ({} window shows {observed:?}){}",
                                        window_name(index),
                                        // The stop ran ahead of a delivery that would have
                                        // shown `Recording` (#1159: the competition's other side).
                                        if w.windows[index]
                                            .inbox
                                            .iter()
                                            .any(|event| event.status == Status::Recording)
                                        {
                                            " while Recording is still queued for it"
                                        } else {
                                            ""
                                        }
                                    )
                                });
                            }
                            continue;
                        }
                        if let Some(trace) = checker.trace.as_mut() {
                            trace.push(format!(
                                "{clock:>4} frontend stop sent ({} window shows Recording) as t{next_tid}",
                                window_name(index)
                            ));
                        }
                        if w.status == Status::Recovering {
                            if let Some(trace) = checker.trace.as_mut() {
                                trace.push(format!(
                                    "{clock:>4} frontend stop issued on a stale observation ({} window shows Recording) while the shell is Recovering; the other window shows {:?}",
                                    window_name(index),
                                    w.windows[1 - index].observed
                                ));
                            }
                        }
                        // The main window now shows `Processing` and holds its guard until
                        // the IPC returns.
                        if index == MAIN_WINDOW && late_reply {
                            checker.hits.second_stop_after_late_reply += 1;
                            late_reply = false;
                        }
                        w.window_stop_sent(index);
                    }
                    StopCaller::Ipc => {
                        if w.status == Status::Recovering {
                            if let Some(trace) = checker.trace.as_mut() {
                                trace.push(format!(
                                    "{clock:>4} unscoped stop IPC issued (status Recovering)"
                                ));
                            }
                        }
                    }
                }
            }
            if let Cmd::Start { caller, .. } = &cmd {
                // The main window's `beginRecording` keeps `startOperationRef` until its start's
                // response was processed: a second start gesture meanwhile awaits the same
                // operation and sends no IPC (`useRecordingState.ts:456-459`).
                if *caller == StartCaller::Window(MAIN_WINDOW)
                    && w.windows[MAIN_WINDOW].start_in_flight
                {
                    if let Some(trace) = checker.trace.as_mut() {
                        trace.push(format!(
                            "{clock:>4} frontend start coalesced (the main window's start is still in flight)"
                        ));
                    }
                    continue;
                }
                if *caller == StartCaller::Window(MAIN_WINDOW) {
                    w.window_start_sent();
                }
                if let Some(trace) = checker.trace.as_mut() {
                    trace.push(format!(
                        "{clock:>4} start issued by {} as t{next_tid}",
                        match caller {
                            StartCaller::Window(index) =>
                                format!("the {} window", window_name(*index)),
                            StartCaller::Ipc => "the IPC".to_string(),
                        }
                    ));
                }
            }
            let task: Box<dyn Task> = match cmd {
                Cmd::Start {
                    plan,
                    blockers,
                    app_disabled,
                    smart,
                    caller,
                    mic,
                } => Box::new(
                    StartFlight::new(plan, blockers, app_disabled, smart)
                        .with_caller(caller)
                        .with_mic(mic),
                ),
                Cmd::Import(plan) => Box::new(ImportFlight::new(plan)),
                Cmd::Stop { cfg, caller } => {
                    let flight = StopFlight::new(None, None, cfg);
                    Box::new(match caller {
                        StopCaller::Window(index) => flight.with_window(index),
                        StopCaller::Ipc => flight,
                    })
                }
                Cmd::Cancel { id } => Box::new(CancelFlight::new(id.resolve(w.current_id))),
            };
            slots.push(Slot {
                tid: next_tid,
                task,
                daemon: false,
            });
            next_tid += 1;
        }

        // A window receives its next status event (D-S2b-17 (b), D-S2b-18). Each window has
        // its own delivery cursor and receives at its own pace, so a delivery is a step of
        // the schedule of its own that can land between any two shell steps (or never
        // before a stop is issued). It is not a shell step: it changes no shell state, so
        // it takes no model time and no actor's weight.
        if window_ready(&w) && rng.chance(1, 3) {
            match deliver_next(&mut w, &mut rng, &mut checker, clock, None) {
                Delivered::ReplyOverProcessing => late_reply = true,
                Delivered::MainIdle => late_reply = false,
                Delivered::Other => {}
            }
            continue;
        }

        // Pick an actor: weighted order, first one that makes progress.
        let mut order: Vec<usize> = (0..slots.len()).collect();
        let mut progressed = false;
        let mut spawned: Vec<Box<dyn Task>> = Vec::new();
        while !order.is_empty() {
            let weights: Vec<u64> = order
                .iter()
                .map(|index| weight(&bias, slots[*index].task.name()))
                .collect();
            let pick = rng.weighted(&weights);
            let index = order.swap_remove(pick);
            w.step = clock;
            out.clear();
            let slot = &mut slots[index];
            let tid = slot.tid;
            let before = strict.then(|| format!("{w:?}"));
            let poll = {
                let mut cx = Cx {
                    w: &mut w,
                    out: &mut out,
                    spawn: &mut spawned,
                    tid,
                    mailbox: &mut mailbox,
                };
                if slot.task.droppable() && rng.chance(1, 6) {
                    slot.task.drop_now(&mut cx);
                    Poll::Done
                } else {
                    slot.task.step(&mut cx)
                }
            };
            if poll == Poll::Blocked {
                if let Some(before) = before {
                    assert_eq!(
                        before,
                        format!("{w:?}"),
                        "a blocked {} step changed the world (seed {seed})",
                        slot.task.name()
                    );
                }
                debug_assert!(
                    out.decisions.is_empty() && out.effects.is_empty(),
                    "a blocked step changed something: {} {:?}",
                    slot.task.name(),
                    out
                );
                continue;
            }
            progressed = true;
            checker.on_step(&w, &out);
            if poll == Poll::Done {
                slots.remove(index);
            }
            break;
        }
        for task in spawned {
            if let Some(trace) = checker.trace.as_mut() {
                trace.push(format!("{clock:>4} spawn t{next_tid} {}", task.name()));
            }
            slots.push(Slot {
                tid: next_tid,
                task,
                daemon: false,
            });
            next_tid += 1;
        }
        if progressed {
            steps += 1;
            clock += 1;
            continue;
        }
        // Nothing can run: jump the clock to the next command, or the run is over.
        match pending.front() {
            Some((at, _)) => clock = clock.max(*at),
            None => break,
        }
    }

    let stuck: Vec<&'static str> = slots
        .iter()
        .filter(|slot| !slot.daemon)
        .map(|slot| slot.task.name())
        .collect();
    checker.on_end(&w, &stuck);
    let lines = checker.trace.take().unwrap_or_default();
    (
        RunReport {
            seed,
            violations: std::mem::take(&mut checker.violations),
            hits: checker.hits.clone(),
            steps,
        },
        lines,
    )
}
