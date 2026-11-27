//! The T-holder commands (start, stop, cancel) and the tasks they spawn (the
//! pipeline, the partial ticker), each as a step machine over the real
//! `decide_*` functions. The order of sections and the placement of each
//! effect follow the shell (`commands/recording.rs`) and the C3 tables.

use super::entry::once;
use super::gen::{ImportPlan, Mic, StartCaller};
use super::model::*;
use super::supervisor::ReplyPoll;
use super::task::*;
use crate::admission::Occupancy;
use crate::cancel as core_cancel;
use crate::decision::{ContKind, Continuation, ObsKind, OpStatus, SiteId, Verdict};
use crate::effects::{Effect, Placement};
use crate::ids::{Handle, Now, RecordingId};
use crate::inputs::{
    CancelCaptureResultInput, CancelInput, CancelOrigin, CaptureErr, CaptureResult,
    CaptureResultInput, DestinationInput, DeviceSel, GestureObs, GuardOrigin, Input,
    InterruptedMeta, Origin, PipelineTerminal, ProcessAudioFinishInput, ProcessAudioFinishKind,
    ProcessAudioStartInput, SmartAutoFailure, SmartAutoSel, SpeculativeObs, StartContextInput,
    StartFlightEndInput, StartInput, StopCaptureResult, StopCaptureResultInput, StopContextInput,
    StopInput,
};
use crate::process_audio as core_pa;
use crate::replies::Reply;
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
    /// Who sent the start (#1162): the main window's `beginRecording` awaits the reply.
    caller: StartCaller,
    /// The reply of the last decision that carried one: the value the command returns.
    reply: Option<Reply>,
    /// The microphone selection the start carries.
    mic: Mic,
    /// The command already returned: it returns once.
    replied: bool,
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
            caller: StartCaller::Ipc,
            reply: None,
            mic: Mic::SystemDefault,
            replied: false,
        }
    }

    /// This start carries microphone selection `mic`.
    pub fn with_mic(mut self, mic: Mic) -> Self {
        self.mic = mic;
        self
    }

    /// This start was sent by `caller`.
    pub fn with_caller(mut self, caller: StartCaller) -> Self {
        self.caller = caller;
        self
    }

    /// Remember the reply a decision carries: the command returns the last one.
    fn note_reply(&mut self, decision: &crate::decision::Decision) {
        if let Some(reply) = &decision.reply {
            self.reply = Some(reply.clone());
        }
    }

    fn input(&self, cx: &Cx<'_>) -> StartInput {
        StartInput {
            origin: Origin::Toggle,
            device: match self.mic {
                Mic::SystemDefault => DeviceSel::SystemDefault,
                Mic::Explicit => DeviceSel::Explicit,
                Mic::SmartAutoResolved | Mic::SmartAutoFailed | Mic::SmartAutoWithFixed => {
                    DeviceSel::SmartAuto
                }
            },
            gesture: GestureObs::default(),
            speculative: SpeculativeObs::default(),
            kept_preview: false,
            app_disabled: self.app_disabled,
            fixed_and_smart: self.mic == Mic::SmartAutoWithFixed,
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
        self.send_reply(cx);
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
                    Some(direct) => {
                        // R101.1-.2 take no D, but the entry is still `handle`'s.
                        let view = cx.w.view();
                        let pre = once(
                            &cx.w.handle_faults,
                            &view,
                            Input::StartRequest(input),
                            at,
                            direct,
                        );
                        self.note_reply(&pre);
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
                // The observation rounds, in the order the shell reads the facts: the
                // retiring-capture check (5523), then the Smart Auto resolve (5546), which
                // takes no D and so is answered from the model's own plan.
                let mut smart_round = false;
                let decision = loop {
                    let direct = core_start::decide_start_request(&view, input.clone(), at);
                    let decision = once(
                        &cx.w.handle_faults,
                        &view,
                        Input::StartRequest(input.clone()),
                        at,
                        direct,
                    );
                    rounds += 1;
                    if matches!(decision.verdict, Verdict::NeedObservation(_)) {
                        cx.out.observations.push(decision.row.0);
                    }
                    match decision.verdict {
                        Verdict::NeedObservation(ObsKind::CleanupPending { id })
                            if input.cleanup_pending.is_none() =>
                        {
                            input.cleanup_pending = Some(cx.w.sup.cleanup_pending(id.0));
                        }
                        Verdict::NeedObservation(ObsKind::SmartAutoSelection)
                            if input.smart_auto_result.is_none() =>
                        {
                            if input.device != DeviceSel::SmartAuto
                                || input.cleanup_pending.is_none()
                            {
                                cx.out.violations.push(format!(
                                    "R101.13: the Smart Auto resolve was asked for with device \
                                     {:?} and the cleanup check {:?}",
                                    input.device, input.cleanup_pending
                                ));
                            }
                            smart_round = true;
                            input.smart_auto_result = Some(if self.mic == Mic::SmartAutoFailed {
                                Err(SmartAutoFailure::ResolveFailed)
                            } else {
                                Ok(SmartAutoSel { handle: Handle(1) })
                            });
                        }
                        Verdict::NeedObservation(kind) => {
                            unreachable!(
                                "a start asks once for each of the cleanup check and the \
                                 Smart Auto resolve, not {kind:?} (round {rounds})"
                            )
                        }
                        _ => break decision,
                    }
                };
                // The Smart Auto round is the start gate's own (admission order: blockers,
                // status, the retiring capture, then the resolve): a start that reached the
                // resolve asked for it exactly once, an admitted one asked for it iff it is a
                // Smart Auto start, and a failed resolve never admitted. These read the
                // decisions the flight acts on, not the direct call.
                let smart_start = input.device == DeviceSel::SmartAuto;
                if decision.row.0 == "R101.15" && smart_start != smart_round {
                    cx.out.violations.push(format!(
                        "R101.13: an admitted start with device {:?} {} the Smart Auto resolve",
                        input.device,
                        if smart_round { "ran" } else { "skipped" }
                    ));
                }
                if self.mic == Mic::SmartAutoFailed && decision.row.0 == "R101.15" {
                    cx.out.violations.push(
                        "R101.14: a start whose Smart Auto resolve failed was admitted".into(),
                    );
                }
                if smart_round && self.mic == Mic::SmartAutoFailed && decision.row.0 != "R101.14" {
                    cx.out.violations.push(format!(
                        "R101.14: a failed Smart Auto resolve decided {}",
                        decision.row.0
                    ));
                }
                self.rid = cx.w.current_id;
                self.note_reply(&decision);
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
                        let input = CaptureResultInput {
                            token,
                            op_status,
                            result: CaptureResult::Ok,
                        };
                        let direct = core_start::decide_start_capture_result_nolock(
                            w.facts(),
                            input.clone(),
                            at,
                        );
                        once(
                            &w.handle_faults,
                            &w.view(),
                            Input::StartCaptureResult(input),
                            at,
                            direct,
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
                        let input = CaptureResultInput {
                            token,
                            op_status,
                            result: CaptureResult::Err(error.clone()),
                        };
                        let view = w.view();
                        let direct =
                            core_start::decide_start_capture_result(&view, input.clone(), at);
                        once(
                            &w.handle_faults,
                            &view,
                            Input::StartCaptureResult(input),
                            at,
                            direct,
                        )
                    },
                );
                let decision = after_pre(cx, decision);
                self.cap = None;
                self.note_reply(&decision);
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
                let direct =
                    core_start::decide_start_destination(&view, RecordingId(self.rid), true, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::StartDestination(DestinationInput {
                        rid: RecordingId(self.rid),
                        smart_enabled: true,
                    }),
                    at,
                    direct,
                );
                self.note_reply(&decision);
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
                        let input = StartContextInput { token, op_status };
                        let view = w.view();
                        let direct = core_start::decide_start_context(&view, input, at);
                        once(
                            &w.handle_faults,
                            &view,
                            Input::StartContext(input),
                            at,
                            direct,
                        )
                    },
                );
                self.ctx = None;
                self.note_reply(&decision);
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
                            let input = crate::inputs::ReadyPublishInput {
                                token,
                                op_status,
                                rid: RecordingId(rid),
                                audio_ready: false,
                                context_present: None,
                                context_ready_at: at,
                            };
                            let direct =
                                core_start::decide_ready_publish_nolock(w.facts(), input, at);
                            once(
                                &w.handle_faults,
                                &w.view(),
                                Input::ReadyPublish(input),
                                at,
                                direct,
                            )
                        });
                    self.ready = None;
                    self.note_reply(&decision);
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
                            let direct = core_start::decide_ready_publish(&view, input, at);
                            let decision = once(
                                &w.handle_faults,
                                &view,
                                Input::ReadyPublish(input),
                                at,
                                direct,
                            );
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
                self.note_reply(&decision);
                log_decision(cx, SiteId::ReadyPublish, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::ReadyPublish);
                apply_in(cx, &ctx, &decision);
                self.st = Ss::FlightEnd;
            }
            Ss::FlightEnd => {
                // R104.7: `PDT:PM`, T released, `PT:PB`, `PT:REL`.
                let direct = core_start::decide_start_flight_end(RecordingId(self.rid), at);
                let decision = once(
                    &cx.w.handle_faults,
                    &cx.w.view(),
                    Input::StartFlightEnd(StartFlightEndInput {
                        rid: RecordingId(self.rid),
                    }),
                    at,
                    direct,
                );
                self.note_reply(&decision);
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
                self.send_reply(cx);
                return Poll::Done;
            }
        }
        Poll::Progress
    }
}

impl StartFlight {
    /// The command returns: the main window's `beginRecording` receives the response later,
    /// in a step of its own (#1162), and it is the response of this one start (`cx.tid`).
    /// Only the main window awaits it: the overlay and the IPC write no window state from it.
    /// A reply that is an `Err` (a refusal with the resolve's text, a failed capture start), or
    /// that a dropped start future never produced, is a failed invoke: the wrapper turns it
    /// into an error response with no `state` (`lib/dictation.ts`), so it writes nothing but
    /// still ends the start operation.
    fn send_reply(&mut self, cx: &mut Cx<'_>) {
        if self.caller != StartCaller::Window(MAIN_WINDOW) || self.replied {
            return;
        }
        self.replied = true;
        let response = match self.reply.take() {
            Some(Reply::Typed {
                ty,
                state,
                recording_id,
            }) if !self.dropped => StartResponse::Typed {
                ty,
                state,
                recording_id: recording_id.map(|id| id.0),
            },
            _ => StartResponse::Error,
        };
        cx.w.start_replied(StartReply {
            tid: cx.tid,
            response,
        });
    }
}

// ----------------------------------------------------------------- import

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Is {
    AcquireT,
    Pre,
    D301,
    DestAwait,
    Dest,
    Parse,
    D302Parse,
    AwaitPipeline,
    D302Tail,
    Finish,
}

/// The legacy `process_audio` import (`commands/recording.rs` `process_audio_inner`), sites
/// 301 and 302. It takes T like a start, decides R301.1-.5 (the benchmark through
/// `NeedObservation`), drops T and runs its `PT` effects, runs its own Smart Destination
/// section (R301.6), then either fails the parse (R302.1) or runs the pipeline and decides the
/// tail (R302.2). No terminal is decided anywhere in it: the pipeline's guard (204a) and the
/// tail restore `Idle`, the import never claims a terminal.
pub struct ImportFlight {
    st: Is,
    q: VecDeque<QItem>,
    plan: ImportPlan,
    rid: Rid,
    /// `K:Pipeline`: the import owns it until the pipeline's guard (or an exit) retires it.
    op: Option<Continuation>,
}

impl ImportFlight {
    pub fn new(plan: ImportPlan) -> Self {
        Self {
            st: Is::AcquireT,
            q: VecDeque::new(),
            plan,
            rid: 0,
            op: None,
        }
    }

    fn input(&self, benchmark: Option<bool>) -> ProcessAudioStartInput {
        ProcessAudioStartInput {
            occupancy: Occupancy {
                meeting_active: self.plan.blockers.meeting,
                file_transcribing: self.plan.blockers.file,
                ..Occupancy::default()
            },
            benchmark_running: benchmark,
        }
    }

    /// The import's own exit: both history entries it owns are retired (`pipeline_owner`'s
    /// `Drop`), `H2` first.
    fn retire_owned(&mut self, cx: &mut Cx<'_>) {
        cx.w.history.pipelines.remove(&self.rid);
        if let Some(token) = self.op.take() {
            cx.w.retire_op(token);
        }
    }
}

impl Queued for ImportFlight {
    fn queue(&mut self) -> &mut VecDeque<QItem> {
        &mut self.q
    }

    fn cross(&mut self, _cx: &mut Cx<'_>, _fx: &FxCtx, effect: &Effect) -> Result<(), ()> {
        unreachable!("an import does not run {effect:?}")
    }
}

impl Task for ImportFlight {
    fn name(&self) -> &'static str {
        "import"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if let Some(poll) = drain_one(self, cx) {
            return poll;
        }
        let at = now(cx);
        match self.st {
            Is::AcquireT => {
                if !acquire_t(cx) {
                    return Poll::Blocked;
                }
                self.st = Is::Pre;
            }
            Is::Pre => {
                // R301.1 (3684): a meeting owns ASR. Decided with no D, but the entry is
                // still `handle`'s.
                let input = self.input(None);
                match core_pa::decide_process_audio_start_pre(&input, at) {
                    Some(direct) => {
                        let view = cx.w.view();
                        let pre = once(
                            &cx.w.handle_faults,
                            &view,
                            Input::ProcessAudioStart(input),
                            at,
                            direct,
                        );
                        self.rid = cx.w.current_id;
                        log_decision(cx, SiteId::ProcessAudioStart, &pre, self.rid, None);
                        release_t(cx);
                        self.st = Is::Finish;
                    }
                    None => self.st = Is::D301,
                }
            }
            Is::D301 => {
                let view = cx.w.view();
                let mut input = self.input(None);
                // The benchmark is read after the file check passed (D-S1-2), through
                // `NeedObservation(BenchmarkRunning)`: the flight answers from the plan.
                let decision = loop {
                    let direct = core_pa::decide_process_audio_start(&view, input.clone(), at);
                    let decision = once(
                        &cx.w.handle_faults,
                        &view,
                        Input::ProcessAudioStart(input.clone()),
                        at,
                        direct,
                    );
                    if matches!(decision.verdict, Verdict::NeedObservation(_)) {
                        cx.out.observations.push(decision.row.0);
                    }
                    match decision.verdict {
                        Verdict::NeedObservation(ObsKind::BenchmarkRunning)
                            if input.benchmark_running.is_none() =>
                        {
                            input.benchmark_running = Some(self.plan.blockers.benchmark);
                        }
                        Verdict::NeedObservation(kind) => {
                            unreachable!("an import asks once for the benchmark, not {kind:?}")
                        }
                        _ => break decision,
                    }
                };
                // The refusal order of today's code (meeting before D, then the file, the
                // benchmark, the status), read off the model, not off either entry: a decision
                // that disagrees fails the seed whichever entry produced it.
                let blockers = self.plan.blockers;
                let expected = if blockers.meeting {
                    "R301.1"
                } else if blockers.file {
                    "R301.2"
                } else if blockers.benchmark {
                    "R301.3"
                } else if cx.w.status != Status::Idle {
                    "R301.4"
                } else {
                    "R301.5"
                };
                if decision.row.0 != expected {
                    cx.out.violations.push(format!(
                        "R301: an import with meeting {}, file {}, benchmark {} on status {:?} \
                         decided {}, not {expected}",
                        blockers.meeting,
                        blockers.file,
                        blockers.benchmark,
                        cx.w.status,
                        decision.row.0
                    ));
                }
                log_decision(
                    cx,
                    SiteId::ProcessAudioStart,
                    &decision,
                    cx.w.current_id,
                    None,
                );
                if decision.row.0 != "R301.5" {
                    release_t(cx);
                    self.st = Is::Finish;
                    return Poll::Progress;
                }
                // R301.5: `ID+` is the first `In` effect, so every `In` effect runs for the new
                // id. The shell registers `H1` and `H2` right after the decide; the model does
                // it once the id exists (SP6 bounds a continuation by the current id), and
                // nothing runs between the two inside one D section.
                let rid = cx.w.current_id + 1;
                let ctx = fx(cx, rid, "R301.5", SiteId::ProcessAudioStart);
                apply_in(cx, &ctx, &decision);
                self.rid = cx.w.current_id;
                let spec = decision.continuation.expect("R301.5 registers K:Pipeline");
                cx.w.history.pipelines.insert(self.rid);
                self.op = Some(cx.w.register(spec));
                // T was dropped between D and `PT:KB(true), EV(GenStarted), EM(processing)`.
                self.q.push_back(QItem::ReleaseT);
                queue_phase(&mut self.q, ctx, &decision, Placement::PostT);
                self.st = if self.plan.smart {
                    Is::DestAwait
                } else {
                    Is::Parse
                };
            }
            Is::DestAwait => {
                // The smart-destination capture's await: other tasks run meanwhile.
                self.st = Is::Dest;
            }
            Is::Dest => {
                // R301.6: under D, is the capture still current?
                let view = cx.w.view();
                let rid = RecordingId(self.rid);
                let direct = core_pa::decide_process_audio_destination(&view, rid, true, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::ProcessAudioDestination(DestinationInput {
                        rid,
                        smart_enabled: true,
                    }),
                    at,
                    direct,
                );
                log_decision(cx, SiteId::ProcessAudioStart, &decision, self.rid, None);
                let ctx = fx(cx, self.rid, decision.row.0, SiteId::ProcessAudioStart);
                apply_in(cx, &ctx, &decision);
                if matches!(decision.verdict, Verdict::Stale(_)) {
                    // A stale return: the owner retires both entries, nothing else runs.
                    self.retire_owned(cx);
                    self.st = Is::Finish;
                } else {
                    self.st = Is::Parse;
                }
            }
            Is::Parse => {
                if self.plan.parses {
                    // The pipeline runs in the caller: the model parks the import on it.
                    let op = self.op.take();
                    cx.spawn.push(Box::new(Pipeline::new(
                        self.rid,
                        cx.tid,
                        op,
                        self.plan.pipeline,
                    )));
                    self.st = Is::AwaitPipeline;
                } else {
                    // `IdleGuard::finish_owned`: `CX-` runs before D.
                    let ctx = fx(cx, self.rid, "R302.pre", SiteId::ProcessAudioFinish);
                    for placed in core_stop::guard_finish_pre_effects(
                        RecordingId(self.rid),
                        GuardOrigin::LegacyParse,
                    ) {
                        self.q.push_back(QItem::Fx(ctx, placed));
                    }
                    self.st = Is::D302Parse;
                }
            }
            Is::D302Parse => {
                // R302.1: the parse failure, `Input::GuardFinish` with `LegacyParse` through
                // `handle`.
                let view = cx.w.view();
                let rid = self.rid;
                let direct = core_pa::decide_process_audio_finish(
                    &view,
                    ProcessAudioFinishInput {
                        rid: RecordingId(rid),
                        kind: ProcessAudioFinishKind::ParseFailed,
                        closure_emit: true,
                    },
                    at,
                );
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::GuardFinish(crate::inputs::GuardFinishInput {
                        rid: RecordingId(rid),
                        origin: GuardOrigin::LegacyParse,
                        closure_emit: true,
                    }),
                    at,
                    direct,
                );
                let decision = after_pre(cx, decision);
                log_decision(
                    cx,
                    SiteId::ProcessAudioFinish,
                    &decision,
                    rid,
                    Some(GuardOrigin::LegacyParse),
                );
                // `H2 ret` and `H1 ret op_pl` come right after the decide and before the `In`
                // effects (C3.9).
                self.retire_owned(cx);
                let ctx = fx(cx, rid, decision.row.0, SiteId::ProcessAudioFinish);
                apply_in(cx, &ctx, &decision);
                self.st = Is::Finish;
            }
            Is::AwaitPipeline => {
                let reply = cx
                    .mailbox
                    .iter()
                    .position(|(tid, _)| *tid == cx.tid)
                    .map(|index| cx.mailbox.remove(index).1);
                let Some(reply) = reply else {
                    return Poll::Blocked;
                };
                // A panic unwinds the command: no tail. An `Err` is returned after the tail
                // (`pipeline_result?`), so the tail still runs.
                self.st = if reply == PipelineReply::Panicked {
                    Is::Finish
                } else {
                    Is::D302Tail
                };
            }
            Is::D302Tail => {
                // R302.2: the tail after the pipeline, `Input::PipelineIdleEmit` with
                // `LegacyPost` through `handle`.
                let view = cx.w.view();
                let rid = self.rid;
                let direct = core_pa::decide_process_audio_finish(
                    &view,
                    ProcessAudioFinishInput {
                        rid: RecordingId(rid),
                        kind: ProcessAudioFinishKind::PipelineDone,
                        closure_emit: false,
                    },
                    at,
                );
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::PipelineIdleEmit(crate::inputs::PipelineIdleEmitInput {
                        rid: RecordingId(rid),
                        kind: crate::inputs::PostKind::LegacyPost,
                        result: None,
                        interrupted: None,
                        char_count: 0,
                    }),
                    at,
                    direct,
                );
                log_decision(cx, SiteId::ProcessAudioFinish, &decision, rid, None);
                let ctx = fx(cx, rid, decision.row.0, SiteId::ProcessAudioFinish);
                apply_in(cx, &ctx, &decision);
                self.st = Is::Finish;
            }
            Is::Finish => {
                self.retire_owned(cx);
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
    /// `Drop` writes the status and claims `PipelineFailure/WorkerPanicked` (R204a.7;
    /// PD-13 fixed, #1191).
    pub panic_after_cp: bool,
    /// The pipeline the stop starts (if the capture is long enough).
    pub pipeline: PipelineCfg,
}

pub struct StopFlight {
    st: Ts,
    q: VecDeque<QItem>,
    expected: Option<Rid>,
    auto: Option<Continuation>,
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
    /// The window whose `handleStop` sent this stop (#1167): its in-flight guard clears when
    /// the flight returns.
    window: Option<usize>,
}

impl StopFlight {
    /// This stop was sent by window `window`'s gesture handler.
    pub fn with_window(mut self, window: usize) -> Self {
        self.window = Some(window);
        self
    }

    pub fn new(expected: Option<Rid>, auto: Option<Continuation>, cfg: StopCfg) -> Self {
        Self {
            window: None,
            st: Ts::AcquireT,
            q: VecDeque::new(),
            expected,
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
                    taken_earlier: false,
                };
                if input.is_stale() {
                    // R201.1: T held, no D; the stale path takes the interrupted
                    // capture first (D-S1-3, PD-15).
                    let stale = self.expected.expect("a stale stop has an expectation");
                    let mut input = input;
                    let view = cx.w.view();
                    let direct = core_stop::decide_stop_request_stale(input, at).expect("stale");
                    let first = once(
                        &cx.w.handle_faults,
                        &view,
                        Input::StopRequest(input),
                        at,
                        direct,
                    );
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
                        input.taken_earlier = earlier.is_some();
                    }
                    let direct = core_stop::decide_stop_request_stale(input, at).expect("stale");
                    let decision = once(
                        &cx.w.handle_faults,
                        &view,
                        Input::StopRequest(input),
                        at,
                        direct,
                    );
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
                let direct = core_stop::decide_stop_request(&view, input, at);
                let mut decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::StopRequest(input),
                    at,
                    direct,
                );
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
                    let direct = core_stop::decide_stop_request(&view, input, at);
                    decision = once(
                        &cx.w.handle_faults,
                        &view,
                        Input::StopRequest(input),
                        at,
                        direct,
                    );
                }
                log_decision(cx, SiteId::StopRequest, &decision, self.rid, None);
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
                    move |w, op_status| {
                        let input = CancelCaptureResultInput {
                            token,
                            op_status,
                            rid: RecordingId(rid),
                            result,
                            origin: CancelOrigin::Stop,
                        };
                        // The direct call reads no fact of the view (`default()`), so a
                        // `handle` that fed it the real ones must still agree.
                        let direct = core_cancel::decide_cancel_capture_result_nolock(
                            crate::state::LockFreeFacts::default(),
                            input,
                            at,
                        );
                        once(
                            &w.handle_faults,
                            &w.view(),
                            Input::CancelCaptureResult(input),
                            at,
                            direct,
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
                            let input = StopContextInput {
                                token,
                                op_status,
                                rid: RecordingId(rid),
                                context_present: true,
                                interrupted_error,
                            };
                            let direct =
                                core_stop::decide_stop_context_nolock(w.facts(), input, at);
                            once(
                                &w.handle_faults,
                                &w.view(),
                                Input::StopContext(input),
                                at,
                                direct,
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
                    let input = StopContextInput {
                        token,
                        op_status,
                        rid: RecordingId(rid),
                        context_present: false,
                        interrupted_error,
                    };
                    let view = w.view();
                    let direct = core_stop::decide_stop_context(&view, input, at);
                    once(
                        &w.handle_faults,
                        &view,
                        Input::StopContext(input),
                        at,
                        direct,
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
                    // `Unwound` origin (R204a.7), which claims the terminal the task
                    // can no longer claim (PD-13 fixed, #1191); the ops retire by `Drop`.
                    self.panicked = true;
                    cx.out.notes.push(Note::TaskPanicked {
                        tid: cx.tid,
                        rid: self.rid,
                    });
                    let ctx = fx(cx, self.rid, "R204a.pre", SiteId::GuardFinish);
                    for placed in core_stop::guard_finish_pre_effects(
                        RecordingId(self.rid),
                        GuardOrigin::Unwound,
                    ) {
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
                            let input = StopCaptureResultInput { op_status, ..probe };
                            let direct =
                                core_stop::decide_stop_capture_result_nolock(w.facts(), input, at);
                            once(
                                &w.handle_faults,
                                &w.view(),
                                Input::StopCaptureResult(input),
                                at,
                                direct,
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
                let input = crate::inputs::GuardFinishInput {
                    rid: RecordingId(rid),
                    origin: GuardOrigin::Unwound,
                    closure_emit: false,
                };
                let direct = core_stop::decide_guard_finish(&view, input, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::GuardFinish(input),
                    at,
                    direct,
                );
                let decision = after_pre(cx, decision);
                log_decision(
                    cx,
                    SiteId::GuardFinish,
                    &decision,
                    rid,
                    Some(GuardOrigin::Unwound),
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
                        let input = StopCaptureResultInput {
                            token,
                            op_status,
                            rid: RecordingId(rid),
                            result: capture,
                        };
                        let view = w.view();
                        let direct = core_stop::decide_stop_capture_result(&view, input, at);
                        once(
                            &w.handle_faults,
                            &view,
                            Input::StopCaptureResult(input),
                            at,
                            direct,
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
                let input = crate::inputs::PipelineIdleEmitInput {
                    rid: RecordingId(rid),
                    kind: crate::inputs::PostKind::StopPost,
                    result: None,
                    interrupted: None,
                    char_count: 0,
                };
                let direct = core_stop::decide_pipeline_idle_emit(&view, input, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::PipelineIdleEmit(input),
                    at,
                    direct,
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
                let input = crate::inputs::PipelineIdleEmitInput {
                    rid: RecordingId(rid),
                    kind: crate::inputs::PostKind::StopPost,
                    result: Some(result),
                    interrupted: self.interrupted.map(|preserved| preserved.code),
                    char_count: usize::from(matches!(
                        result,
                        crate::inputs::PipelineResult::Ok(PipelineTerminal::Success)
                    )),
                };
                let direct = core_stop::decide_pipeline_idle_emit_tail(input, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &cx.w.view(),
                    Input::PipelineIdleEmit(input),
                    at,
                    direct,
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
                if let Some(window) = self.window {
                    cx.w.window_stop_returned(window);
                    cx.out.notes.push(Note::WindowStopReturned {
                        window,
                        tid: cx.tid,
                    });
                }
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
                let input = CancelInput {
                    expected: self.expected.map(RecordingId),
                };
                let direct = core_cancel::decide_cancel_request(&view, input, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::CancelRequest(input),
                    at,
                    direct,
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
                    move |w, op_status| {
                        let input = CancelCaptureResultInput {
                            token,
                            op_status,
                            rid: RecordingId(rid),
                            result,
                            origin: CancelOrigin::Cancel,
                        };
                        // The direct call reads no fact of the view (`default()`), so a
                        // `handle` that fed it the real ones must still agree.
                        let direct = core_cancel::decide_cancel_capture_result_nolock(
                            crate::state::LockFreeFacts::default(),
                            input,
                            at,
                        );
                        once(
                            &w.handle_faults,
                            &w.view(),
                            Input::CancelCaptureResult(input),
                            at,
                            direct,
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
    /// Panics: the guard's `Drop` finishes with `Unwound` and claims the terminal
    /// (R204a.7), and the stop task unwinds.
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
            PipelineEnd::Err => GuardOrigin::Dropped,
            PipelineEnd::Panic => GuardOrigin::Unwound,
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
                for placed in
                    core_stop::guard_finish_pre_effects(RecordingId(self.rid), self.origin())
                {
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
                let input = crate::inputs::GuardFinishInput {
                    rid: RecordingId(rid),
                    origin,
                    closure_emit: false,
                };
                let direct = core_stop::decide_guard_finish(&view, input, at);
                let decision = once(
                    &cx.w.handle_faults,
                    &view,
                    Input::GuardFinish(input),
                    at,
                    direct,
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
