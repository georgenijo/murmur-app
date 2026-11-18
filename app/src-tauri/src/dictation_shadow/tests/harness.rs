//! The simulated live path and the comparison harness the shadow tests share.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use murmur_dictation_core::admission::Occupancy;
use murmur_dictation_core::decision::{ContKind, Continuation, Decision, OpStatus};
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::{Now, OperationId, RecordingId};
use murmur_dictation_core::inputs::{
    CancelInput, CaptureErr, CaptureResult, CaptureResultInput, DeviceSel, GestureObs,
    GuardFinishInput, GuardOrigin, Input, Origin, PipelineIdleEmitInput, PipelineResult,
    PipelineTerminal, PostKind, ProcessAudioFinishInput, ProcessAudioFinishKind,
    ProcessAudioStartInput, ReadyPublishInput, SpeculativeObs, StartInput, StopCaptureResult,
    StopCaptureResultInput, StopInput, SupervisorEvent,
};
use murmur_dictation_core::state::{DictationCore, LiveObs};
use murmur_dictation_core::status::{Status, StopTrigger};

use crate::dictation_shadow::compare::{Comparator, MismatchEvent};
use crate::dictation_shadow::summary::Stats;
use crate::dictation_shadow::transport::{
    mark, submit, witness, Mark, ShadowRecorder, Transport, QUEUE_CAPACITY,
};
use crate::dictation_shadow::types::{
    CondWitness, EffectResult, HistoryAction, HistoryTable, LiveOp, MirrorField, MirrorValue,
    MirrorWitness, ObservedFact, PredId, Projection, RecorderId, ReplyKind, ShadowHeader,
    ShadowMsg, TickerGate, WitnessedEffect,
};

pub(super) const NOW: Now = Now(0);
pub(super) const RID: RecordingId = RecordingId(7);
/// The `K:Pipeline` operation the first flight's registration draws (`1_000 + seq`).
pub(super) const OWNER_OP: OperationId = OperationId(1_001);
pub(super) const OWNER_H2_OP: OperationId = OperationId(2_001);

pub(super) type Events = Arc<Mutex<Vec<MismatchEvent>>>;

pub(super) struct Harness {
    pub comparator: Comparator,
    pub events: Events,
    pub clock: Arc<AtomicU64>,
}

pub(super) fn harness() -> Harness {
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
    pub fn stats(&self) -> &Stats {
        &self.comparator.stats
    }

    pub fn events(&self) -> Vec<MismatchEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn only_event(&self) -> MismatchEvent {
        let events = self.events();
        assert_eq!(events.len(), 1, "expected exactly one event: {events:?}");
        events[0]
    }

    pub fn send(&mut self, msg: ShadowMsg) {
        self.comparator.handle_msg(msg);
    }

    /// Only the header.
    pub fn feed_header(&mut self, flight: &Flight) {
        self.send(ShadowMsg::Header(flight.header.clone()));
    }

    pub fn feed_later(&mut self, flight: &Flight) {
        for msg in flight.later.clone() {
            self.send(msg);
        }
    }

    /// The whole flight: header, then every later message, then the close.
    pub fn feed(&mut self, flight: &Flight) {
        self.feed_header(flight);
        self.feed_later(flight);
    }
}

pub(super) fn assert_clean(h: &Harness) {
    assert_eq!(h.stats().mismatched_root, 0, "{:?}", h.events());
    assert_eq!(h.stats().followup, 0);
    assert!(h.events().is_empty(), "{:?}", h.events());
}

pub(super) fn view(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(
        status,
        current,
        LiveObs {
            cancelled_through: RecordingId(3),
            context_for: Some(RID),
        },
    )
}

/// What a faithful live path did for one decision, as messages.
#[derive(Clone)]
pub(super) struct Flight {
    pub header: ShadowHeader,
    pub later: Vec<ShadowMsg>,
    pub decision: Decision,
}

/// Playback options of the simulated live path.
#[derive(Clone, Copy)]
pub(super) struct Live {
    /// `speculative_flowing` as the live path observed it.
    pub spec_flowing: bool,
    /// Whether every emit the live path made returned `Ok`.
    pub emit_ok: bool,
    /// Whether a terminal claim won the ledger.
    pub term_won: bool,
    /// The ticker high-water mark before an `SD` ran.
    pub ticker_before: RecordingId,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            spec_flowing: false,
            emit_ok: true,
            term_won: true,
            ticker_before: RecordingId(0),
        }
    }
}

pub(super) fn flight(seq: u64, rec: u64, core: DictationCore, input: Input) -> Flight {
    flight_with(seq, rec, core, input, Live::default())
}

fn pred_of(cond: Cond) -> Option<PredId> {
    match cond {
        Cond::Always => None,
        Cond::UnlessSpeculativeFlowing => Some(PredId::SpecFlowing),
        Cond::EmitOk(p) => Some(PredId::EmitOk(p)),
    }
}

/// The live fields the simulated adapter holds while it plays a decision back.
struct Sim {
    slot: Option<RecordingId>,
    cancelled: RecordingId,
    kb: bool,
    status: Status,
    id: RecordingId,
}

impl Sim {
    /// Apply an effect that ran and return the live-field write it made, if any.
    fn run(&mut self, effect: &Effect) -> Option<MirrorWitness> {
        match effect {
            Effect::ClearContext(id) => {
                let before = self.slot;
                if self.slot == Some(*id) {
                    self.slot = None;
                }
                Some(MirrorWitness {
                    field: MirrorField::ActiveContext,
                    before: MirrorValue::Slot(before),
                    after: MirrorValue::Slot(self.slot),
                })
            }
            Effect::SetContext(id) => {
                let before = self.slot;
                self.slot = Some(*id);
                Some(MirrorWitness {
                    field: MirrorField::ActiveContext,
                    before: MirrorValue::Slot(before),
                    after: MirrorValue::Slot(self.slot),
                })
            }
            Effect::MarkCancelled(id) => {
                let before = self.cancelled;
                self.cancelled = self.cancelled.max(*id);
                Some(MirrorWitness {
                    field: MirrorField::CancelledThrough,
                    before: MirrorValue::Id(before),
                    after: MirrorValue::Id(self.cancelled),
                })
            }
            Effect::KeyboardProcessing(flag) => {
                let before = self.kb;
                self.kb = *flag;
                Some(MirrorWitness {
                    field: MirrorField::KeyboardProcessing,
                    before: MirrorValue::Flag(before),
                    after: MirrorValue::Flag(self.kb),
                })
            }
            Effect::SetStatus(to) => {
                self.status = *to;
                None
            }
            Effect::AllocId => {
                self.id = RecordingId(self.id.0 + 1);
                None
            }
            _ => None,
        }
    }

    fn gate(&self, live: &Live) -> TickerGate {
        let mut gate = TickerGate {
            id: self.id,
            status: self.status,
            current_id: self.id,
            cancelled_through: self.cancelled,
            ticker_before: live.ticker_before,
            registered: false,
        };
        gate.registered = gate.expected();
        gate
    }
}

pub(super) fn flight_with(
    seq: u64,
    rec: u64,
    core: DictationCore,
    input: Input,
    live: Live,
) -> Flight {
    let decision = core.handle(input.clone(), NOW);
    let rec_id = RecorderId(rec);
    let mut sim = Sim {
        slot: core.context_for,
        cancelled: core.cancelled_through,
        kb: false,
        status: core.status,
        id: core.current_id,
    };
    let mut emit_results: HashMap<usize, bool> = HashMap::new();
    let pre = Projection {
        status: core.status,
        current_id: core.current_id,
        obs: LiveObs {
            cancelled_through: core.cancelled_through,
            context_for: core.context_for,
        },
    };
    let carried_token = crate::dictation_shadow::compare::input_token(&input);
    let owner_finish = crate::dictation_shadow::compare::pipeline_owned_finish(&input);
    let mut header = ShadowHeader::new(
        seq,
        rec_id,
        input.site().code(),
        decision.row.ordinal(),
        pre,
        input,
        NOW,
    );
    header.add_observed(ObservedFact::SpeculativeFlowing(live.spec_flowing));
    // The continuation input retires its own token before the decision is taken
    // (`ret_if_matches`): the full token, found exactly when it was outstanding.
    if let Some((token, status)) = carried_token {
        header.add_op(LiveOp {
            table: HistoryTable::H1,
            action: HistoryAction::Retired {
                matched: status == OpStatus::Outstanding,
            },
            cont: token.kind,
            recording_id: token.recording_id,
            op: token.op,
        });
    }
    // A pipeline-owned finish retires its owner at its boundary: H1 with the owner's
    // full token (the one a `K:Pipeline` registration at `seq` 1 drew) and H2 for the
    // finishing recording.
    if let Some(rid) = owner_finish {
        for (table, op) in [
            (HistoryTable::H1, OWNER_OP),
            (HistoryTable::H2, OWNER_H2_OP),
        ] {
            header.add_op(LiveOp {
                table,
                action: HistoryAction::Retired { matched: true },
                cont: ContKind::Pipeline,
                recording_id: rid,
                op,
            });
        }
    }
    if let Some(spec) = decision.continuation {
        header.add_op(LiveOp {
            table: HistoryTable::H1,
            action: HistoryAction::Registered,
            cont: spec.kind,
            recording_id: spec.recording_id,
            op: OperationId(1_000 + seq),
        });
        // `K:Pipeline` also registers the pipeline in H2 (`PipelineOwner::register`).
        if spec.kind == ContKind::Pipeline {
            header.add_op(LiveOp {
                table: HistoryTable::H2,
                action: HistoryAction::Registered,
                cont: spec.kind,
                recording_id: spec.recording_id,
                op: OperationId(2_000 + seq),
            });
        }
    }
    let mut later = Vec::new();
    for (idx, placed) in decision.effects.iter().enumerate() {
        let effect = placed.effect.clone();
        let cond = match &effect {
            Effect::EmitStatus { cond, .. } | Effect::Presentation { cond, .. } => *cond,
            _ => Cond::Always,
        };
        let pred = pred_of(cond);
        let taken = match cond {
            Cond::Always => true,
            Cond::UnlessSpeculativeFlowing => !live.spec_flowing,
            Cond::EmitOk(p) => emit_results.get(&usize::from(p)).copied().unwrap_or(true),
        };
        let result = match &effect {
            Effect::EmitStatus { .. } | Effect::EmitEvent(_) | Effect::SmartDestinationEmit => {
                EffectResult::Emitted { ok: live.emit_ok }
            }
            Effect::Terminal(_) => EffectResult::Claimed { won: live.term_won },
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
        // The ticker gate is read where the attempt ran, before the effect's own
        // state is applied (it has none).
        let gate = (taken && effect == Effect::StartSideTasks).then(|| sim.gate(&live));
        let mirror = if taken { sim.run(&effect) } else { None };
        match placed.at {
            Placement::Pre | Placement::In => {
                let in_d = placed.at == Placement::In;
                if let Some(pred) = pred {
                    let witness = CondWitness { pred, taken };
                    if in_d {
                        assert!(header.in_d_conditionals.push(witness));
                    } else {
                        assert!(header.pre_conditionals.push(witness));
                    }
                }
                if taken {
                    if in_d {
                        assert!(header.in_d_effects.push(witnessed));
                    } else {
                        assert!(header.pre_effects.push(witnessed));
                    }
                    // Every live field an effect placed `Pre` / `In` wrote is in the
                    // header (the context slot, the cancelled-through mark, the
                    // keyboard flag).
                    if let Some(mirror) = mirror {
                        if in_d {
                            assert!(header.in_d_mirrors.push(mirror));
                        } else {
                            assert!(header.pre_mirrors.push(mirror));
                        }
                    }
                    if let (true, Some(gate)) = (in_d, gate) {
                        assert!(header.in_d_tickers.push(gate));
                    }
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
                    if let Some(mirror) = mirror {
                        later.push(ShadowMsg::Mirror {
                            rec: rec_id,
                            mirror,
                        });
                    }
                    if let Some(gate) = gate {
                        later.push(ShadowMsg::Ticker { rec: rec_id, gate });
                    }
                }
            }
        }
    }
    header.live_post_d = Projection {
        status: sim.status,
        current_id: sim.id,
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
pub(super) fn short_idle(seq: u64, rec: u64) -> Flight {
    idle_for(seq, rec, RID.0)
}

/// R216.1 for an arbitrary recording id.
pub(super) fn idle_for(seq: u64, rec: u64, id: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Recovering, RecordingId(id)),
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle {
            rid: RecordingId(id),
        }),
    )
}

fn start_input(cleanup_pending: bool) -> Input {
    Input::StartRequest(StartInput {
        origin: Origin::Hold,
        device: DeviceSel::SystemDefault,
        gesture: GestureObs::default(),
        speculative: SpeculativeObs::default(),
        kept_preview: false,
        app_disabled: false,
        fixed_and_smart: false,
        occupancy: Occupancy::default(),
        cleanup_pending: Some(cleanup_pending),
        smart_auto_result: None,
    })
}

/// R101.15: `In:ID+ ... ST`, `PDT:SPEC, EV, EM(starting; skipped while a
/// speculative capture flows), PR, CS`, and the `K:CaptureStart` continuation.
pub(super) fn start(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Idle, RecordingId(RID.0 - 1)),
        start_input(false),
        live,
    )
}

/// R101.12: a start taken while the previous attempt's cleanup is pending.
pub(super) fn start_during_cleanup(seq: u64, rec: u64) -> Flight {
    flight(seq, rec, view(Status::Idle, RID), start_input(true))
}

/// R205.5: `In:ST`, `PDT:CX-, MC, TERM, KB, EM, EV`, `PT:PC`.
pub(super) fn cancel(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Processing, RID),
        Input::CancelRequest(CancelInput { expected: None }),
    )
}

/// R214.1: `PDT:EV(RecoveryStalled)` and `PRES` paired with that emit.
pub(super) fn stalled(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Recovering, RID),
        Input::Supervisor(SupervisorEvent::RecoveryStalled { rid: RID }),
        live,
    )
}

/// R201.1: `Pre:TERM(Superseded)` for a stop that expected a stale recording.
pub(super) fn stale_stop(seq: u64, rec: u64) -> Flight {
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

fn token(kind: ContKind, op: u64) -> Continuation {
    Continuation {
        recording_id: RID,
        op: OperationId(op),
        kind,
    }
}

/// R104.2: the start's rendezvous completes: `In:ST→Recording, TL, EM, SD`.
pub(super) fn ready_publish(seq: u64, rec: u64, op_status: OpStatus) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Starting, RID),
        Input::ReadyPublish(ReadyPublishInput {
            token: token(ContKind::ReadyPublish, 500),
            op_status,
            rid: RID,
            audio_ready: true,
            context_present: Some(true),
            context_ready_at: NOW,
        }),
    )
}

/// R204a: the pipeline's own guard finishes for `rid` while `current` is the
/// recording in `status` (R204a.1 with `rid == current` while `Processing`; R204a.5c
/// for a stale `rid` while a newer one is `Recording`).
pub(super) fn pipeline_exit(
    seq: u64,
    rec: u64,
    status: Status,
    current: RecordingId,
    rid: RecordingId,
) -> Flight {
    flight(
        seq,
        rec,
        view(status, current),
        Input::GuardFinish(GuardFinishInput {
            rid,
            origin: GuardOrigin::PipelineExit,
            closure_emit: false,
        }),
    )
}

/// R302.1: the legacy import's parse failure, as either of its two inputs (the guard
/// finish with the `LegacyParse` origin, or the site-302 input).
pub(super) fn legacy_parse(seq: u64, rec: u64, as_guard: bool) -> Flight {
    let input = if as_guard {
        Input::GuardFinish(GuardFinishInput {
            rid: RID,
            origin: GuardOrigin::LegacyParse,
            closure_emit: false,
        })
    } else {
        Input::ProcessAudioFinish(ProcessAudioFinishInput {
            rid: RID,
            kind: ProcessAudioFinishKind::ParseFailed,
            closure_emit: false,
        })
    };
    flight(seq, rec, view(Status::Processing, RID), input)
}

/// R204b.6: the stop command's pipeline returned successfully for `RID`:
/// `PDT:TERM(Success)` and `PGF(Completed)`, the recording's successful outcome.
pub(super) fn pipeline_success(seq: u64, rec: u64) -> Flight {
    pipeline_success_for(seq, rec, RID)
}

/// R204b.6 for `rid`, while `RID` is the recording that is current (the tail is
/// D-less: it names its recording and reads no id).
pub(super) fn pipeline_success_for(seq: u64, rec: u64, rid: RecordingId) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Processing, RID),
        Input::PipelineIdleEmit(PipelineIdleEmitInput {
            rid,
            kind: PostKind::StopPost,
            result: Some(PipelineResult::Ok(PipelineTerminal::Success)),
            interrupted: None,
            char_count: 0,
        }),
    )
}

/// R203.9: the stop's capture returned a long recording: `PDT:PE(Vad), PS`, the
/// `K:Pipeline` continuation (and its H2 registration). Retires `K:CaptureStop`.
pub(super) fn pipeline_start(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Processing, RID),
        Input::StopCaptureResult(StopCaptureResultInput {
            token: token(ContKind::CaptureStop, 700),
            op_status: OpStatus::Outstanding,
            rid: RID,
            result: StopCaptureResult::Samples {
                n: 1_000_000,
                interrupted: None,
            },
        }),
    )
}

/// R301.5: the accepted import: `In:ID+, ST, AX`, `PT:KB, EV, EM`, and `K:Pipeline`
/// (with its H2 registration). No token to retire.
pub(super) fn import_start(seq: u64, rec: u64) -> Flight {
    flight(
        seq,
        rec,
        view(Status::Idle, RecordingId(RID.0 - 1)),
        Input::ProcessAudioStart(ProcessAudioStartInput {
            occupancy: Occupancy::default(),
            benchmark_running: Some(false),
        }),
    )
}

/// R207.3: the supervisor's `Ready` arrives for a recording that is already
/// `Recording` (PD-4): `In:TERM(Superseded)`, and the wrapper's `SD` again.
pub(super) fn ready_while_recording(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Recording, RID),
        Input::Supervisor(SupervisorEvent::Ready {
            rid: RID,
            context_present: Some(true),
            ready_received: NOW,
        }),
        live,
    )
}

/// R102.2: the start's capture failed while the recording is current.
pub(super) fn capture_failed(seq: u64, rec: u64, live: Live) -> Flight {
    flight_with(
        seq,
        rec,
        view(Status::Starting, RID),
        Input::StartCaptureResult(CaptureResultInput {
            token: token(ContKind::CaptureStart, 600),
            op_status: OpStatus::Outstanding,
            result: CaptureResult::Err(CaptureErr::Other),
        }),
        live,
    )
}

/// The same message, addressed to another recorder.
pub(super) fn retarget(msg: ShadowMsg, to: RecorderId) -> ShadowMsg {
    match msg {
        ShadowMsg::Header(mut header) => {
            header.rec = to;
            ShadowMsg::Header(header)
        }
        ShadowMsg::Witness {
            idx,
            placement,
            effect,
            ..
        } => ShadowMsg::Witness {
            rec: to,
            idx,
            placement,
            effect,
        },
        ShadowMsg::Cond { pred, taken, .. } => ShadowMsg::Cond {
            rec: to,
            pred,
            taken,
        },
        ShadowMsg::Mirror { mirror, .. } => ShadowMsg::Mirror { rec: to, mirror },
        ShadowMsg::Ticker { gate, .. } => ShadowMsg::Ticker { rec: to, gate },
        ShadowMsg::Observed { fact, .. } => ShadowMsg::Observed { rec: to, fact },
        ShadowMsg::TickerSkipped { target, reason, .. } => ShadowMsg::TickerSkipped {
            rec: to,
            target,
            reason,
        },
        ShadowMsg::History { op, .. } => ShadowMsg::History { rec: to, op },
        ShadowMsg::Reply { kind, .. } => ShadowMsg::Reply { rec: to, kind },
        ShadowMsg::Close { completed, .. } => ShadowMsg::Close { rec: to, completed },
        ShadowMsg::PreAbandoned { .. } => ShadowMsg::PreAbandoned { rec: to },
    }
}

/// The flight as one whose ticket was taken when the transport's `lost` count was
/// `epoch` (the comparator admits a header only if its epoch is at least the count it
/// last acknowledged).
pub(super) fn at_epoch(mut flight: Flight, epoch: u64) -> Flight {
    flight.header.epoch = epoch;
    flight
}

pub(super) fn local_transport(capacity: usize) -> Arc<Transport> {
    Transport::new(capacity)
}

pub(super) fn default_transport() -> Arc<Transport> {
    Transport::new(QUEUE_CAPACITY)
}

/// Send a flight's header through the real producer path under a ticket of
/// `recorder` (the transport assigns `seq`). Returns the assigned `seq`.
pub(super) fn submit_header(recorder: &ShadowRecorder, flight: &Flight) -> u64 {
    recorder.enter_pre();
    let ticket = recorder.ticket(flight.header.site_id);
    let seq = ticket.seq();
    let mut header = flight.header.clone();
    header.seq = seq;
    header.rec = recorder.id();
    submit(ticket, header);
    seq
}

/// Send a flight's later messages through the real producer path, ending with
/// `finish`.
pub(super) fn send_later(recorder: &ShadowRecorder, flight: &Flight) {
    for msg in flight.later.clone() {
        match msg {
            ShadowMsg::Witness {
                idx,
                placement,
                effect,
                ..
            } => witness(recorder, idx, placement, effect),
            ShadowMsg::Cond { pred, taken, .. } => mark(recorder, Mark::Cond { pred, taken }),
            ShadowMsg::Mirror { mirror, .. } => mark(recorder, Mark::Mirror(mirror)),
            ShadowMsg::Ticker { gate, .. } => mark(recorder, Mark::Ticker(gate)),
            ShadowMsg::Observed { fact, .. } => mark(recorder, Mark::Observed(fact)),
            ShadowMsg::TickerSkipped { target, reason, .. } => {
                mark(recorder, Mark::TickerSkipped { target, reason });
            }
            ShadowMsg::History { op, .. } => mark(recorder, Mark::History(op)),
            ShadowMsg::Reply { kind, .. } => mark(recorder, Mark::Reply(kind)),
            ShadowMsg::Close { .. } => recorder.finish(),
            other => panic!("unexpected {other:?}"),
        }
    }
}

/// The index of the first later `Witness` whose effect satisfies `matches`.
pub(super) fn witness_at(flight: &Flight, matches: impl Fn(&Effect) -> bool) -> usize {
    flight
        .later
        .iter()
        .position(|m| matches!(m, ShadowMsg::Witness { effect, .. } if matches(&effect.effect)))
        .expect("the flight has such a witness")
}
