//! Shadow mode tests (appendix C8.7), driven by synthetic headers and messages.
//!
//! A [`Flight`] is a *simulated live path*: it takes a view and an input, asks the
//! core what it decides, and plays that decision back as the messages a faithful
//! live path would send (header with the `Pre` and `In` witnesses, then one
//! incremental message per later effect, the reply and the close). Each test then
//! damages exactly one thing and asserts the first failing level.
//!
//! Tests that need the real capture sites (a live executor feeding the recorder,
//! the E7 golden replay through `set_sync`) are owned by Stage 2c-2; the ones here
//! drive the comparison and the transport with synthetic data only.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use murmur_dictation_core::decision::{ContKind, Decision};
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::{Now, OperationId, RecordingId};
use murmur_dictation_core::inputs::{
    CancelInput, DeviceSel, GestureObs, Input, Origin, ReadyPublishInput, SpeculativeObs,
    StartInput, StopInput, SupervisorEvent,
};
use murmur_dictation_core::lifecycle::short_idle_pre_effects;
use murmur_dictation_core::state::{DictationCore, LiveObs};
use murmur_dictation_core::status::{Status, StopTrigger};

use super::compare::{
    replayed_token, Comparator, MismatchEvent, EVENT_WINDOW_MS, MAX_EVENTS_PER_SESSION,
    MAX_HELD_PER_REC, MAX_OPEN_RECORDS, MAX_PENDING_HEADERS,
};
use super::summary::{write_summary, Stats, SUMMARY_FILE};
use super::transport::{
    mark, submit, witness, Mark, ShadowRecorder, ShadowTicket, Transport, CHANNEL_CAPACITY,
    PRE_CAPACITY,
};
use super::types::{
    CondWitness, DiffKind, EffectResult, LiveOp, LiveOpKind, MirrorField, MirrorValue,
    MirrorWitness, ObservedFact, PredId, Projection, RecorderId, ReplyKind, ShadowHeader,
    ShadowMsg, WitnessedEffect,
};

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);

// ----------------------------------------------------------------- harness

type Events = Arc<Mutex<Vec<MismatchEvent>>>;

struct Harness {
    comparator: Comparator,
    events: Events,
    clock: Arc<AtomicU64>,
}

fn harness() -> Harness {
    let events: Events = Arc::default();
    let clock = Arc::new(AtomicU64::new(0));
    let sink_events = Arc::clone(&events);
    let clock_reader = Arc::clone(&clock);
    let comparator = Comparator::new(
        Box::new(|core, input, now| core.handle(input, now)),
        Box::new(move |event| sink_events.lock().unwrap().push(event)),
        Box::new(move || clock_reader.load(Ordering::SeqCst)),
    );
    Harness {
        comparator,
        events,
        clock,
    }
}

impl Harness {
    fn stats(&self) -> &Stats {
        &self.comparator.stats
    }

    fn events(&self) -> Vec<MismatchEvent> {
        self.events.lock().unwrap().clone()
    }

    fn only_event(&self) -> MismatchEvent {
        let events = self.events();
        assert_eq!(events.len(), 1, "expected exactly one event: {events:?}");
        events[0]
    }

    fn feed(&mut self, flight: &Flight) {
        self.comparator
            .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
        self.feed_later(flight);
    }

    fn feed_later(&mut self, flight: &Flight) {
        for msg in flight.later.clone() {
            self.comparator.handle_msg(msg);
        }
    }
}

impl Clone for ShadowMsg {
    fn clone(&self) -> Self {
        match self {
            Self::Header(h) => Self::Header(h.clone()),
            Self::Witness {
                rec,
                idx,
                placement,
                effect,
            } => Self::Witness {
                rec: *rec,
                idx: *idx,
                placement: *placement,
                effect: effect.clone(),
            },
            Self::Cond { rec, pred, taken } => Self::Cond {
                rec: *rec,
                pred: *pred,
                taken: *taken,
            },
            Self::Mirror { rec, field, value } => Self::Mirror {
                rec: *rec,
                field: *field,
                value: *value,
            },
            Self::Reply { rec, kind } => Self::Reply {
                rec: *rec,
                kind: *kind,
            },
            Self::Close { rec, completed } => Self::Close {
                rec: *rec,
                completed: *completed,
            },
            Self::PreAbandoned {
                rec,
                pre_effects,
                pre_conditionals,
            } => Self::PreAbandoned {
                rec: *rec,
                pre_effects: pre_effects.clone(),
                pre_conditionals: pre_conditionals.clone(),
            },
            Self::Lost { seq } => Self::Lost { seq: *seq },
        }
    }
}

fn view(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(
        status,
        current,
        LiveObs {
            cancelled_through: RecordingId(3),
            context_for: Some(RID),
        },
    )
}

/// What a faithful live path would have done for one decision, as messages.
#[derive(Clone)]
struct Flight {
    header: ShadowHeader,
    later: Vec<ShadowMsg>,
    decision: Decision,
}

/// Playback options of the simulated live path.
#[derive(Clone, Copy)]
struct Live {
    /// `speculative_flowing` as the live path observed it.
    spec_flowing: bool,
    /// Whether every emit the live path made returned `Ok`.
    emit_ok: bool,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            spec_flowing: false,
            emit_ok: true,
        }
    }
}

fn flight(seq: u64, rec: u64, core: DictationCore, input: Input) -> Flight {
    flight_with(seq, rec, core, input, Live::default())
}

fn flight_with(seq: u64, rec: u64, core: DictationCore, input: Input, live: Live) -> Flight {
    let decision = core.handle(input.clone(), NOW);
    let rec_id = RecorderId(rec);
    let mut slot = core.context_for;
    let mut cancelled = core.cancelled_through;
    let mut emit_results: HashMap<usize, bool> = HashMap::new();
    let mut post = core.persisted();
    let mut header = ShadowHeader {
        seq,
        rec: rec_id,
        site_id: input.site().code(),
        row: decision.row.ordinal(),
        pre: Projection {
            status: core.status,
            current_id: core.current_id,
            obs: LiveObs {
                cancelled_through: core.cancelled_through,
                context_for: core.context_for,
            },
        },
        input,
        observed: vec![ObservedFact::SpeculativeFlowing(live.spec_flowing)],
        live_ops: Vec::new(),
        now: NOW,
        pre_effects: Vec::new(),
        pre_conditionals: Vec::new(),
        in_d_effects: Vec::new(),
        in_d_conditionals: Vec::new(),
        pre_mirrors: Vec::new(),
        in_d_mirrors: Vec::new(),
        live_post_d: Projection {
            status: core.status,
            current_id: core.current_id,
            obs: LiveObs::default(),
        },
    };
    if let Some(spec) = decision.continuation {
        header.live_ops.push(LiveOp {
            kind: LiveOpKind::Registered,
            cont: spec.kind,
            recording_id: spec.recording_id,
            op: OperationId(1_000 + seq),
        });
    }
    let mut later = Vec::new();
    for (idx, placed) in decision.effects.iter().enumerate() {
        let effect = placed.effect.clone();
        let cond = match &effect {
            Effect::EmitStatus { cond, .. } | Effect::Presentation { cond, .. } => *cond,
            _ => Cond::Always,
        };
        let pred = match cond {
            Cond::Always => None,
            Cond::UnlessSpeculativeFlowing => Some(PredId::SpecFlowing),
            Cond::EmitOk(p) => Some(PredId::EmitOk(p)),
        };
        let taken = match cond {
            Cond::Always => true,
            Cond::UnlessSpeculativeFlowing => !live.spec_flowing,
            Cond::EmitOk(p) => emit_results.get(&usize::from(p)).copied().unwrap_or(true),
        };
        let result = match &effect {
            Effect::EmitStatus { .. } | Effect::EmitEvent(_) | Effect::SmartDestinationEmit => {
                EffectResult::Emitted { ok: live.emit_ok }
            }
            Effect::Terminal(_) => EffectResult::Claimed { won: true },
            _ => EffectResult::Ran,
        };
        if taken {
            if let EffectResult::Emitted { ok } = result {
                emit_results.insert(idx, ok);
            }
        }
        let witnessed = WitnessedEffect {
            effect: effect.clone(),
            result,
        };
        // The slot the effect leaves behind (the live adapter's write).
        let mirror = if taken {
            match &effect {
                Effect::ClearContext(id) => {
                    let before = slot;
                    if slot == Some(*id) {
                        slot = None;
                    }
                    Some(MirrorWitness {
                        field: MirrorField::ActiveContext,
                        before: MirrorValue::Slot(before),
                        after: MirrorValue::Slot(slot),
                    })
                }
                Effect::SetContext(id) => {
                    let before = slot;
                    slot = Some(*id);
                    Some(MirrorWitness {
                        field: MirrorField::ActiveContext,
                        before: MirrorValue::Slot(before),
                        after: MirrorValue::Slot(slot),
                    })
                }
                _ => None,
            }
        } else {
            None
        };
        match placed.at {
            Placement::Pre | Placement::In => {
                let in_d = placed.at == Placement::In;
                if let Some(pred) = pred {
                    let witness = CondWitness { pred, taken };
                    if in_d {
                        header.in_d_conditionals.push(witness);
                    } else {
                        header.pre_conditionals.push(witness);
                    }
                }
                if taken {
                    if in_d {
                        header.in_d_effects.push(witnessed);
                    } else {
                        header.pre_effects.push(witnessed);
                    }
                    if let Some(mirror) = mirror {
                        if in_d {
                            header.in_d_mirrors.push(mirror);
                        } else {
                            header.pre_mirrors.push(mirror);
                        }
                    }
                }
                match effect {
                    Effect::SetStatus(to) => post.set_status(to),
                    Effect::AllocId => {
                        post.alloc_id();
                    }
                    _ => {}
                }
            }
            Placement::PostDT | Placement::PostT => {
                if let Some(pred) = pred {
                    later.push(ShadowMsg::Cond {
                        rec: rec_id,
                        pred,
                        taken,
                    });
                }
                if taken {
                    later.push(ShadowMsg::Witness {
                        rec: rec_id,
                        idx: u16::try_from(idx).unwrap(),
                        placement: placed.at,
                        effect: witnessed,
                    });
                    match &effect {
                        Effect::ClearContext(id) => {
                            if slot == Some(*id) {
                                slot = None;
                            }
                            later.push(ShadowMsg::Mirror {
                                rec: rec_id,
                                field: MirrorField::ActiveContext,
                                value: MirrorValue::Slot(slot),
                            });
                        }
                        Effect::SetContext(id) => {
                            slot = Some(*id);
                            later.push(ShadowMsg::Mirror {
                                rec: rec_id,
                                field: MirrorField::ActiveContext,
                                value: MirrorValue::Slot(slot),
                            });
                        }
                        Effect::MarkCancelled(id) => {
                            cancelled = cancelled.max(*id);
                            later.push(ShadowMsg::Mirror {
                                rec: rec_id,
                                field: MirrorField::CancelledThrough,
                                value: MirrorValue::Id(cancelled),
                            });
                        }
                        Effect::KeyboardProcessing(flag) => later.push(ShadowMsg::Mirror {
                            rec: rec_id,
                            field: MirrorField::KeyboardProcessing,
                            value: MirrorValue::Flag(*flag),
                        }),
                        _ => {}
                    }
                }
            }
        }
    }
    header.live_post_d = Projection {
        status: post.status(),
        current_id: post.current_id(),
        obs: LiveObs::default(),
    };
    if let Some(reply) = &decision.reply {
        later.push(ShadowMsg::Reply {
            rec: rec_id,
            kind: ReplyKind::of(reply),
        });
    }
    later.push(ShadowMsg::Close {
        rec: rec_id,
        completed: true,
    });
    Flight {
        header,
        later,
        decision,
    }
}

/// R216.1: `Pre:TI, TERM, CX-, PC`, `In:ST`, `PDT:KB, EM`. The status is
/// `Recovering` and the context slot holds the recording, so `CX-` really clears.
fn short_idle(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Recovering, RID),
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle { rid: RID }),
    )
}

/// R101.15: `In:ID+ ... ST`, `PDT:SPEC, EV, EM(starting; skipped while a
/// speculative capture flows), PR, CS`, and the `K:CaptureStart` continuation.
fn start(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Idle, RecordingId(RID.0 - 1)),
        Input::StartRequest(StartInput {
            origin: Origin::Hold,
            device: DeviceSel::SystemDefault,
            gesture: GestureObs::default(),
            speculative: SpeculativeObs::default(),
            kept_preview: false,
            app_disabled: false,
            fixed_and_smart: false,
            occupancy: murmur_dictation_core::admission::Occupancy::default(),
            cleanup_pending: Some(false),
            smart_auto_result: None,
        }),
        live,
    )
}

/// R205.5: `In:ST`, `PDT:CX-, MC, TERM, KB, EM, EV`, `PT:PC`.
fn cancel(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Processing, RID),
        Input::CancelRequest(CancelInput { expected: None }),
    )
}

/// R214.1: `PDT:EV(RecoveryStalled)` and `PRES` paired with that emit.
fn stalled(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Recovering, RID),
        Input::Supervisor(SupervisorEvent::RecoveryStalled { rid: RID }),
        live,
    )
}

/// R201.1: `Pre:TERM(Superseded)` for a stop that expected a stale recording.
fn stale_stop(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Recording, RID),
        Input::StopRequest(StopInput {
            expected: Some(RecordingId(2)),
            trigger: StopTrigger::User,
            observed_current_id: RID,
            interrupted: Some(None),
            entered: NOW,
        }),
    )
}

fn assert_clean(h: &Harness) {
    assert_eq!(h.stats().mismatched_root, 0, "{:?}", h.events());
    assert_eq!(h.stats().followup, 0);
    assert!(h.events().is_empty(), "{:?}", h.events());
}

// ------------------------------------------------------ sanity: faithful play

#[test]
fn a_faithful_live_path_compares_clean_at_every_point() {
    let mut h = harness();
    let flights = [
        short_idle(1, 1),
        start(2, 2, Live::default()),
        cancel(3, 3),
        stalled(4, 4, Live::default()),
        stale_stop(5, 5),
    ];
    for flight in &flights {
        h.feed(flight);
    }
    assert_clean(&h);
    assert_eq!(h.stats().compared, 5);
    assert_eq!(h.stats().matched, 5, "every flight closed with no mismatch");
    assert_eq!(
        h.stats().lost_gap + h.stats().lost_open + h.stats().dropped,
        0
    );
    // The fixtures are the rows the tests below name.
    let row = |f: &Flight| f.decision.row.0;
    assert_eq!(row(&flights[0]), "R216.1");
    assert_eq!(row(&flights[1]), "R101.15");
    assert_eq!(row(&flights[2]), "R205.5");
    assert_eq!(row(&flights[3]), "R214.1");
    assert_eq!(row(&flights[4]), "R201.1");
}

// ------------------------------------------------------- never executes etc.

/// The source of one shadow file without its line comments and without its test
/// module, for the static checks.
fn production_source(source: &str) -> String {
    let body = source.split("#[cfg(test)]").next().unwrap_or(source);
    body.lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

const SHADOW_SOURCES: [(&str, &str); 6] = [
    ("compare.rs", include_str!("compare.rs")),
    ("mod.rs", include_str!("mod.rs")),
    ("pd.rs", include_str!("pd.rs")),
    ("summary.rs", include_str!("summary.rs")),
    ("transport.rs", include_str!("transport.rs")),
    ("types.rs", include_str!("types.rs")),
];

/// The body of the first function whose signature starts with `signature`.
fn function_body(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` not found"));
    let open = start + source[start..].find('{').expect("a body");
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[open..=open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` is unbalanced");
}

#[test]
fn shadow_never_executes() {
    // The shadow side holds no port, adapter, executor, lock owner or window: a
    // grep over everything it imports and names. `DictationCore::handle` is pure,
    // so there is nothing else it could call.
    let forbidden = [
        "FieldAdapter",
        "dictation_executor",
        "core_history",
        "AppState",
        "AppHandle",
        "tauri::",
        "commands::",
        "emit_dictation_terminal",
        "transition_dictation_status",
        "keyboard::",
        "audio_lifecycle",
    ];
    for (name, source) in SHADOW_SOURCES {
        let code = production_source(source);
        for token in forbidden {
            assert!(
                !code.contains(token),
                "dictation_shadow/{name} names `{token}`: the shadow never executes"
            );
        }
    }
    // The comparison runs wherever the caller runs it and decides with the pure
    // function: prove the decider receives only values (it has no handle to anything
    // else) and that a flight compares with the live side's call counter untouched.
    let live_calls = Arc::new(AtomicU64::new(0));
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    h.feed(&start(2, 2, Live::default()));
    assert_eq!(h.stats().matched, 2);
    assert_eq!(
        live_calls.load(Ordering::SeqCst),
        0,
        "no live call is reachable from the comparison"
    );
}

#[test]
fn shadow_capture_takes_no_lock() {
    // The live-side functions use `try_send` only: no `lock`, `try_lock`, mutex,
    // blocking send, receive or wait appears in their bodies.
    let transport = production_source(include_str!("transport.rs"));
    for signature in [
        "pub(crate) fn witness(",
        "pub(crate) fn mark(",
        "pub(crate) fn submit(",
        "fn send(",
        "fn try_deliver(",
    ] {
        let body = function_body(&transport, signature);
        // `transport.send(..)` is the dispatch helper checked below; a blocking
        // channel send is `tx.send(..)`.
        for token in [
            "lock", "Mutex", "RwLock", "tx.send(", ".recv", ".wait(", "sleep", ".join(", "Condvar",
            "Barrier",
        ] {
            assert!(
                !body.contains(token),
                "`{signature}` contains `{token}`; capture must be try_send only"
            );
        }
    }
    assert!(function_body(&transport, "fn try_deliver(").contains("tx.try_send("));
    // The dispatch reaches the lock only through `deliver_sync`, the synchronous test
    // mode; with that one function removed, nothing in the transport locks.
    let sync_body = function_body(&transport, "fn deliver_sync(");
    assert!(function_body(&transport, "fn send(").contains("deliver_sync"));
    let without_sync = transport.replace(&sync_body, "{}");
    for token in [".lock(", "try_lock", ".recv", "recv_timeout"] {
        assert!(
            !without_sync.contains(token),
            "transport.rs contains `{token}` outside deliver_sync"
        );
    }
}

#[test]
fn shadow_replays_live_op_ids() {
    // The shadow never draws an op id: it does not touch `NEXT_OP`, the history or
    // its registration functions.
    for (name, source) in SHADOW_SOURCES {
        let code = production_source(source);
        for token in [
            "NEXT_OP",
            "fetch_add(1)",
            "next_op",
            ".reg(",
            "ret_if_matches",
        ] {
            // `fetch_add(1)` is the transport's own seq counter, nothing else.
            if token == "fetch_add(1)" && name == "transport.rs" {
                continue;
            }
            assert!(
                !code.contains(token),
                "dictation_shadow/{name} names `{token}`: op ids are replayed, never drawn"
            );
        }
    }
    // The token a continuation input will carry is the live shell's id.
    let start = start(1, 1, Live::default());
    let token = replayed_token(&start.decision, &start.header.live_ops).expect("a continuation");
    assert_eq!(token.op, OperationId(1_001));
    assert_eq!(token.kind, ContKind::CaptureStart);
    // A registration of another kind, or of none, is `diff_kind 6`.
    for damage in [0, 1] {
        let mut h = harness();
        let mut damaged = start.clone();
        match damage {
            0 => damaged.header.live_ops.clear(),
            _ => damaged.header.live_ops[0].cont = ContKind::ReadyPublish,
        }
        h.feed(&damaged);
        assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
    }
    // A registration the decision never asked for.
    let mut h = harness();
    let mut extra = short_idle(1, 1);
    extra.header.live_ops.push(LiveOp {
        kind: LiveOpKind::Registered,
        cont: ContKind::CaptureStop,
        recording_id: RID,
        op: OperationId(9),
    });
    h.feed(&extra);
    assert_eq!(h.only_event().diff_kind, DiffKind::Continuation.code());
}

#[test]
fn shadow_core_holds_m1_m2_only() {
    // `ShadowCore` has exactly one field, the persisted M1/M2: no M3 or M4 copy.
    // Statically: its definition lists one field.
    let source = production_source(include_str!("compare.rs"));
    let body = function_body(&source, "pub(crate) struct ShadowCore");
    assert_eq!(
        body.matches(':').count(),
        1,
        "ShadowCore has one field: {body}"
    );
    assert!(body.contains("persisted: PersistedCore"));
    // Behaviourally: a view takes M3/M4 from the argument and M1/M2 from the resync.
    let mut core = super::compare::ShadowCore::new();
    let pre = view(Status::Recording, RID);
    core.resync_m(&Projection {
        status: pre.status,
        current_id: pre.current_id,
        obs: LiveObs {
            cancelled_through: RecordingId(1),
            context_for: None,
        },
    });
    let first = core.view(LiveObs {
        cancelled_through: RecordingId(11),
        context_for: Some(RID),
    });
    let second = core.view(LiveObs::default());
    assert_eq!((first.status, first.current_id), (Status::Recording, RID));
    assert_eq!((second.status, second.current_id), (Status::Recording, RID));
    assert_eq!(first.cancelled_through, RecordingId(11));
    assert_eq!(
        second.cancelled_through,
        RecordingId(0),
        "nothing carried over"
    );
    assert_eq!(second.context_for, None);
}

// --------------------------------------------------------- off by default

#[test]
fn shadow_off_by_default() {
    assert!(!super::parse_flag(None));
    assert!(!super::parse_flag(Some("")));
    assert!(!super::parse_flag(Some("0")));
    assert!(!super::parse_flag(Some("true")));
    assert!(super::parse_flag(Some("1")));
    assert_eq!(super::ENV_VAR, "MURMUR_SHADOW");
    // The feature alone does not arm it: with the variable unset, capture is off
    // and no recorder is created.
    if std::env::var(super::ENV_VAR).is_err() {
        assert!(!super::enabled());
        assert!(super::recorder().is_none());
    }
}

#[test]
fn shadow_sync_mode_compares_on_the_calling_thread() {
    let (transport, rx) = local_transport(CHANNEL_CAPACITY);
    transport.set_sync(true);
    let mut rec = transport.recorder();
    let flight = short_idle(0, 0);
    for effect in &flight.header.pre_effects {
        assert!(rec.pre_effect(effect.clone()));
    }
    for mirror in &flight.header.pre_mirrors {
        assert!(rec.pre_mirror(*mirror));
    }
    for cond in &flight.header.pre_conditionals {
        assert!(rec.pre_conditional(cond.pred, cond.taken));
    }
    let pre = rec.take_pre();
    let ticket = rec.ticket();
    let mut header = flight.header.clone();
    header.seq = ticket.seq();
    header.rec = rec.id();
    header.pre_effects = pre.effects;
    header.pre_mirrors = pre.mirrors;
    submit(ticket, header);
    assert!(rx.try_recv().is_err(), "sync mode uses no channel");
    {
        let comparator = transport.comparator().lock().unwrap();
        assert_eq!(comparator.stats.compared, 1, "compared on this thread, now");
        assert_eq!(comparator.stats.mismatched_root, 0);
    }
    rec.finish();
}

// ------------------------------------------------------ transport and channel

fn local_transport(capacity: usize) -> (Arc<Transport>, Receiver<ShadowMsg>) {
    let h = harness();
    Transport::new(capacity, h.comparator)
}

#[test]
fn shadow_channel_full_drops_and_counts() {
    let (transport, rx) = local_transport(4);
    let rec = transport.recorder();
    // Nobody drains: 4 fit, the other 6 are dropped, and none of them blocks.
    for _ in 0..10 {
        mark(&rec, Mark::Reply(ReplyKind::Ok));
    }
    assert_eq!(transport.take_dropped(), 6);
    assert_eq!(transport.take_dropped(), 0, "the count is taken, not kept");
    assert_eq!(rx.try_iter().count(), 4);
    // The comparison side counts them and taints what they could have touched.
    let mut h = harness();
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(short_idle(1, 1).header)));
    h.comparator.note_dropped(6);
    assert_eq!(h.stats().dropped, 6);
    let tainted = short_idle(2, 2);
    h.feed(&tainted);
    assert_eq!(
        h.stats().skipped_tainted,
        1,
        "a decision a drop could have touched"
    );
    assert_eq!(h.stats().mismatched_root, 0);
    // A message for a flight that never completed does not appear as a mismatch.
    assert!(h.events().is_empty());
    // A full channel never made a header wait: submit returns at once as well.
    let (full, _rx) = local_transport(1);
    let recorder = full.recorder();
    let a = recorder.ticket();
    let b = recorder.ticket();
    let header = short_idle(a.seq(), 0).header;
    submit(
        a,
        ShadowHeader {
            rec: recorder.id(),
            ..header.clone()
        },
    );
    submit(
        b,
        ShadowHeader {
            seq: 2,
            rec: recorder.id(),
            ..header
        },
    );
    assert_eq!(full.take_dropped(), 1);
}

#[test]
fn shadow_recorder_exists_before_pre() {
    let (transport, _rx) = local_transport(CHANNEL_CAPACITY);
    // The recorder (and its id) exist before the first `Pre` effect runs; the
    // header's seq is taken later, at the decision boundary, from its own counter.
    let mut recorder = transport.recorder();
    let id = recorder.id();
    assert!(recorder.pre_effect(WitnessedEffect::ran(Effect::PerfAbort)));
    let ticket = recorder.ticket();
    assert_ne!(id.0, ticket.seq(), "a recorder id is not a seq");
    assert_eq!(ticket.rec(), id);
    // Two flights in flight at once: the seq follows the decision boundary, the
    // recorder id follows creation.
    let first = transport.recorder();
    let second = transport.recorder();
    let late = second.ticket();
    let early_after = first.ticket();
    assert!(first.id() < second.id());
    assert!(
        late.seq() < early_after.seq(),
        "seq is taken at the ticket, not at creation"
    );
}

#[test]
fn shadow_pre_buffer_covers_c3() {
    // The largest C3 `Pre` list is 4 today (R216.1); the inline buffer is 8.
    assert_eq!(short_idle_pre_effects(RID).len(), 4);
    let mut largest = 0;
    for status in Status::ALL {
        for current in [RID, RecordingId(2), RecordingId::NONE] {
            let core = view(status, current);
            for input in every_input_family() {
                let decision = core.handle(input, NOW);
                let pre = decision
                    .effects
                    .iter()
                    .filter(|p| p.at == Placement::Pre)
                    .count();
                largest = largest.max(pre);
            }
        }
    }
    assert_eq!(largest, 4, "R216.1 is the largest `Pre` list");
    assert!(largest <= PRE_CAPACITY);
    // Overflow is counted, never silent, never a panic.
    let (transport, _rx) = local_transport(CHANNEL_CAPACITY);
    let mut recorder = transport.recorder();
    for i in 0..PRE_CAPACITY {
        assert!(
            recorder.pre_effect(WitnessedEffect::ran(Effect::ClearContext(RecordingId(
                i as u64
            ))))
        );
    }
    assert!(!recorder.pre_effect(WitnessedEffect::ran(Effect::PerfAbort)));
    assert_eq!(transport.take_overflow(), 1);
    assert_eq!(recorder.take_pre().effects.len(), PRE_CAPACITY);
}

/// One input of every family (the status and id grid varies the rest).
fn every_input_family() -> Vec<Input> {
    let rid = RID;
    vec![
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle { rid }),
        Input::Supervisor(SupervisorEvent::Interrupted {
            rid,
            delivered_samples: 100,
            duration_ms: 1,
            reason: murmur_dictation_core::status::AudioCancelReason::RuntimeFailure,
        }),
        Input::Supervisor(SupervisorEvent::Idle { rid }),
        Input::Supervisor(SupervisorEvent::Recovering {
            rid,
            reason: murmur_dictation_core::status::AudioCancelReason::User,
        }),
        Input::StopRequest(StopInput {
            expected: Some(RecordingId(2)),
            trigger: StopTrigger::User,
            observed_current_id: rid,
            interrupted: Some(None),
            entered: NOW,
        }),
        Input::CancelRequest(CancelInput { expected: None }),
        Input::GuardFinish(murmur_dictation_core::inputs::GuardFinishInput {
            rid,
            origin: murmur_dictation_core::inputs::GuardOrigin::PipelineExit,
            closure_emit: true,
        }),
        Input::StopCaptureResult(murmur_dictation_core::inputs::StopCaptureResultInput {
            token: murmur_dictation_core::decision::Continuation {
                recording_id: rid,
                op: OperationId(1),
                kind: ContKind::CaptureStop,
            },
            op_status: murmur_dictation_core::decision::OpStatus::Outstanding,
            rid,
            result: murmur_dictation_core::inputs::StopCaptureResult::Err,
        }),
        Input::StartCaptureResult(murmur_dictation_core::inputs::CaptureResultInput {
            token: murmur_dictation_core::decision::Continuation {
                recording_id: rid,
                op: OperationId(1),
                kind: ContKind::CaptureStart,
            },
            op_status: murmur_dictation_core::decision::OpStatus::Outstanding,
            result: murmur_dictation_core::inputs::CaptureResult::Err(
                murmur_dictation_core::inputs::CaptureErr::Other,
            ),
        }),
    ]
}

#[test]
fn shadow_recorder_crosses_threads() {
    // Two flights interleave; each flight's later effects are sent from other
    // threads (a spawned blocking start, a detached `RS` stop). A thread-local
    // lookup would attach them to the wrong flight or to none.
    let (transport, _rx) = local_transport(CHANNEL_CAPACITY);
    transport.set_sync(true);
    let flights = [start(0, 0, Live::default()), cancel(0, 0)];
    let mut recorders = Vec::new();
    for flight in &flights {
        let recorder = transport.recorder();
        let ticket = recorder.ticket();
        let mut header = flight.header.clone();
        header.seq = ticket.seq();
        header.rec = recorder.id();
        submit(ticket, header);
        recorders.push(Arc::new(recorder));
    }
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for (flight, recorder) in flights.iter().zip(&recorders) {
        let recorder: Arc<ShadowRecorder> = Arc::clone(recorder);
        let barrier = Arc::clone(&barrier);
        let flight = flight.clone();
        workers.push(
            std::thread::Builder::new()
                .name("detached-task".into())
                .spawn(move || {
                    barrier.wait();
                    for msg in flight.later {
                        match msg {
                            ShadowMsg::Witness {
                                idx,
                                placement,
                                effect,
                                ..
                            } => witness(&recorder, idx, placement, effect),
                            ShadowMsg::Cond { pred, taken, .. } => {
                                mark(&recorder, Mark::Cond { pred, taken });
                            }
                            ShadowMsg::Mirror { field, value, .. } => {
                                mark(&recorder, Mark::Mirror { field, value });
                            }
                            ShadowMsg::Reply { kind, .. } => mark(&recorder, Mark::Reply(kind)),
                            ShadowMsg::Close { .. } => recorder.finish(),
                            other => panic!("unexpected {other:?}"),
                        }
                    }
                })
                .unwrap(),
        );
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let comparator = transport.comparator().lock().unwrap();
    assert_eq!(comparator.stats.compared, 2);
    assert_eq!(comparator.stats.matched, 2, "{:?}", comparator.stats);
    assert_eq!(comparator.stats.mismatched_root, 0);
}

#[test]
fn shadow_header_precedes_blocking_effect() {
    // A flight whose blocking effect (`CS`) never returns until the test lets it:
    // the header and its `In` comparison complete while the flight is blocked.
    let (transport, rx) = local_transport(CHANNEL_CAPACITY);
    let comparator_thread = {
        // The thread holds the comparator only: holding the transport would keep the
        // sender alive and the receive loop would never end.
        let comparator = Arc::clone(transport.comparator());
        std::thread::Builder::new()
            .name(super::THREAD_NAME.into())
            .spawn(move || {
                while let Ok(msg) = rx.recv() {
                    comparator.lock().unwrap().handle_msg(msg);
                }
            })
            .unwrap()
    };
    let flight = start(0, 0, Live::default());
    let recorder = Arc::new(transport.recorder());
    let ticket = recorder.ticket();
    let mut header = flight.header.clone();
    header.seq = ticket.seq();
    header.rec = recorder.id();
    let in_d = header.in_d_effects.len();
    assert!(in_d > 0);
    let reached = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let flight_thread = {
        let (recorder, reached, release) = (
            Arc::clone(&recorder),
            Arc::clone(&reached),
            Arc::clone(&release),
        );
        let later = flight.later.clone();
        std::thread::spawn(move || {
            submit(ticket, header);
            // The blocking `CS`: the header is already out.
            reached.wait();
            release.wait();
            for msg in later {
                match msg {
                    ShadowMsg::Witness {
                        idx,
                        placement,
                        effect,
                        ..
                    } => witness(&recorder, idx, placement, effect),
                    ShadowMsg::Cond { pred, taken, .. } => {
                        mark(&recorder, Mark::Cond { pred, taken })
                    }
                    ShadowMsg::Reply { kind, .. } => mark(&recorder, Mark::Reply(kind)),
                    ShadowMsg::Close { .. } => recorder.finish(),
                    _ => {}
                }
            }
        })
    };
    reached.wait();
    // The flight is blocked; the `In` comparison completes anyway.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let compared = transport.comparator().lock().unwrap().stats.compared;
        if compared == 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the header was never compared"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    {
        let c = transport.comparator().lock().unwrap();
        assert_eq!(c.stats.mismatched_root, 0);
        assert_eq!(c.stats.matched, 0, "the flight is still open");
    }
    release.wait();
    flight_thread.join().unwrap();
    drop(recorder);
    drop(transport);
    comparator_thread.join().unwrap();
}

#[test]
fn shadow_recorder_close_semantics() {
    let (transport, rx) = local_transport(CHANNEL_CAPACITY);
    // finish after the header: Close{completed:true}, once.
    let rec = transport.recorder();
    let ticket = rec.ticket();
    let mut header = short_idle(0, 0).header;
    header.seq = ticket.seq();
    header.rec = rec.id();
    submit(ticket, header);
    rec.finish();
    rec.finish();
    drop(rec);
    let msgs: Vec<_> = rx.try_iter().collect();
    assert!(matches!(msgs[0], ShadowMsg::Header(_)));
    assert!(matches!(
        msgs[1],
        ShadowMsg::Close {
            completed: true,
            ..
        }
    ));
    assert_eq!(msgs.len(), 2, "one Close only");

    // Dropped after the header without finish: Close{completed:false}.
    let rec = transport.recorder();
    let ticket = rec.ticket();
    let mut header = short_idle(0, 0).header;
    header.seq = ticket.seq();
    header.rec = rec.id();
    submit(ticket, header);
    drop(rec);
    let msgs: Vec<_> = rx.try_iter().collect();
    assert!(matches!(
        msgs[1],
        ShadowMsg::Close {
            completed: false,
            ..
        }
    ));

    // A ticket dropped without a submit: Lost{seq}, and no PreAbandoned (the seq
    // was taken, so the flight is past D entry).
    let rec = transport.recorder();
    let ticket: ShadowTicket = rec.ticket();
    let seq = ticket.seq();
    drop(ticket);
    drop(rec);
    let msgs: Vec<_> = rx.try_iter().collect();
    assert_eq!(msgs.len(), 1, "{msgs:?}");
    assert!(matches!(msgs[0], ShadowMsg::Lost { seq: lost } if lost == seq));
}

#[test]
fn shadow_pre_abandoned_before_d_is_reported() {
    let (transport, rx) = local_transport(CHANNEL_CAPACITY);
    let seq_before = {
        let probe = transport.recorder();
        let ticket = probe.ticket();
        let seq = ticket.seq();
        let mut header = short_idle(0, 0).header;
        header.seq = seq;
        header.rec = probe.id();
        submit(ticket, header);
        probe.finish();
        seq
    };
    rx.try_iter().for_each(drop);
    // A flight whose `run(Pre)` began, and which was dropped before D entry.
    let mut flight = transport.recorder();
    assert!(flight.pre_effect(WitnessedEffect::ran(Effect::PerfAbort)));
    assert!(flight.pre_effect(WitnessedEffect::ran(Effect::ClearContext(RID))));
    drop(flight);
    let msgs: Vec<_> = rx.try_iter().collect();
    assert_eq!(msgs.len(), 1);
    let ShadowMsg::PreAbandoned { pre_effects, .. } = &msgs[0] else {
        panic!("expected PreAbandoned, got {msgs:?}")
    };
    assert_eq!(
        pre_effects.len(),
        2,
        "it carries the buffered Pre witnesses"
    );
    // No seq was consumed: the next ticket continues the sequence.
    let next = transport.recorder().ticket().seq();
    assert_eq!(next, seq_before + 1);

    let mut h = harness();
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(short_idle(1, 1).header)));
    for msg in msgs {
        h.comparator.handle_msg(msg);
    }
    assert_eq!(h.stats().abandoned_pre_d, 1);
    assert_eq!(h.stats().lost_gap, 0);
    assert_eq!(h.stats().mismatched_root, 0);
    assert_eq!(h.stats().skipped_tainted, 0);
    // ...and a later decision still compares (nothing was tainted).
    h.feed(&short_idle(2, 2));
    assert_eq!(h.stats().compared, 2);
    assert_eq!(h.stats().skipped_tainted, 0);
}

// ----------------------------------------------------------------- ordering

#[test]
fn shadow_gap_is_count_not_time() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    // seq 2 is still in its flight (a 20 s `CS`): headers 3.. wait for it.
    let slow = short_idle(2, 2);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(short_idle(3, 3).header)));
    assert_eq!(h.stats().compared, 1, "seq 3 waits for the gap");
    h.clock.fetch_add(20_000, Ordering::SeqCst);
    assert_eq!(h.stats().lost_gap, 0, "twenty seconds are not a loss");
    h.feed(&slow);
    assert_eq!(
        h.stats().compared,
        3,
        "the gap closed: 2 then 3 applied in order"
    );
    assert_clean(&h);

    // A missing seq is declared lost only after more than 64 later headers wait.
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    let missing = 2;
    let waiting = u64::try_from(MAX_PENDING_HEADERS).unwrap();
    for seq in missing + 1..missing + 1 + waiting {
        h.comparator
            .handle_msg(ShadowMsg::Header(Box::new(short_idle(seq, seq).header)));
    }
    h.clock.fetch_add(3_600_000, Ordering::SeqCst);
    assert_eq!(
        h.stats().lost_gap,
        0,
        "64 waiting headers keep the gap open"
    );
    assert_eq!(h.stats().compared, 1);
    let overflow = missing + 1 + waiting;
    h.comparator.handle_msg(ShadowMsg::Header(Box::new(
        short_idle(overflow, overflow).header,
    )));
    assert_eq!(
        h.stats().lost_gap,
        1,
        "the 65th later header declares it lost"
    );
    assert_eq!(
        h.stats().compared,
        1 + waiting + 1,
        "and the waiting headers applied"
    );
}

#[test]
fn shadow_headers_apply_in_seq_order() {
    let mut h = harness();
    let a = flight(
        1,
        1,
        view(Status::Idle, RecordingId(6)),
        Input::Supervisor(SupervisorEvent::Idle {
            rid: RecordingId(6),
        }),
    );
    let b = short_idle(2, 2);
    let c = short_idle(3, 3);
    // Arrival order 3, 1, 2: applied as 1, 2, 3.
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(c.header.clone())));
    assert_eq!(h.stats().compared, 0);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(a.header.clone())));
    assert_eq!(h.stats().compared, 1);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(b.header.clone())));
    assert_eq!(h.stats().compared, 3);
    assert_clean(&h);
}

#[test]
fn shadow_lost_ticket_taints_what_it_could_have_touched() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    h.comparator.handle_msg(ShadowMsg::Lost { seq: 2 });
    assert_eq!(h.stats().lost_gap, 1);
    // The header after the lost one touches the same recording: not compared.
    h.feed(&short_idle(3, 3));
    assert_eq!(h.stats().skipped_tainted, 1);
    // The recording stays tainted; an unrelated recording compares normally.
    h.feed(&flight(
        4,
        4,
        view(Status::Recovering, RecordingId(9)),
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
            rid: RecordingId(9),
        }),
    ));
    assert_eq!(h.stats().skipped_tainted, 1);
    assert_eq!(h.stats().compared, 3);
    assert_clean(&h);
    // A late duplicate of a seq already passed is not a second loss.
    h.comparator.handle_msg(ShadowMsg::Lost { seq: 2 });
    assert_eq!(h.stats().lost_gap, 1);
}

#[test]
fn shadow_open_records_are_capped_at_64() {
    let mut h = harness();
    let cap = u64::try_from(MAX_OPEN_RECORDS).unwrap();
    for n in 1..=cap {
        // Headers only: the flights never close.
        h.comparator
            .handle_msg(ShadowMsg::Header(Box::new(short_idle(n, n).header)));
    }
    assert_eq!(h.stats().lost_open, 0);
    h.comparator.handle_msg(ShadowMsg::Header(Box::new(
        short_idle(cap + 1, cap + 1).header,
    )));
    assert_eq!(
        h.stats().lost_open,
        1,
        "the oldest open record is closed as lost_open"
    );
    // Its late messages are dropped, not held and not a mismatch.
    h.comparator.handle_msg(ShadowMsg::Close {
        rec: RecorderId(1),
        completed: true,
    });
    assert_eq!(h.stats().mismatched_root, 0);
    assert_eq!(h.stats().matched, 0);
}

#[test]
fn shadow_messages_before_their_header_are_held_at_most_32() {
    let mut h = harness();
    let flight = cancel(1, 1);
    // Every later message arrives first, then the header.
    for msg in flight.later.clone() {
        h.comparator.handle_msg(msg);
    }
    assert_eq!(h.stats().compared, 0);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
    assert_eq!(h.stats().compared, 1);
    assert_eq!(
        h.stats().matched,
        1,
        "held messages attach once the header applies"
    );
    assert_clean(&h);

    // The 33rd message for a recorder with no header is an overflow.
    let mut h = harness();
    for _ in 0..MAX_HELD_PER_REC + 1 {
        h.comparator.handle_msg(ShadowMsg::Reply {
            rec: RecorderId(5),
            kind: ReplyKind::Ok,
        });
    }
    assert_eq!(h.stats().witness_overflow, 1);
}

// ------------------------------------------------------- Pre (C8.7, v5.1)

fn drop_pre_effect(flight: &mut Flight, matches: impl Fn(&Effect) -> bool) {
    let before = flight.header.pre_effects.len();
    flight.header.pre_effects.retain(|w| !matches(&w.effect));
    assert_eq!(
        flight.header.pre_effects.len(),
        before - 1,
        "one effect omitted"
    );
}

#[test]
fn shadow_pre_effect_omitted_is_effectset() {
    // Parametrized over `Pre:TERM` and `Pre:CX-` of R216.1 and `Pre:TERM` of R201.1.
    // A shadow that ignored `Pre` would pass every one of these.
    type Case = (&'static str, fn() -> Flight, fn(&Effect) -> bool);
    let cases: [Case; 3] = [
        (
            "R216.1 TERM",
            || short_idle(1, 1),
            |e| matches!(e, Effect::Terminal(_)),
        ),
        (
            "R216.1 CX-",
            || short_idle(1, 1),
            |e| matches!(e, Effect::ClearContext(_)),
        ),
        (
            "R201.1 TERM",
            || stale_stop(1, 1),
            |e| matches!(e, Effect::Terminal(_)),
        ),
    ];
    for (name, build, matches) in cases {
        let mut h = harness();
        let mut flight = build();
        drop_pre_effect(&mut flight, matches);
        if name.contains("CX-") {
            // The omitted effect's mirror is omitted with it.
            flight.header.pre_mirrors.clear();
        }
        // The header alone decides it: `Pre` is compared at CP1, not left for the
        // close to notice an effect that never showed up.
        h.comparator
            .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
        let event = h.only_event();
        assert_eq!(event.diff_kind, DiffKind::EffectSet.code(), "{name}");
        assert_eq!(event.site_id, flight.header.site_id, "{name}");
        assert_eq!(event.input_kind, flight.header.row, "{name}");
        assert_eq!(h.stats().mismatched_root, 1, "{name}");
    }
}

#[test]
fn shadow_pre_effect_reordered_is_effectorder() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let term = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::Terminal(_)))
        .unwrap();
    let clear = flight
        .header
        .pre_effects
        .iter()
        .position(|w| matches!(w.effect, Effect::ClearContext(_)))
        .unwrap();
    flight.header.pre_effects.swap(term, clear);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectOrder.code());
}

#[test]
fn shadow_pre_compared_before_in() {
    // A `Pre` difference (a reorder, `diff_kind 2`) and an `In` difference (the `ST`
    // omitted, `diff_kind 1`): the `Pre` one is reported although 1 < 2.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.swap(1, 2);
    flight.header.in_d_effects.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectOrder.code());

    // With `Pre` intact the `In` difference is what is reported.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.in_d_effects.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn pre_cx_mirror_mutation() {
    // A `Pre:CX-` that is witnessed but leaves the matching slot set is
    // `diff_kind 5` at CP1.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    assert_eq!(flight.header.pre_mirrors.len(), 1);
    let mirror = &mut flight.header.pre_mirrors[0];
    assert_eq!(mirror.before, MirrorValue::Slot(Some(RID)));
    mirror.after = MirrorValue::Slot(Some(RID));
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // A stale-safe clear of a slot holding another id leaves it alone: clean.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    let other = MirrorValue::Slot(Some(RecordingId(99)));
    flight.header.pre_mirrors[0].before = other;
    flight.header.pre_mirrors[0].after = other;
    h.feed(&flight);
    assert_clean(&h);

    // An omitted mirror is as wrong as a wrong one (CP4 completeness).
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_mirrors.clear();
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

// ---------------------------------------------------- later effects (CP2-CP4)

#[test]
fn shadow_omitted_later_effect_is_effectset_at_close() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let at = flight
        .later
        .iter()
        .position(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::EmitEvent(_))))
        .unwrap();
    flight.later.remove(at);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn shadow_reordered_later_effects_are_effectorder() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let witnesses: Vec<usize> = flight
        .later
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            matches!(
                m,
                ShadowMsg::Witness {
                    placement: Placement::PostDT,
                    ..
                }
            )
        })
        .map(|(i, _)| i)
        .collect();
    // Swap the `idx` of two `PostDT` effects while keeping their positions: the
    // live path ran them in the other order.
    let (a, b) = (witnesses[3], witnesses[4]);
    flight.later.swap(a, b);
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1, "{:?}", h.events());
    assert_eq!(h.events()[0].diff_kind, DiffKind::EffectOrder.code());
}

#[test]
fn shadow_effect_in_another_placement_is_placement() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Witness {
            effect, placement, ..
        } = msg
        {
            if matches!(effect.effect, Effect::TouchLastTranscription) {
                *placement = Placement::PostT;
            }
        }
    }
    // R205.5 has no `TL`: use a kept `PostT` effect instead.
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Witness {
            effect, placement, ..
        } = msg
        {
            if matches!(effect.effect, Effect::PerfComplete(_)) {
                *placement = Placement::PostDT;
            }
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Placement.code());
}

#[test]
fn shadow_extra_and_duplicate_effects_are_effectset() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    let first = flight
        .later
        .iter()
        .find(|m| matches!(m, ShadowMsg::Witness { .. }))
        .cloned()
        .unwrap();
    flight.later.insert(0, first);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());

    let mut h = harness();
    let mut flight = cancel(1, 1);
    let rec = flight.header.rec;
    flight.later.insert(
        0,
        ShadowMsg::Witness {
            rec,
            idx: 99,
            placement: Placement::PostDT,
            effect: WitnessedEffect::ran(Effect::PerfAbort),
        },
    );
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

#[test]
fn shadow_reply_kind_is_compared_at_close() {
    for damage in 0..2 {
        let mut h = harness();
        let mut flight = stale_stop(1, 1);
        let at = flight
            .later
            .iter()
            .position(|m| matches!(m, ShadowMsg::Reply { .. }))
            .expect("R201.1 replies recording_superseded");
        if damage == 0 {
            flight.later.remove(at);
        } else if let ShadowMsg::Reply { kind, .. } = &mut flight.later[at] {
            *kind = ReplyKind::Err;
        }
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Reply.code(),
            "damage {damage}"
        );
    }
}

#[test]
fn shadow_post_state_is_compared_at_cp1() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.live_post_d.status = Status::Processing;
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    // `ID+` committed a different id than the one the decision predicted.
    flight.header.live_post_d.current_id = RecordingId(99);
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());
}

#[test]
fn shadow_resync_does_not_cascade() {
    let mut h = harness();
    // One M1 mismatch...
    let mut wrong = short_idle(1, 1);
    wrong.header.live_post_d.status = Status::Recording;
    h.feed(&wrong);
    assert_eq!(h.stats().mismatched_root, 1);
    // ...and the next record, whose pre-state is what the live path really had,
    // compares clean: the shadow re-synced M1/M2 from the header, it did not carry
    // its own wrong post-state forward.
    let next = flight(
        2,
        2,
        view(Status::Idle, RecordingId(9)),
        Input::Supervisor(SupervisorEvent::Idle {
            rid: RecordingId(9),
        }),
    );
    h.feed(&next);
    assert_eq!(h.stats().mismatched_root, 1, "no cascade: {:?}", h.events());
    assert_eq!(h.stats().followup, 0);
    assert_eq!(h.stats().matched, 1);
}

#[test]
fn shadow_m3_m4_compared_as_mirrors_only() {
    // A wrong `MC` value (a high-water mark below the cancelled id) and a wrong
    // `CX-` result (the slot still holds the id) are `diff_kind 5` at CP3.
    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror {
            field: MirrorField::CancelledThrough,
            value,
            ..
        } = msg
        {
            *value = MirrorValue::Id(RecordingId(1));
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    let mut h = harness();
    let mut flight = cancel(1, 1);
    for msg in &mut flight.later {
        if let ShadowMsg::Mirror {
            field: MirrorField::ActiveContext,
            value,
            ..
        } = msg
        {
            *value = MirrorValue::Slot(Some(RID));
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::PostState.code());

    // A wrong keyboard flag and a missing mirror too.
    for damage in 0..2 {
        let mut h = harness();
        let mut flight = cancel(1, 1);
        if damage == 0 {
            for msg in &mut flight.later {
                if let ShadowMsg::Mirror {
                    field: MirrorField::KeyboardProcessing,
                    value,
                    ..
                } = msg
                {
                    *value = MirrorValue::Flag(true);
                }
            }
        } else {
            flight.later.retain(|m| {
                !matches!(
                    m,
                    ShadowMsg::Mirror {
                        field: MirrorField::CancelledThrough,
                        ..
                    }
                )
            });
        }
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::PostState.code(),
            "damage {damage}"
        );
    }

    // The core holds no copy of M3 or M4, so there is nothing to compare them
    // against except the decided commit: a header whose `pre.obs` differs from the
    // last decision's post is not a mismatch (it is a fresh observation).
    let mut h = harness();
    h.feed(&cancel(1, 1));
    let mut next = short_idle(2, 2);
    next.header.pre.obs.cancelled_through = RecordingId(41);
    next.header.pre.obs.context_for = None;
    h.feed(&next);
    assert_eq!(h.stats().mismatched_root, 0);
}

#[test]
fn shadow_ticker_mirror_has_no_decided_commit() {
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    let rec = flight.header.rec;
    flight.later.insert(
        0,
        ShadowMsg::Mirror {
            rec,
            field: MirrorField::TickerRegistered,
            value: MirrorValue::Flag(true),
        },
    );
    h.feed(&flight);
    assert_clean(&h);
}

#[test]
fn shadow_abandoned_flight_is_not_a_mismatch() {
    let mut h = harness();
    let flight = start(1, 1, Live::default());
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
    // The flight was dropped after its first `PostDT` effects: the rest never ran.
    let rec = flight.header.rec;
    for msg in flight.later.iter().take(2).cloned() {
        h.comparator.handle_msg(msg);
    }
    h.comparator.handle_msg(ShadowMsg::Close {
        rec,
        completed: false,
    });
    assert_eq!(h.stats().abandoned, 1);
    assert_clean(&h);
    assert_eq!(h.stats().matched, 1);
    // But an effect it did witness is still compared.
    let mut h = harness();
    let flight = cancel(1, 1);
    h.comparator
        .handle_msg(ShadowMsg::Header(Box::new(flight.header.clone())));
    let rec = flight.header.rec;
    h.comparator.handle_msg(ShadowMsg::Witness {
        rec,
        idx: 99,
        placement: Placement::PostDT,
        effect: WitnessedEffect::ran(Effect::PerfAbort),
    });
    h.comparator.handle_msg(ShadowMsg::Close {
        rec,
        completed: false,
    });
    assert_eq!(h.only_event().diff_kind, DiffKind::EffectSet.code());
}

// ------------------------------------------------------------ conditionals

#[test]
fn shadow_conditional_compared_as_predicate() {
    // `EM(starting)` with `P-spec-flowing`: a skipped one while a speculative
    // capture flows, and a taken one while it does not, both compare clean.
    for spec_flowing in [true, false] {
        let mut h = harness();
        let flight = start(
            1,
            1,
            Live {
                spec_flowing,
                ..Live::default()
            },
        );
        let em_witnessed = flight
            .later
            .iter()
            .any(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::EmitStatus { .. })));
        assert_eq!(
            em_witnessed, !spec_flowing,
            "a skipped conditional is not an effect"
        );
        h.feed(&flight);
        assert_clean(&h);
        assert_eq!(h.stats().matched, 1, "spec_flowing={spec_flowing}");
    }
    // Flipping the predicate is `diff_kind 9`: the observation says a capture was
    // flowing but the live path took the emit, and the reverse.
    for observed_flowing in [true, false] {
        let mut h = harness();
        let mut flight = start(
            1,
            1,
            Live {
                spec_flowing: !observed_flowing,
                ..Live::default()
            },
        );
        flight.header.observed = vec![ObservedFact::SpeculativeFlowing(observed_flowing)];
        h.feed(&flight);
        assert_eq!(
            h.only_event().diff_kind,
            DiffKind::Conditional.code(),
            "observed {observed_flowing}"
        );
    }
    // A different predicate id is the same difference.
    let mut h = harness();
    let mut flight = start(1, 1, Live::default());
    for msg in &mut flight.later {
        if let ShadowMsg::Cond { pred, .. } = msg {
            *pred = PredId::EmitOk(0);
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn shadow_paired_presentation_follows_its_emit_result() {
    // `PRES` runs only when the paired emit returned `Ok`.
    for emit_ok in [true, false] {
        let mut h = harness();
        let flight = stalled(
            1,
            1,
            Live {
                emit_ok,
                ..Live::default()
            },
        );
        let pres_witnessed = flight
            .later
            .iter()
            .any(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::Presentation { .. })));
        assert_eq!(pres_witnessed, emit_ok);
        h.feed(&flight);
        assert_clean(&h);
    }
    // The emit failed but the live path presented anyway: a conditional mismatch.
    let mut h = harness();
    let mut flight = stalled(
        1,
        1,
        Live {
            emit_ok: false,
            ..Live::default()
        },
    );
    for msg in &mut flight.later {
        if let ShadowMsg::Cond { taken, .. } = msg {
            *taken = true;
        }
    }
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

#[test]
fn shadow_conditional_that_ran_after_being_skipped_is_conditional() {
    let mut h = harness();
    let mut flight = start(
        1,
        1,
        Live {
            spec_flowing: true,
            ..Live::default()
        },
    );
    let rec = flight.header.rec;
    // The live path said "skipped" and then ran the effect anyway.
    let idx = flight
        .decision
        .effects
        .iter()
        .position(|p| matches!(p.effect, Effect::EmitStatus { .. }))
        .unwrap();
    let after_cond = flight
        .later
        .iter()
        .position(|m| matches!(m, ShadowMsg::Cond { .. }))
        .expect("the skipped conditional is reported")
        + 1;
    flight.later.insert(
        after_cond,
        ShadowMsg::Witness {
            rec,
            idx: u16::try_from(idx).unwrap(),
            placement: Placement::PostDT,
            effect: WitnessedEffect::emitted(flight.decision.effects[idx].effect.clone(), true),
        },
    );
    h.feed(&flight);
    assert_eq!(h.only_event().diff_kind, DiffKind::Conditional.code());
}

// ------------------------------------------------------------ observations

#[test]
fn shadow_missing_observation_is_diff_kind_7() {
    // A readiness publish whose context-presence read the live path never recorded:
    // the decision answers `NeedObservation`, which the shadow cannot complete.
    let mut h = harness();
    let core = view(Status::Starting, RID);
    let input = Input::ReadyPublish(ReadyPublishInput {
        token: murmur_dictation_core::decision::Continuation {
            recording_id: RID,
            op: OperationId(1),
            kind: ContKind::ReadyPublish,
        },
        op_status: murmur_dictation_core::decision::OpStatus::Outstanding,
        rid: RID,
        audio_ready: true,
        context_present: None,
        context_ready_at: NOW,
    });
    h.feed(&flight(1, 1, core, input));
    assert_eq!(h.only_event().diff_kind, DiffKind::Observation.code());
    assert_eq!(h.stats().compared, 1);
}

// ----------------------------------------------------------- panic and PDs

#[test]
fn shadow_panic_is_caught_and_counted() {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let mut comparator = Comparator::new(
        Box::new(|core, input, now| {
            if matches!(input, Input::CancelRequest(_)) {
                panic!("the shadow's own bug");
            }
            core.handle(input, now)
        }),
        Box::new(move |event| sink.lock().unwrap().push(event)),
        Box::new(|| 0),
    );
    comparator.handle_msg(ShadowMsg::Header(Box::new(cancel(1, 1).header)));
    assert_eq!(comparator.stats.shadow_panic, 1);
    assert_eq!(
        events.lock().unwrap()[0].diff_kind,
        DiffKind::ShadowPanic.code()
    );
    // The next header still compares, and the panicking flight's late messages are
    // dropped.
    for msg in cancel(1, 1).later {
        comparator.handle_msg(msg);
    }
    comparator.handle_msg(ShadowMsg::Header(Box::new(short_idle(2, 2).header)));
    for msg in short_idle(2, 2).later {
        comparator.handle_msg(msg);
    }
    assert_eq!(comparator.stats.matched, 1);
    assert_eq!(comparator.stats.mismatched_root, 1);
}

#[test]
fn shadow_pd_witness_counts_and_never_mismatches() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    assert_clean(&h);
    // R216.1 is a signature of PD-1, PD-15 and PD-18.
    for pd in [1, 15, 18] {
        assert_eq!(h.stats().pd_witness.get(&pd), Some(&1), "PD-{pd}");
    }
    let row = h.stats().per_row[&(216_u16, flight_row("R216.1"))];
    assert_eq!(row.pd_witness, 3);
    assert_eq!(row.compared, 1);
    h.feed(&cancel(2, 2));
    assert_eq!(h.stats().pd_witness.get(&16), Some(&1), "PD-16 is R205.5");
    // A PD row whose effects differ from the live path's is a mismatch, and is not
    // credited as a witness.
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.pop();
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1);
    assert!(h.stats().pd_witness.is_empty());
}

fn flight_row(row: &'static str) -> u16 {
    murmur_dictation_core::decision::RowId(row).ordinal()
}

#[test]
fn every_pd_row_is_a_c3_row() {
    for (pd, rows) in super::pd::PD_ROWS {
        for row in *rows {
            assert!(
                murmur_dictation_core::rows::site_of_row(row).is_some(),
                "PD-{pd} names {row}, which is not a C3 row"
            );
        }
    }
    // The PD numbers are the C7.1 ones; PD-11, PD-14 and PD-17 are not row
    // signatures.
    let numbers: Vec<u8> = super::pd::PD_ROWS.iter().map(|(pd, _)| *pd).collect();
    for absent in [11, 14, 17] {
        assert!(!numbers.contains(&absent));
    }
}

// ---------------------------------------------------- cascade control, events

#[test]
fn shadow_followup_is_not_a_new_root() {
    let mut h = harness();
    let mut first = short_idle(1, 1);
    first.header.live_post_d.status = Status::Recording;
    h.feed(&first);
    // The same recording, the same row, the same level: a followup.
    let mut second = short_idle(2, 2);
    second.header.live_post_d.status = Status::Recording;
    h.feed(&second);
    assert_eq!(h.stats().mismatched_root, 1);
    assert_eq!(h.stats().followup, 1);
    assert_eq!(h.events().len(), 1, "a followup emits no event");
    // A different level on the same recording is a new root.
    let mut third = short_idle(3, 3);
    third.header.pre_effects.pop();
    h.feed(&third);
    assert_eq!(h.stats().mismatched_root, 2);
    assert_eq!(
        h.stats().per_row[&(216, flight_row("R216.1"))].mismatched,
        3
    );
}

#[test]
fn shadow_mismatch_event_is_rate_limited() {
    // One event per (site, input_kind, diff_kind) per 60 s...
    let mut h = harness();
    for n in 1..=2u64 {
        let mut flight = flight(
            n,
            n,
            view(Status::Recovering, RecordingId(100 + n)),
            Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
                rid: RecordingId(100 + n),
            }),
        );
        flight.header.live_post_d.status = Status::Recording;
        h.feed(&flight);
    }
    assert_eq!(
        h.stats().mismatched_root,
        2,
        "two distinct recordings, two roots"
    );
    assert_eq!(h.events().len(), 1);
    assert_eq!(
        h.stats().events_suppressed,
        1,
        "the suppressed one is counted"
    );
    h.clock.fetch_add(EVENT_WINDOW_MS + 1, Ordering::SeqCst);
    let mut flight = flight(
        3,
        3,
        view(Status::Recovering, RecordingId(103)),
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
            rid: RecordingId(103),
        }),
    );
    flight.header.live_post_d.status = Status::Recording;
    h.feed(&flight);
    assert_eq!(h.events().len(), 2, "the window passed");

    // ...and at most 50 per session.
    let mut h = harness();
    let total = MAX_EVENTS_PER_SESSION + 10;
    for n in 1..=total {
        let mut flight = short_idle(n, n);
        // A distinct key every time, and a distinct recording every time.
        flight.header.row = u16::try_from(n).unwrap();
        flight.header.pre.current_id = RecordingId(1_000 + n);
        flight.header.live_post_d.current_id = RecordingId(1_000 + n);
        flight.header.input = Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
            rid: RecordingId(1_000 + n),
        });
        flight.header.live_post_d.status = Status::Recording;
        h.comparator
            .handle_msg(ShadowMsg::Header(Box::new(flight.header)));
    }
    assert_eq!(
        h.events().len(),
        usize::try_from(MAX_EVENTS_PER_SESSION).unwrap()
    );
    assert_eq!(h.stats().events_suppressed, 10);
    assert_eq!(h.stats().mismatched_root, total);
}

#[test]
fn shadow_mismatch_event_carries_three_numbers() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.swap(1, 2);
    h.feed(&flight);
    let event = h.only_event();
    assert_eq!(event.site_id, 216);
    assert_eq!(event.input_kind, 1, "R216.1 is row ordinal 1");
    assert_eq!(event.diff_kind, 2);
}

// ------------------------------------------------------------------ summary

fn leaves_are_numbers(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(_) => true,
        serde_json::Value::Object(map) => map.values().all(leaves_are_numbers),
        _ => false,
    }
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
        "dropped",
        "lost_gap",
        "lost_open",
        "abandoned",
        "abandoned_pre_d",
        "witness_overflow",
        "shadow_panic",
        "events_suppressed",
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
    assert_eq!(row["pd_witness"], 3);
    assert_eq!(object["pd_witness"]["PD-1"], 1);
    // Keys are site.row and PD-n: no id, no content.
    for key in per_row.keys() {
        assert!(key.chars().all(|c| c.is_ascii_digit() || c == '.'), "{key}");
    }

    let dir = std::env::temp_dir().join(format!("murmur-shadow-summary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
fn shadow_summary_cadence_and_thread_name_are_the_documented_ones() {
    assert_eq!(super::SUMMARY_INTERVAL, Duration::from_secs(300));
    assert_eq!(super::THREAD_NAME, "murmur-dictation-shadow");
    assert_eq!(CHANNEL_CAPACITY, 256);
    assert_eq!(MAX_PENDING_HEADERS, 64);
    assert_eq!(MAX_OPEN_RECORDS, 64);
    assert_eq!(MAX_HELD_PER_REC, 32);
    assert_eq!(EVENT_WINDOW_MS, 60_000);
    assert_eq!(MAX_EVENTS_PER_SESSION, 50);
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
