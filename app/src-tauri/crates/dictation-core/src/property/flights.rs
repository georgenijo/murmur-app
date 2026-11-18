//! The T-holder commands (start, stop, cancel) and the tasks they spawn (the
//! pipeline, the partial ticker), each as a step machine over the real
//! `decide_*` functions. The order of sections and the placement of each
//! effect follow the shell (`commands/recording.rs`) and the C3 tables.

use super::model::*;
use super::supervisor::ReplyPoll;
use super::task::*;
use crate::admission::Occupancy;
use crate::cancel as core_cancel;
use crate::decision::{ContKind, Continuation, ObsKind, OpStatus, SiteId, Verdict};
use crate::effects::{Effect, Placement};
use crate::ids::{Now, RecordingId};
use crate::inputs::{
    CancelCaptureResultInput, CancelInput, CancelOrigin, CaptureErr, CaptureResult,
    CaptureResultInput, DeviceSel, GestureObs, GuardOrigin, InterruptedMeta, Origin,
    PipelineTerminal, SpeculativeObs, StartContextInput, StartInput, StopCaptureResult,
    StopCaptureResultInput, StopContextInput, StopInput,
};
use crate::start as core_start;
use crate::status::{Status, StopTrigger};
use crate::stop as core_stop;
use std::collections::VecDeque;

fn now(cx: &Cx<'_>) -> Now {
    Now(u64::from(cx.w.step))
}

fn fx(cx: &Cx<'_>, rid: Rid, row: &'static str, site: SiteId) -> FxCtx {
    FxCtx {
        tid: cx.tid,
        rid,
        row,
        site,
    }
}

/// Occupancy facts a start reads under D (the blockers and the status).
#[derive(Clone, Copy, Debug, Default)]
pub struct Blockers {
    pub relaunch: bool,
    pub meeting: bool,
    pub file: bool,
    pub benchmark: bool,
    pub corpus: bool,
}

// ------------------------------------------------------------------ start

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ss {
    AcquireT,
    Pre,
    D101,
    WaitAck,
    Site102,
    Fail102,
    SmartAwait,
    SmartRead,
    SmartSde,
    D103,
    ReadyRead,
    D104,
    FlightEnd,
    Finish,
}

pub struct StartFlight {
    st: Ss,
    q: VecDeque<QItem>,
    plan: AttemptPlan,
    blockers: Blockers,
    app_disabled: bool,
    smart_enabled: bool,
    /// Whether the scheduler may drop the future at its smart-capture await.
    rid: Rid,
    cap: Option<Continuation>,
    ctx: Option<Continuation>,
    ready: Option<Continuation>,
    answer: Option<super::supervisor::StartAnswer>,
    audio_ready: bool,
    capture_current: bool,
    dropped: bool,
}

impl StartFlight {
    pub fn new(
        plan: AttemptPlan,
        blockers: Blockers,
        app_disabled: bool,
        smart_enabled: bool,
    ) -> Self {
        Self {
            st: Ss::AcquireT,
            q: VecDeque::new(),
            plan,
            blockers,
            app_disabled,
            smart_enabled,
            rid: 0,
            cap: None,
            ctx: None,
            ready: None,
            answer: None,
            audio_ready: false,
            capture_current: false,
            dropped: false,
        }
    }

    fn input(&self, cx: &Cx<'_>) -> StartInput {
        StartInput {
            origin: Origin::Toggle,
            device: DeviceSel::SystemDefault,
            gesture: GestureObs::default(),
            speculative: SpeculativeObs::default(),
            kept_preview: false,
            app_disabled: self.app_disabled,
            fixed_and_smart: false,
            occupancy: Occupancy {
                status: cx.w.status,
                relaunch_committed: self.blockers.relaunch,
                meeting_active: self.blockers.meeting,
                file_transcribing: self.blockers.file,
                benchmark_running: self.blockers.benchmark,
                corpus_active: self.blockers.corpus,
                ..Occupancy::default()
            },
            cleanup_pending: None,
            smart_auto_result: None,
        }
    }

    fn retire_tokens(&mut self, cx: &mut Cx<'_>) {
        for token in [self.cap.take(), self.ctx.take(), self.ready.take()]
            .into_iter()
            .flatten()
        {
            cx.w.retire_op(token);
        }
    }

    /// A refusal or a flush: the tail runs after T is released.
    fn leave(&mut self, cx: &mut Cx<'_>, decision: &crate::decision::Decision, site: SiteId) {
        release_t(cx);
        let ctx = fx(cx, self.rid, decision.row.0, site);
        queue_phase(&mut self.q, ctx, decision, Placement::PostDT);
        queue_phase(&mut self.q, ctx, decision, Placement::PostT);
        self.st = Ss::Finish;
    }
}

impl Queued for StartFlight {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, cx: &mut Cx<'_>, _fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        match effect {
            Effect::CaptureStart(id) => {
                // `send_start_request`: the client enqueues; a busy supervisor never
                // blocks it (D-S2b-9 (a)).
                cx.w.sup_send_start(id.0, cx.tid, self.plan.clone());
                Ok(())
            }
            other => unreachable!("a start flight does not run {other:?}"),
        }
    }
}

impl Task for StartFlight {
    fn name(&self) -> &'static str {
        "start"
    }

    fn droppable(&self) -> bool {
        self.st == Ss::SmartAwait && !self.dropped
    }

    fn drop_now(&mut self, cx: &mut Cx<'_>) {
        // A start future dropped at its await: the guards' `Drop` retire the ops, T
        // is released, nothing else runs (PD-9: the `PR` row has no `PB`).
        self.dropped = true;
        cx.out.notes.push(Note::StartDropped {
            rid: self.rid,
            registered: cx.w.perf.registered.contains(&self.rid),
            begun: cx.w.perf.begun.contains(&self.rid),
        });
        self.q.clear();
        self.retire_tokens(cx);
        release_t(cx);
        self.st = Ss::Finish;
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if let Some(poll) = drain_one(self, cx) {
            return poll;
        }
        let at = now(cx);
        match self.st {
            Ss::AcquireT => {
                if !acquire_t(cx) {
                    return Poll::Blocked;
                }
                self.st = Ss::Pre;
            }
            Ss::Pre => {
                let input = self.input(cx);
                match core_start::decide_start_request_pre(&input, at) {
                    Some(pre) => {
                        log_decision(cx, SiteId::StartRequest, &pre, cx.w.current_id, None);
                        self.rid = cx.w.current_id;
                        self.leave(cx, &pre, SiteId::StartRequest);
                    }
                    None => self.st = Ss::D101,
                }
            }
            Ss::D101 => {
                let view = cx.w.view();
                let mut input = self.input(cx);
                let mut rounds = 0;
                let decision = loop {
                    let decision = core_start::decide_start_request(&view, input.clone(), at);
                    rounds += 1;
                    match decision.verdict {
                        Verdict::NeedObservation(ObsKind::CleanupPending { id }) if rounds == 1 => {
                            input.cleanup_pending = Some(cx.w.sup.cleanup_pending(id.0));
                        }
                        Verdict::NeedObservation(kind) => {
                            unreachable!("a start asks once for the cleanup check, not {kind:?}")
                        }
                        _ => break decision,
                    }
                };
                self.rid = cx.w.current_id;
                log_decision(cx, SiteId::StartRequest, &decision, self.rid, None);
                match decision.row.0 {
                    "R101.15" => {
                        // SP9: an admitted start implies Idle, no blocker and no
                        // cleanup pending (C6.2 #1), at admission time under D+T.
                        let blocked = self.blockers.relaunch
                            || self.blockers.meeting
                            || self.blockers.file
                            || self.blockers.benchmark
                            || self.blockers.corpus;
                        if cx.w.status != Status::Idle
                            || blocked
                            || cx.w.sup.cleanup_pending(cx.w.current_id)
                        {
                            cx.out.violations.push(format!(
                                "SP9: a start was admitted with status {:?}, blocked {blocked}, \
                                 cleanup pending {}",
                                cx.w.status,
                                cx.w.sup.cleanup_pending(cx.w.current_id)
                            ));
                        }
                        let spec = decision
                            .continuation
                            .expect("R101.15 registers K:CaptureStart");
                        self.cap = Some(cx.w.register(spec));
                        let ctx = fx(cx, cx.w.current_id + 1, "R101.15", SiteId::StartRequest);
                        // `AllocId` is the first `In` effect: apply it, then bind the id.
                        let mut first = true;
                        for placed in by_placement(&decision, Placement::In) {
                            let bound = if first {
                                cx.w.current_id + 1
                            } else {
                                cx.w.current_id
                            };
                            let effect_ctx = FxCtx { rid: bound, ..ctx };
                            apply(cx, &effect_ctx, Placement::In, &placed.effect);
                            first = false;
                        }
                        self.rid = cx.w.current_id;
                        let bound = fx(cx, self.rid, "R101.15", SiteId::StartRequest);
                        queue_phase(&mut self.q, bound, &decision, Placement::PostDT);
                        self.st = Ss::WaitAck;
                    }
                    "R101.12" => {
                        let ctx = fx(cx, cx.w.current_id, "R101.12", SiteId::StartRequest);
                        apply_in(cx, &ctx, &decision);
                        self.leave(cx, &decision, SiteId::StartRequest);
                    }
                    _ => self.leave(cx, &decision, SiteId::StartRequest),
                }
            }
            Ss::WaitAck => {
                // The supervisor's response (`SupStep::StartAnswer`, a refusal), or
                // the receiver's timeout (`await_start_response`, 3065-3079).
                let answer = match cx.w.sup_poll_reply(cx.tid) {
                    ReplyPoll::Waiting => return Poll::Progress,
                    ReplyPoll::Ready(_) => {
                        cx.w.sup
                            .start_answer(self.rid)
                            .expect("a delivered start response has its value")
                    }
                    ReplyPoll::TimedOut => {
                        cx.w.sup_start_timed_out(self.rid, cx.tid);
                        super::supervisor::StartAnswer::Failed(CaptureErr::Other)
                    }
                };
                self.answer = Some(answer);
                self.st = if matches!(self.answer, Some(super::supervisor::StartAnswer::Started)) {
                    Ss::Site102
                } else {
                    Ss::Fail102
                };
                if self.st == Ss::Fail102 {
                    // R102.2-.6: `Pre:PA, Pre:CX-` run before D.
                    let ctx = fx(cx, self.rid, "R102.pre", SiteId::StartCaptureResult);
                    for placed in
                        core_start::start_capture_failure_pre_effects(RecordingId(self.rid))
                    {
                        self.q.push_back(QItem::Fx(ctx, placed));
                    }
                }
            }
            Ss::Site102 => {
                // R102.1 (D-less): the capture started.
                let token = self.cap.expect("a started capture has its token");
                let decision = consume(
                    cx,
                    token,
                    SiteId::StartCaptureResult,
                    self.rid,
                    move |w, op_status| {
                        core_start::decide_start_capture_result_nolock(
                            w.facts(),
                            CaptureResultInput {
                                token,
                                op_status,
                                result: CaptureResult::Ok,
                            },
                            at,
                        )
                    },
                );
                self.cap = None;
                log_decision(cx, SiteId::StartCaptureResult, &decision, self.rid, None);
                if let Some(spec) = decision.continuation {
                    self.ctx = Some(cx.w.register(spec));
                }
                self.st = if self.smart_enabled {
                    Ss::SmartAwait
                } else {
                    Ss::D103
                };
            }
            Ss::Fail102 => {
                let token = self.cap.expect("a failed start has its token");
                let error = match self.answer.clone() {
                    Some(super::supervisor::StartAnswer::Failed(error)) => error,
                    _ => CaptureErr::Other,
                };
                let rid = self.rid;
                let decision = consume(
                    cx,
                    token,
                    SiteId::StartCaptureResult,
                    rid,
                    move |w, op_status| {
                        core_start::decide_start_capture_result(
                            &w.view(),
                            CaptureResultInput {
                                token,
                                op_status,
                                result: CaptureResult::Err(error.clone()),
                            },
                            at,
                        )
                    },
                );
                let decision = after_pre(cx, decision);
                self.cap = None;
                log_decision(cx, SiteId::StartCaptureResult, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::StartCaptureResult);
                apply_in(cx, &ctx, &decision);
                release_t(cx);
                self.st = Ss::Finish;
            }
            Ss::SmartAwait => {
                // The smart-destination capture's await: the one point where the
                // future can be parked (or dropped) while it holds T.
                self.st = Ss::SmartRead;
            }
            Ss::SmartRead => {
                // Query 402 (`q_capture_current`): a short D read.
                self.capture_current = cx.w.view().q_capture_current(RecordingId(self.rid));
                self.st = if self.capture_current {
                    Ss::SmartSde
                } else {
                    Ss::D103
                };
            }
            Ss::SmartSde => {
                // R103.0: the emit's own D section rechecks ownership.
                let view = cx.w.view();
                let decision =
                    core_start::decide_start_destination(&view, RecordingId(self.rid), true, at);
                log_decision(cx, SiteId::StartContext, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::StartContext);
                apply_in(cx, &ctx, &decision);
                self.st = Ss::D103;
            }
            Ss::D103 => {
                let token = self.ctx.expect("a started capture has its context token");
                let decision = consume(
                    cx,
                    token,
                    SiteId::StartContext,
                    self.rid,
                    move |w, op_status| {
                        core_start::decide_start_context(
                            &w.view(),
                            StartContextInput { token, op_status },
                            at,
                        )
                    },
                );
                self.ctx = None;
                log_decision(cx, SiteId::StartContext, &decision, self.rid, None);
                if let Some(spec) = decision.continuation {
                    self.ready = Some(cx.w.register(spec));
                }
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::StartContext);
                apply_in(cx, &ctx, &decision);
                match decision.row.0 {
                    "R103.1" => self.st = Ss::ReadyRead,
                    "R103.2" | "R103.3" => self.leave(cx, &decision, SiteId::StartContext),
                    _ => {
                        // R103.4-.6: `PDT` effects with T held, then T released.
                        queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                        self.q.push_back(QItem::ReleaseT);
                        queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                        self.st = Ss::Finish;
                    }
                }
            }
            Ss::ReadyRead => {
                // The supervisor read before D (5306).
                self.audio_ready = cx.w.sup.audio_ready(self.rid);
                if self.audio_ready {
                    self.st = Ss::D104;
                } else {
                    // R104.1 (D-less).
                    let token = self.ready.expect("the context install registered it");
                    let rid = self.rid;
                    let decision =
                        consume(cx, token, SiteId::ReadyPublish, rid, move |w, op_status| {
                            core_start::decide_ready_publish_nolock(
                                w.facts(),
                                crate::inputs::ReadyPublishInput {
                                    token,
                                    op_status,
                                    rid: RecordingId(rid),
                                    audio_ready: false,
                                    context_present: None,
                                    context_ready_at: at,
                                },
                                at,
                            )
                        });
                    self.ready = None;
                    log_decision(cx, SiteId::ReadyPublish, &decision, self.rid, None);
                    self.st = Ss::FlightEnd;
                }
            }
            Ss::D104 => {
                let token = self.ready.expect("the context install registered it");
                let rid = self.rid;
                let decision =
                    consume(cx, token, SiteId::ReadyPublish, rid, move |w, op_status| {
                        let view = w.view();
                        let mut input = crate::inputs::ReadyPublishInput {
                            token,
                            op_status,
                            rid: RecordingId(rid),
                            audio_ready: true,
                            context_present: None,
                            context_ready_at: at,
                        };
                        let mut rounds = 0;
                        loop {
                            let decision = core_start::decide_ready_publish(&view, input, at);
                            rounds += 1;
                            match decision.verdict {
                                Verdict::NeedObservation(ObsKind::ContextPresence { id })
                                    if rounds == 1 =>
                                {
                                    input.context_present = Some(w.context_for == Some(id.0));
                                }
                                _ => break decision,
                            }
                        }
                    });
                self.ready = None;
                log_decision(cx, SiteId::ReadyPublish, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::ReadyPublish);
                apply_in(cx, &ctx, &decision);
                self.st = Ss::FlightEnd;
            }
            Ss::FlightEnd => {
                // R104.7: `PDT:PM`, T released, `PT:PB`, `PT:REL`.
                let decision = core_start::decide_start_flight_end(RecordingId(self.rid), at);
                log_decision(cx, SiteId::ReadyPublish, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::ReadyPublish);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.q.push_back(QItem::ReleaseT);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                self.st = Ss::Finish;
            }
            Ss::Finish => {
                self.retire_tokens(cx);
                release_t(cx);
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

// ------------------------------------------------------------------- stop

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ts {
    AcquireT,
    Entry,
    D201,
    CtxRead,
    D202,
    AfterCp,
    D203,
    PanicGuard,
    AwaitPipeline,
    D204b,
    Tail,
    Site206,
    Finish,
}

#[derive(Clone, Copy, Debug)]
pub struct StopCfg {
    /// The stop task panics once the capture stop returned: its `IdleGuard`'s
    /// `Drop` writes the status (PD-13) and no terminal is emitted.
    pub panic_after_cp: bool,
    /// The pipeline the stop starts (if the capture is long enough).
    pub pipeline: PipelineCfg,
}

pub struct StopFlight {
    st: Ts,
    q: VecDeque<QItem>,
    expected: Option<Rid>,
    auto: Option<Continuation>,
    /// This is the stop the `Interrupted` arm spawned (its `AutoStop` token was held).
    is_auto: bool,
    cfg: StopCfg,
    rid: Rid,
    input: Option<StopInput>,
    interrupted: Option<super::model::Preserved>,
    op: Option<Continuation>,
    stop: Option<Continuation>,
    pipeline_op: Option<Continuation>,
    cp: Option<Result<usize, ()>>,
    cc: Option<Result<bool, ()>>,
    reply: Option<PipelineReply>,
    panicked: bool,
}

impl StopFlight {
    pub fn new(expected: Option<Rid>, auto: Option<Continuation>, cfg: StopCfg) -> Self {
        Self {
            st: Ts::AcquireT,
            q: VecDeque::new(),
            expected,
            is_auto: auto.is_some(),
            auto,
            cfg,
            rid: 0,
            input: None,
            interrupted: None,
            op: None,
            stop: None,
            pipeline_op: None,
            cp: None,
            cc: None,
            reply: None,
            panicked: false,
        }
    }

    fn input_ref(&mut self) -> &mut StopInput {
        self.input.as_mut().expect("entry built the input")
    }

    fn retire_tokens(&mut self, cx: &mut Cx<'_>) {
        for token in [self.auto.take(), self.op.take(), self.stop.take()]
            .into_iter()
            .flatten()
        {
            cx.w.retire_op(token);
        }
    }
}

fn meta_of(preserved: super::model::Preserved) -> InterruptedMeta {
    InterruptedMeta {
        samples: preserved.samples,
        reason_code: preserved.code,
        duration_ms: preserved.samples as u64 / 16,
    }
}

impl Queued for StopFlight {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, cx: &mut Cx<'_>, fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        match effect {
            Effect::CaptureStop(id) => {
                self.cp = Some(match self.interrupted {
                    // An interrupted capture's preserved samples bypass the
                    // supervisor (`arm_capture_stop`).
                    Some(preserved) => Ok(preserved.samples),
                    None => {
                        // `stop_with_trigger`: the client enqueues and waits; the
                        // reply, the receiver's timeout and the continuation are
                        // later events (`AfterCp`).
                        cx.w.sup_send_stop(id.0, cx.tid);
                        return Ok(());
                    }
                });
                Ok(())
            }
            Effect::CaptureCancel { id, .. } => {
                cx.w.sup_send_cancel(id.0, cx.tid);
                Ok(())
            }
            Effect::PipelineSpawn(id) => {
                let op = self.pipeline_op.take();
                let cfg = self.cfg.pipeline;
                cx.spawn
                    .push(Box::new(Pipeline::new(id.0, cx.tid, op, cfg)));
                let _ = fx;
                Ok(())
            }
            other => unreachable!("a stop does not run {other:?}"),
        }
    }
}

impl Task for StopFlight {
    fn name(&self) -> &'static str {
        "stop"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if let Some(poll) = drain_one(self, cx) {
            return poll;
        }
        let at = now(cx);
        match self.st {
            Ts::AcquireT => {
                if !acquire_t(cx) {
                    return Poll::Blocked;
                }
                self.st = Ts::Entry;
            }
            Ts::Entry => {
                // Stop admission (T held): the `AutoStop` op is consumed here.
                if let Some(token) = self.auto.take() {
                    cx.w.retire(token);
                }
                let current = cx.w.current_id;
                self.rid = current;
                let input = StopInput {
                    expected: self.expected.map(RecordingId),
                    trigger: StopTrigger::User,
                    observed_current_id: RecordingId(current),
                    interrupted: None,
                    entered: at,
                };
                if input.is_stale() {
                    // R201.1: T held, no D; the stale path takes the interrupted
                    // capture first (D-S1-3, PD-15).
                    let stale = self.expected.expect("a stale stop has an expectation");
                    let mut input = input;
                    let first = core_stop::decide_stop_request_stale(input, at).expect("stale");
                    if matches!(
                        first.verdict,
                        Verdict::NeedObservation(ObsKind::TakeInterrupted { .. })
                    ) {
                        let earlier = cx.w.taken_by.get(&stale).map(|row| (*row,));
                        let taken = cx.w.take_interrupted(stale, "R201.1");
                        cx.out.notes.push(Note::Take {
                            tid: cx.tid,
                            rid: stale,
                            row: "R201.1",
                            found: taken.is_some(),
                            earlier,
                        });
                        input.interrupted = Some(taken.map(meta_of));
                    }
                    let decision = core_stop::decide_stop_request_stale(input, at).expect("stale");
                    log_decision(cx, SiteId::StopRequest, &decision, stale, None);
                    let ctx = fx(cx, stale, decision.row.0, SiteId::StopRequest);
                    queue_phase(&mut self.q, ctx, &decision, Placement::Pre);
                    // The shell holds T through `Pre` and the return (recording.rs
                    // 7500): the release is queued behind the effects.
                    self.q.push_back(QItem::ReleaseT);
                    self.st = Ts::Finish;
                } else {
                    self.input = Some(input);
                    self.st = Ts::D201;
                }
            }
            Ts::D201 => {
                let view = cx.w.view();
                let mut input = *self.input_ref();
                let mut decision = core_stop::decide_stop_request(&view, input, at);
                if let Verdict::NeedObservation(ObsKind::TakeInterrupted { id }) = decision.verdict
                {
                    let earlier = cx.w.taken_by.get(&id.0).map(|row| (*row,));
                    let taken = cx.w.take_interrupted(id.0, "R201.5");
                    cx.out.notes.push(Note::Take {
                        tid: cx.tid,
                        rid: id.0,
                        row: "R201.5",
                        found: taken.is_some(),
                        earlier,
                    });
                    self.interrupted = taken;
                    input.interrupted = Some(taken.map(meta_of));
                    *self.input_ref() = input;
                    decision = core_stop::decide_stop_request(&view, input, at);
                }
                log_decision(cx, SiteId::StopRequest, &decision, self.rid, None);
                if self.is_auto && self.interrupted.is_none() {
                    cx.out.notes.push(Note::AutoStopTookNothing {
                        rid: self.expected.unwrap_or(self.rid),
                    });
                }
                match decision.verdict {
                    Verdict::Applied => {}
                    _ => {
                        release_t(cx);
                        self.st = Ts::Finish;
                        return Poll::Progress;
                    }
                }
                let cancelling = decision
                    .continuation
                    .is_some_and(|spec| spec.kind == ContKind::CaptureCancel);
                self.op = decision.continuation.map(|spec| cx.w.register(spec));
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::StopRequest);
                apply_in(cx, &ctx, &decision);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                if cancelling {
                    // R201.4: `PT:CC` after T is released, then site 206.
                    self.q.push_back(QItem::ReleaseT);
                    queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                    self.st = Ts::Site206;
                } else {
                    self.st = Ts::CtxRead;
                }
            }
            Ts::Site206 => {
                let rid = self.rid;
                let result = match self.cc {
                    Some(result) => result,
                    None => match cx.w.sup_poll_reply(cx.tid) {
                        ReplyPoll::Waiting => return Poll::Progress,
                        ReplyPoll::Ready(ReplyVal::Cancel(result)) => {
                            self.cc = Some(result);
                            result
                        }
                        ReplyPoll::TimedOut => {
                            self.cc = Some(Err(()));
                            Err(())
                        }
                        ReplyPoll::Ready(ReplyVal::Stop(_) | ReplyVal::Start) => {
                            unreachable!("a cancel waits for a cancel reply")
                        }
                    },
                };
                let token = self.op.take().expect("R201.4 registered K:CaptureCancel");
                let decision = consume(
                    cx,
                    token,
                    SiteId::CancelCaptureResult,
                    self.rid,
                    move |_, op_status| {
                        core_cancel::decide_cancel_capture_result_nolock(
                            crate::state::LockFreeFacts::default(),
                            CancelCaptureResultInput {
                                token,
                                op_status,
                                rid: RecordingId(rid),
                                result,
                                origin: CancelOrigin::Stop,
                            },
                            at,
                        )
                    },
                );
                log_decision(cx, SiteId::CancelCaptureResult, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::CancelCaptureResult);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                self.st = Ts::Finish;
            }
            Ts::CtxRead => {
                // The `[pre]` context read with T held and no D (6000).
                let present = cx.w.context_for == Some(self.rid);
                let token = self.op.expect("an accepted stop registered K:StopContext");
                let interrupted_error = self.interrupted.map(|preserved| preserved.code);
                let rid = self.rid;
                if present {
                    let decision = consume(
                        cx,
                        token,
                        SiteId::StopContextNoLock,
                        rid,
                        move |w, op_status| {
                            core_stop::decide_stop_context_nolock(
                                w.facts(),
                                StopContextInput {
                                    token,
                                    op_status,
                                    rid: RecordingId(rid),
                                    context_present: true,
                                    interrupted_error,
                                },
                                at,
                            )
                        },
                    );
                    self.op = None;
                    log_decision(cx, SiteId::StopContextNoLock, &decision, rid, None);
                    if let Some(spec) = decision.continuation {
                        self.stop = Some(cx.w.register(spec));
                    }
                    let ctx = fx(cx, rid, decision.row.0, SiteId::StopContextNoLock);
                    queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                    // The `IdleGuard` exists from here; T is released; `PT:CP`.
                    self.q.push_back(QItem::ReleaseT);
                    queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                    self.st = Ts::AfterCp;
                } else {
                    // R202.3-.5: `Pre:TERM(MissingContext)` before D.
                    let ctx = fx(cx, rid, "R202.pre", SiteId::StopContext);
                    for placed in core_stop::stop_context_pre_effects() {
                        self.q.push_back(QItem::Fx(ctx, placed));
                    }
                    self.st = Ts::D202;
                }
            }
            Ts::D202 => {
                let token = self.op.expect("an accepted stop registered K:StopContext");
                let rid = self.rid;
                let interrupted_error = self.interrupted.map(|preserved| preserved.code);
                let decision = consume(cx, token, SiteId::StopContext, rid, move |w, op_status| {
                    core_stop::decide_stop_context(
                        &w.view(),
                        StopContextInput {
                            token,
                            op_status,
                            rid: RecordingId(rid),
                            context_present: false,
                            interrupted_error,
                        },
                        at,
                    )
                });
                let decision = after_pre(cx, decision);
                self.op = None;
                log_decision(cx, SiteId::StopContext, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::StopContext);
                apply_in(cx, &ctx, &decision);
                release_t(cx);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.st = Ts::Finish;
            }
            Ts::AfterCp => {
                let result = match self.cp {
                    Some(result) => result,
                    None => match cx.w.sup_poll_reply(cx.tid) {
                        ReplyPoll::Waiting => return Poll::Progress,
                        ReplyPoll::Ready(ReplyVal::Stop(result)) => {
                            self.cp = Some(result);
                            result
                        }
                        ReplyPoll::TimedOut => {
                            self.cp = Some(Err(()));
                            Err(())
                        }
                        ReplyPoll::Ready(ReplyVal::Cancel(_) | ReplyVal::Start) => {
                            unreachable!("a stop waits for a stop reply")
                        }
                    },
                };
                if self.cfg.panic_after_cp {
                    // The stop task unwinds: `IdleGuard::drop` finishes with the
                    // `Dropped` origin (R204a.6; PD-13), the ops retire by `Drop`.
                    self.panicked = true;
                    cx.out.notes.push(Note::TaskPanicked {
                        tid: cx.tid,
                        rid: self.rid,
                    });
                    let ctx = fx(cx, self.rid, "R204a.pre", SiteId::GuardFinish);
                    for placed in core_stop::guard_finish_pre_effects(RecordingId(self.rid)) {
                        self.q.push_back(QItem::Fx(ctx, placed));
                    }
                    self.st = Ts::PanicGuard;
                    return Poll::Progress;
                }
                let token = self
                    .stop
                    .expect("the stop context registered K:CaptureStop");
                let interrupted = self.interrupted.map(|preserved| preserved.code);
                let rid = self.rid;
                let capture = match result {
                    Err(()) => StopCaptureResult::Err,
                    Ok(n) => StopCaptureResult::Samples { n, interrupted },
                };
                let probe = StopCaptureResultInput {
                    token,
                    op_status: OpStatus::Outstanding,
                    rid: RecordingId(rid),
                    result: capture,
                };
                if matches!(result, Ok(n) if n >= core_stop::MIN_RECORDING_SAMPLES) {
                    // R203.9/.10 (D-less): the pipeline starts whatever status and
                    // id are now. H2 reg and H1 reg `op_pl` come with it.
                    let decision = consume(
                        cx,
                        token,
                        SiteId::StopCaptureResult,
                        rid,
                        move |w, op_status| {
                            core_stop::decide_stop_capture_result_nolock(
                                w.facts(),
                                StopCaptureResultInput { op_status, ..probe },
                                at,
                            )
                        },
                    );
                    self.stop = None;
                    cx.out.input_code = interrupted;
                    log_decision(cx, SiteId::StopCaptureResult, &decision, rid, None);
                    if let Some(spec) = decision.continuation {
                        cx.w.history.pipelines.insert(rid);
                        self.pipeline_op = Some(cx.w.register(spec));
                    }
                    let ctx = fx(cx, rid, decision.row.0, SiteId::StopCaptureResult);
                    queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                    self.st = Ts::AwaitPipeline;
                } else {
                    // R203.1-.8: `Pre` effects run before D.
                    let ctx = fx(cx, rid, "R203.pre", SiteId::StopCaptureResult);
                    for placed in core_stop::stop_capture_pre_effects(&probe) {
                        self.q.push_back(QItem::Fx(ctx, placed));
                    }
                    self.st = Ts::D203;
                }
            }
            Ts::PanicGuard => {
                let view = cx.w.view();
                let rid = self.rid;
                let decision = core_stop::decide_guard_finish(
                    &view,
                    crate::inputs::GuardFinishInput {
                        rid: RecordingId(rid),
                        origin: GuardOrigin::Dropped,
                        closure_emit: false,
                    },
                    at,
                );
                let decision = after_pre(cx, decision);
                log_decision(
                    cx,
                    SiteId::GuardFinish,
                    &decision,
                    rid,
                    Some(GuardOrigin::Dropped),
                );
                let ctx = fx(cx, rid, decision.row.0, SiteId::GuardFinish);
                apply_in(cx, &ctx, &decision);
                self.retire_tokens(cx);
                self.st = Ts::Finish;
            }
            Ts::D203 => {
                let token = self
                    .stop
                    .expect("the stop context registered K:CaptureStop");
                let interrupted = self.interrupted.map(|preserved| preserved.code);
                let rid = self.rid;
                let capture = match self.cp {
                    Some(Ok(n)) => StopCaptureResult::Samples { n, interrupted },
                    _ => StopCaptureResult::Err,
                };
                let decision = consume(
                    cx,
                    token,
                    SiteId::StopCaptureResult,
                    rid,
                    move |w, op_status| {
                        core_stop::decide_stop_capture_result(
                            &w.view(),
                            StopCaptureResultInput {
                                token,
                                op_status,
                                rid: RecordingId(rid),
                                result: capture,
                            },
                            at,
                        )
                    },
                );
                let decision = after_pre(cx, decision);
                self.stop = None;
                cx.out.input_code = interrupted;
                log_decision(
                    cx,
                    SiteId::StopCaptureResult,
                    &decision,
                    rid,
                    Some(GuardOrigin::StopCapture),
                );
                let ctx = fx(cx, rid, decision.row.0, SiteId::StopCaptureResult);
                apply_in(cx, &ctx, &decision);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.st = Ts::Finish;
            }
            Ts::AwaitPipeline => {
                let reply = cx
                    .mailbox
                    .iter()
                    .position(|(tid, _)| *tid == cx.tid)
                    .map(|index| cx.mailbox.remove(index).1);
                let Some(reply) = reply else {
                    return Poll::Blocked;
                };
                self.reply = Some(reply);
                if reply == PipelineReply::Panicked {
                    // The pipeline's panic unwound this task too.
                    self.panicked = true;
                    self.st = Ts::Finish;
                } else {
                    self.st = Ts::D204b;
                }
            }
            Ts::D204b => {
                // R204b.1-.4: the second D section, id-only (`KB(false)`, `EM(idle)`).
                let view = cx.w.view();
                let rid = self.rid;
                let decision = core_stop::decide_pipeline_idle_emit(
                    &view,
                    crate::inputs::PipelineIdleEmitInput {
                        rid: RecordingId(rid),
                        kind: crate::inputs::PostKind::StopPost,
                        result: None,
                        interrupted: None,
                        char_count: 0,
                    },
                    at,
                );
                log_decision(cx, SiteId::PipelineIdleEmit, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::PipelineIdleEmit);
                apply_in(cx, &ctx, &decision);
                self.st = Ts::Tail;
            }
            Ts::Tail => {
                // R204b.5 / .6 (D-less): the terminal, the performance finish.
                let rid = self.rid;
                let result = match self.reply {
                    Some(PipelineReply::Ok(terminal)) => {
                        crate::inputs::PipelineResult::Ok(terminal)
                    }
                    _ => crate::inputs::PipelineResult::Err,
                };
                let decision = core_stop::decide_pipeline_idle_emit_tail(
                    crate::inputs::PipelineIdleEmitInput {
                        rid: RecordingId(rid),
                        kind: crate::inputs::PostKind::StopPost,
                        result: Some(result),
                        interrupted: self.interrupted.map(|preserved| preserved.code),
                        char_count: usize::from(matches!(
                            result,
                            crate::inputs::PipelineResult::Ok(PipelineTerminal::Success)
                        )),
                    },
                    at,
                );
                // R204b.5/.6: the terminal code is the interrupted capture's, else the
                // pipeline's own no-speech code.
                cx.out.input_code =
                    self.interrupted
                        .map(|preserved| preserved.code)
                        .or(match result {
                            crate::inputs::PipelineResult::Ok(PipelineTerminal::NoSpeech(code)) => {
                                Some(code)
                            }
                            _ => None,
                        });
                log_decision(cx, SiteId::PipelineIdleEmit, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::PipelineIdleEmit);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.st = Ts::Finish;
            }
            Ts::Finish => {
                self.retire_tokens(cx);
                release_t(cx);
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

// ----------------------------------------------------------------- cancel

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cs {
    AcquireT,
    D205,
    Site206,
    Finish,
}

pub struct CancelFlight {
    st: Cs,
    q: VecDeque<QItem>,
    expected: Option<Rid>,
    rid: Rid,
    op: Option<Continuation>,
    cc: Option<Result<bool, ()>>,
    tail: bool,
}

impl CancelFlight {
    pub fn new(expected: Option<Rid>) -> Self {
        Self {
            st: Cs::AcquireT,
            q: VecDeque::new(),
            expected,
            rid: 0,
            op: None,
            cc: None,
            tail: false,
        }
    }
}

impl Queued for CancelFlight {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, cx: &mut Cx<'_>, _fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        match effect {
            Effect::CaptureCancel { id, .. } => {
                cx.w.sup_send_cancel(id.0, cx.tid);
                Ok(())
            }
            other => unreachable!("a cancel does not run {other:?}"),
        }
    }
}

impl Task for CancelFlight {
    fn name(&self) -> &'static str {
        "cancel"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        let polled = drain_one(self, cx);
        if self.tail && self.q.is_empty() {
            // PD-16's window (D released, the clears and emits still to run) closed.
            cx.w.cancel_tail.remove(&self.rid);
            self.tail = false;
        }
        if let Some(poll) = polled {
            return poll;
        }
        let at = now(cx);
        match self.cs_state() {
            Cs::AcquireT => {
                if !acquire_t(cx) {
                    return Poll::Blocked;
                }
                self.st = Cs::D205;
            }
            Cs::D205 => {
                let view = cx.w.view();
                self.rid = cx.w.current_id;
                let decision = core_cancel::decide_cancel_request(
                    &view,
                    CancelInput {
                        expected: self.expected.map(RecordingId),
                    },
                    at,
                );
                log_decision(cx, SiteId::CancelRequest, &decision, self.rid, None);
                if decision.verdict != Verdict::Applied {
                    release_t(cx);
                    self.st = Cs::Finish;
                    return Poll::Progress;
                }
                self.op = decision.continuation.map(|spec| cx.w.register(spec));
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::CancelRequest);
                apply_in(cx, &ctx, &decision);
                if decision.row.0 == "R205.5" {
                    // D is released: PD-16's window opens (the `PDT` clears and emits
                    // run with no recheck).
                    cx.w.cancel_tail.insert(self.rid, cx.tid);
                    self.tail = true;
                }
                queue_phase(&mut self.q, ctx, &decision, Placement::PostDT);
                self.q.push_back(QItem::ReleaseT);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                self.st = if self.op.is_some() {
                    Cs::Site206
                } else {
                    Cs::Finish
                };
            }
            Cs::Site206 => {
                let rid = self.rid;
                let result = match self.cc {
                    Some(result) => result,
                    None => match cx.w.sup_poll_reply(cx.tid) {
                        ReplyPoll::Waiting => return Poll::Progress,
                        ReplyPoll::Ready(ReplyVal::Cancel(result)) => {
                            self.cc = Some(result);
                            result
                        }
                        ReplyPoll::TimedOut => {
                            self.cc = Some(Err(()));
                            Err(())
                        }
                        ReplyPoll::Ready(ReplyVal::Stop(_) | ReplyVal::Start) => {
                            unreachable!("a cancel waits for a cancel reply")
                        }
                    },
                };
                let token = self.op.take().expect("R205.4 registered K:CaptureCancel");
                let decision = consume(
                    cx,
                    token,
                    SiteId::CancelCaptureResult,
                    rid,
                    move |_, op_status| {
                        core_cancel::decide_cancel_capture_result_nolock(
                            crate::state::LockFreeFacts::default(),
                            CancelCaptureResultInput {
                                token,
                                op_status,
                                rid: RecordingId(rid),
                                result,
                                origin: CancelOrigin::Cancel,
                            },
                            at,
                        )
                    },
                );
                log_decision(cx, SiteId::CancelCaptureResult, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::CancelCaptureResult);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                self.st = Cs::Finish;
            }
            Cs::Finish => {
                if let Some(token) = self.op.take() {
                    cx.w.retire_op(token);
                }
                release_t(cx);
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

impl CancelFlight {
    fn cs_state(&self) -> Cs {
        self.st
    }
}

// --------------------------------------------------------------- pipeline

/// How a pipeline ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PipelineEnd {
    Ok(PipelineTerminal),
    /// Returns an error: the guard's `Drop` finishes (`Dropped` origin).
    Err,
    /// Panics: the guard's `Drop` finishes, and the stop task unwinds (PD-13).
    Panic,
}

#[derive(Clone, Copy, Debug)]
pub struct PipelineCfg {
    /// Steps the pipeline spends parked in decode before it returns.
    pub work: u8,
    pub end: PipelineEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ps {
    Work(u8),
    Deliver,
    Pre,
    Finish,
    Post,
}

pub struct Pipeline {
    st: Ps,
    q: VecDeque<QItem>,
    rid: Rid,
    owner: TaskId,
    op: Option<Continuation>,
    /// How the pipeline really ends: a cancel (or a lost fence) at a checkpoint
    /// turns a result into `Cancelled` (`recording.rs` 3974, 4083, 4239, 4603).
    end: PipelineEnd,
    panicked: bool,
}

impl Pipeline {
    pub fn new(rid: Rid, owner: TaskId, op: Option<Continuation>, cfg: PipelineCfg) -> Self {
        Self {
            st: Ps::Work(cfg.work),
            q: VecDeque::new(),
            rid,
            owner,
            op,
            end: cfg.end,
            panicked: false,
        }
    }

    fn origin(&self) -> GuardOrigin {
        match self.end {
            PipelineEnd::Ok(_) => GuardOrigin::PipelineExit,
            PipelineEnd::Err | PipelineEnd::Panic => GuardOrigin::Dropped,
        }
    }
}

impl Queued for Pipeline {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, _cx: &mut Cx<'_>, _fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        unreachable!("the pipeline does not run {effect:?}")
    }
}

impl Task for Pipeline {
    fn name(&self) -> &'static str {
        "pipeline"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if let Some(poll) = drain_one(self, cx) {
            return poll;
        }
        let at = now(cx);
        match self.st {
            Ps::Work(0) => self.st = Ps::Deliver,
            Ps::Work(n) => self.st = Ps::Work(n - 1),
            Ps::Deliver => {
                // The delivery fence: a D section that checks ownership
                // (`q_owns_delivery`) and pastes under it (SP8).
                if matches!(self.end, PipelineEnd::Ok(_)) && cx.w.cancelled_through >= self.rid {
                    // Checkpoint: cancelled before the text is queued.
                    self.end = PipelineEnd::Ok(PipelineTerminal::Cancelled);
                }
                if matches!(self.end, PipelineEnd::Ok(PipelineTerminal::Success)) {
                    let view = cx.w.view();
                    let owns = view.q_owns_delivery(RecordingId(self.rid));
                    let spec = cx.w.status == Status::Processing
                        && cx.w.current_id == self.rid
                        && cx.w.cancelled_through < self.rid;
                    if owns != spec {
                        cx.out.violations.push(format!(
                            "SP8: q_owns_delivery({}) = {owns}, but the fence is {spec}",
                            self.rid
                        ));
                    }
                    if owns {
                        cx.w.delivered.insert(self.rid);
                    } else {
                        self.end = PipelineEnd::Ok(PipelineTerminal::Cancelled);
                    }
                    cx.out.notes.push(Note::Delivered {
                        rid: self.rid,
                        owned: owns,
                    });
                }
                self.st = Ps::Pre;
            }
            Ps::Pre => {
                // `IdleGuard::finish`: `CX-` before D.
                if self.end == PipelineEnd::Panic {
                    self.panicked = true;
                    cx.out.notes.push(Note::TaskPanicked {
                        tid: cx.tid,
                        rid: self.rid,
                    });
                }
                let ctx = fx(cx, self.rid, "R204a.pre", SiteId::GuardFinish);
                for placed in core_stop::guard_finish_pre_effects(RecordingId(self.rid)) {
                    self.q.push_back(QItem::Fx(ctx, placed));
                }
                self.st = Ps::Finish;
            }
            Ps::Finish => {
                // Site 204a: the D section. `H2 ret` and `H1 ret op_pl` come right
                // after the decide and before the `In` effects (C3.9).
                let view = cx.w.view();
                let rid = self.rid;
                let origin = self.origin();
                let decision = core_stop::decide_guard_finish(
                    &view,
                    crate::inputs::GuardFinishInput {
                        rid: RecordingId(rid),
                        origin,
                        closure_emit: false,
                    },
                    at,
                );
                let decision = after_pre(cx, decision);
                log_decision(cx, SiteId::GuardFinish, &decision, rid, Some(origin));
                cx.w.history.pipelines.remove(&rid);
                if let Some(token) = self.op.take() {
                    cx.w.retire(token);
                }
                let ctx = fx(cx, rid, decision.row.0, SiteId::GuardFinish);
                apply_in(cx, &ctx, &decision);
                self.st = Ps::Post;
            }
            Ps::Post => {
                let reply = match self.end {
                    PipelineEnd::Ok(terminal) => PipelineReply::Ok(terminal),
                    PipelineEnd::Err => PipelineReply::Err,
                    PipelineEnd::Panic => PipelineReply::Panicked,
                };
                cx.mailbox.push((self.owner, reply));
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

// ----------------------------------------------------------------- ticker

/// The partial ticker task of a recording (`SD`): loops, then `hide_on_ticker_exit`.
pub struct Ticker {
    rid: Rid,
    loops: u8,
    panics: bool,
    /// This ticker holds the partial-decode lease (a decode is running).
    holding: bool,
}

impl Ticker {
    pub fn new(rid: Rid) -> Self {
        // Deterministic from the recording: every seventh ticker panics (PD-14).
        Self {
            rid,
            loops: (rid % 3) as u8,
            panics: false,
            holding: false,
        }
    }

    pub fn panicking(mut self) -> Self {
        self.panics = true;
        self
    }

    /// The lease guard's `Drop`: the only release (SP10).
    fn drop_lease(&mut self, cx: &mut Cx<'_>) {
        if self.holding {
            self.holding = false;
            cx.w.partial_lease = None;
            cx.out.notes.push(Note::Lease {
                rid: self.rid,
                tid: cx.tid,
                acquire: false,
            });
        }
    }
}

impl Task for Ticker {
    fn name(&self) -> &'static str {
        "ticker"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if self.loops > 0 {
            self.loops -= 1;
            // One partial decode per loop: `try_begin` is a CAS that a second ticker
            // for the same recording (PD-4) loses while the first decodes.
            if self.holding {
                self.drop_lease(cx);
            } else if cx.w.partial_lease.is_none() {
                cx.w.partial_lease = Some((self.rid, cx.tid));
                self.holding = true;
                cx.out.notes.push(Note::Lease {
                    rid: self.rid,
                    tid: cx.tid,
                    acquire: true,
                });
            }
            return Poll::Progress;
        }
        self.drop_lease(cx);
        if self.panics {
            cx.out.notes.push(Note::TickerPanicked { rid: self.rid });
        } else {
            cx.out.notes.push(Note::TickerExited { rid: self.rid });
        }
        Poll::Done
    }
}
