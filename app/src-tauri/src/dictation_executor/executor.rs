//! The executor skeleton (appendix C4.1, C4.1a, C4.2, C4.4a).
//!
//! `run(phase)` executes the effects of **exactly one placement** of a
//! [`Decision`], in returned order, on the caller's thread. The **shell** owns
//! every lock scope and calls `run` at each site's own boundaries
//! (`Pre` → acquire D → decide → `In` → release D → `PostDT` → release T →
//! `PostT`); the executor
//!
//! * never takes D or T, never touches `core_history`, and never calls a decide
//!   function (the structural `executor_never_locks` test greps this file);
//! * runs the Sync, Blocking and Detached effects of a phase and returns
//!   `Awaiting` **instead of** running an Awaited effect (`CS` on the
//!   kept-preview path, `PS`), which is always the last effect of its phase; the
//!   shell awaits it and re-enters through [`Executor::resume`] and the next
//!   decide (C4.1a);
//! * refuses re-entrancy **before any lock**: a thread-local `IN_CORE` flag makes
//!   any nested decide or `run` return `Invalid(Reentrant)` with no effects
//!   (C4.2). The flag covers only the synchronous spans (`decide_guarded`, each
//!   `run`, each `resume`); it is never held across an await;
//! * lets a panic unwind (X5): it does not `catch_unwind`, so effects after the
//!   panic do not run and each earlier effect has already committed its own
//!   field (X6). The persisted M1/M2 copy commits through the field adapter at
//!   `ST` / `ID+`; M3/M4 write only the live slot.
//!
//! **Stage 2a PR 0: no production path calls the executor.** Tests drive it with
//! fake [`Ports`]; the family PRs call it from the shell at today's lines.

use super::adapter::{dictation_status, FieldAdapter};
use crate::dictation_context::DictationContextSnapshot;
use crate::state::{AppState, DictationState};
use murmur_dictation_core::decision::{Decision, DecisionError, InvalidKind, RowId, Verdict};
use murmur_dictation_core::effects::{Cond, Effect, Placement};
use murmur_dictation_core::ids::RecordingId;
use std::cell::Cell;
use std::sync::Arc;

thread_local! {
    /// `true` while this thread is inside `decide_guarded`, `run` or `resume`
    /// (C4.2). Thread-local: a task may migrate threads across an await, so the
    /// flag is never held across one.
    static IN_CORE: Cell<bool> = const { Cell::new(false) };
}

/// The shell's monotonic stamp (`Now`, C2.1): microseconds since the first call.
/// Taken before D is acquired, where today's code takes its own timing instant.
/// No decide function branches on it.
pub(crate) fn core_now() -> murmur_dictation_core::ids::Now {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let micros = EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_micros();
    murmur_dictation_core::ids::Now(u64::try_from(micros).unwrap_or(u64::MAX))
}

/// `true` while the current thread is inside a core span.
pub(crate) fn in_core() -> bool {
    IN_CORE.with(Cell::get)
}

/// Proof that this thread entered the core and no one else is inside. Dropping
/// it (including by unwinding) clears the flag.
pub(crate) struct CoreEntry(());

impl CoreEntry {
    /// The **first** statement of every shell entry, before D, T or
    /// `core_history` is attempted: a nested entry is refused with
    /// `Invalid(Reentrant)` and touches no lock (C4.2).
    pub(crate) fn enter() -> Result<Self, InvalidKind> {
        if IN_CORE.with(|flag| flag.replace(true)) {
            // Dev builds assert (C4.2); test builds return the refusal so the
            // re-entrancy tests can observe it.
            #[cfg(not(test))]
            debug_assert!(false, "re-entrant dictation core entry");
            return Err(InvalidKind::Reentrant);
        }
        Ok(Self(()))
    }
}

impl Drop for CoreEntry {
    fn drop(&mut self) {
        IN_CORE.with(|flag| flag.set(false));
        // Test builds: the outermost entry ends here; work it deferred must have a
        // drain guard around it (D-S2-4, D-S2-8).
        #[cfg(test)]
        crate::commands::recording::assert_deferred_work_has_a_drain();
    }
}

/// The decision a refused entry returns: `Invalid`, no effects, no mutation.
pub(crate) fn refused(kind: InvalidKind) -> Decision {
    Decision {
        verdict: Verdict::Invalid(kind),
        transition: None,
        effects: Vec::new(),
        continuation: None,
        reply: None,
        row: RowId("refused"),
    }
}

/// Run a (pure) decide function under the re-entrancy guard. A nested call
/// returns `Invalid(Reentrant)` without calling `decide`.
pub(crate) fn decide_guarded(decide: impl FnOnce() -> Decision) -> Decision {
    match CoreEntry::enter() {
        Ok(_entry) => {
            let decision = decide();
            // Entry/header conservation (D-S2c-7 a): E7 counts the decisions the live
            // shell took against the headers its flights submitted.
            #[cfg(feature = "dictation-shadow")]
            super::shadow::note_decision(&decision);
            decision
        }
        Err(kind) => refused(kind),
    }
}

// ------------------------------------------------------------------ dispatch

/// How an effect runs (C4.1a). Exactly one per effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchKind {
    /// Returns before the next effect and never waits on another thread.
    Sync,
    /// A synchronous call that can block the thread up to a stated timeout; run
    /// inline on the shell's thread.
    Blocking,
    /// A future the shell must `.await`; always the last effect of its phase.
    Awaited,
    /// Spawns and returns, with today's spawn primitive.
    Detached,
}

/// The dispatch kind of `effect`. `kept_preview` selects the awaited form of `CS`
/// (`spawn_blocking(..).await`, ≤ 17 s) over the direct blocking one (≤ 2 s).
/// Exhaustive on purpose: a new effect must be classified.
pub(crate) fn dispatch_kind(effect: &Effect, kept_preview: bool) -> DispatchKind {
    use DispatchKind::*;
    match effect {
        Effect::SetStatus(_)
        | Effect::AllocId
        | Effect::EmitStatus { .. }
        | Effect::EmitEvent(_)
        | Effect::Presentation { .. }
        | Effect::Terminal(_)
        | Effect::PerfAbort
        | Effect::PerfRegister
        | Effect::ClearContext(_)
        | Effect::SetContext(_)
        | Effect::KeyboardProcessing(_)
        | Effect::MarkCancelled(_)
        | Effect::TouchLastTranscription
        | Effect::GestureAccepted
        | Effect::PrepareCaptureClaimed
        | Effect::PreviewHideClaim
        | Effect::TakeInterrupted { .. }
        | Effect::TakeStopStamps
        | Effect::ReleaseClaims
        | Effect::TraceRequested
        | Effect::Observe(_)
        | Effect::AcceptTelemetry
        | Effect::ProductionCaptureFailure(_)
        | Effect::SpeculativeConsume
        | Effect::PipelineEnter(_)
        | Effect::PipelineGuardFinish(_)
        | Effect::SmartDestinationEmit => Sync,
        Effect::AxSnapshot(_) | Effect::CaptureStop(_) | Effect::CaptureCancel { .. } => Blocking,
        Effect::CaptureStart(_) if kept_preview => Awaited,
        Effect::CaptureStart(_) => Blocking,
        Effect::PipelineSpawn(_) => Awaited,
        Effect::StartSideTasks
        | Effect::ModelPreparation
        | Effect::PerfBegin
        | Effect::PerfComplete(_)
        | Effect::SelfStop { .. } => Detached,
    }
}

// --------------------------------------------------------------------- ports

/// A port could not perform its effect. Only an emit's failure is acted on (a
/// paired `PRES` is skipped, C4.4a); every other failure is recorded in the
/// witness and the phase continues (the result is unused today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PortFailure;

/// What a port returns.
pub(crate) type PortResult = Result<(), PortFailure>;

/// The effect ports: production's implementation is today's code, the tests'
/// is a recorder. One method per effect family; the executor picks the family
/// from the effect and the dispatch kind from [`dispatch_kind`]. The field
/// effects (`ST`, `ID+`, `MC`, `CX±`) are not ports: the executor commits them
/// through the `FieldAdapter` on [`Ports::app_state`].
pub(crate) trait Ports {
    /// The state the field adapter writes.
    fn app_state(&self) -> &AppState;
    /// Shell predicate `P-spec-flowing` (skips `EM(starting)`, 5618).
    fn speculative_flowing(&self) -> bool;
    /// The context payload bound to a `CX+` (an opaque shell local, X4).
    fn context_snapshot(&self, id: RecordingId) -> Option<Arc<DictationContextSnapshot>>;
    /// `EM`, `EV`, `SDE` (Sync). The result gates a paired `PRES`.
    fn emit(&self, effect: &Effect) -> PortResult;
    /// An `EM` that runs in `In`, with the D guard's contents in hand: today's
    /// `IdleGuard::finish` hands its `on_idle` closure the held `DictationState`
    /// (Stage 2a-2, site 204a). Defaults to [`Ports::emit`], so only a port that
    /// needs D's contents overrides it.
    fn emit_under_d(&self, effect: &Effect, _dictation: &DictationState) -> PortResult {
        self.emit(effect)
    }
    /// `PRES`, `TRQ` (Sync traces).
    fn trace(&self, effect: &Effect) -> PortResult;
    /// `TERM` (Sync claim; the persist is a Detached sub-step).
    fn terminal(&self, effect: &Effect) -> PortResult;
    /// `PA`, `PR`, `PE`, `PGF`, `PCF` (Sync) and `PB`, `PC` (Detached).
    fn perf(&self, effect: &Effect) -> PortResult;
    /// `KB` (Sync).
    fn keyboard(&self, effect: &Effect) -> PortResult;
    /// `GA`, `PCC`, `PH`, `TS`, `TI`, `SPEC`, `REL`, `TL`, `ACC`, `OBS` (Sync).
    fn claims(&self, effect: &Effect) -> PortResult;
    /// `AX`, `CP`, `CC`, and `CS` on the direct path (Blocking).
    fn blocking(&self, effect: &Effect) -> PortResult;
    /// `SD`, `PM`, `RS` (Detached).
    fn detached(&self, effect: &Effect) -> PortResult;
    /// A Detached effect that runs in `In`, with the D guard's contents in hand:
    /// R104.2's `SD` registers the partial ticker under the same D hold that wrote
    /// `Recording` (`register_partial_ticker_locked` takes the held state).
    /// Defaults to [`Ports::detached`], so only a port that needs D's contents
    /// overrides it.
    fn detached_under_d(&self, effect: &Effect, _dictation: &DictationState) -> PortResult {
        self.detached(effect)
    }
    /// The ticker registration attempt the last `SD` made (query 404's inputs and
    /// result), if it made one: the shadow witnesses it (appendix C8.2). A port that
    /// runs `SD` stores it where the gate ran and the executor takes it right after
    /// the effect. Default: none.
    #[cfg(feature = "dictation-shadow")]
    fn take_ticker_note(&self) -> Option<super::shadow::TickerNote> {
        None
    }
}

// -------------------------------------------------------------------- report

/// What `run` / `resume` return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PhaseReport {
    /// The phase ran to its end.
    Done,
    /// An Awaited effect is next: the shell awaits it (holding exactly the guards
    /// it holds today) and calls [`Executor::resume`]. Always the last effect of
    /// its phase.
    Awaiting(AwaitedEffect),
    /// Nothing ran: a nested entry (`Reentrant`) or a call out of order
    /// (`Impossible`).
    Invalid(InvalidKind),
}

/// The Awaited effect the shell must drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AwaitedEffect {
    /// Index in the decision's effect list.
    pub(crate) index: usize,
    /// The effect (`CS` on the kept-preview path, or `PS`).
    pub(crate) effect: Effect,
}

/// What the shell observed when it drove an Awaited effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AwaitedOutcome {
    /// The awaited call returned `Ok`.
    Completed,
    /// The awaited call returned `Err` (or its worker's `JoinError`); the shell
    /// feeds that to the next decide as the continuation input.
    Failed,
}

/// What the executor did with one effect (the shadow witness, C8.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WitnessOutcome {
    /// Executed; the port reported success.
    Ran,
    /// Executed; the port reported failure (an emit's gates a paired `PRES`).
    Failed,
    /// A shell predicate skipped it (`UnlessSpeculativeFlowing`, or a `PRES`
    /// whose paired emit failed).
    Skipped,
    /// Handed to the shell to await.
    Awaiting,
    /// The awaited effect's outcome, recorded by `resume`.
    Resumed(AwaitedOutcome),
}

/// One witnessed effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Witness {
    /// Index in the decision's effect list.
    pub(crate) index: usize,
    /// The legend code (`ST`, `EM`, ...).
    pub(crate) code: &'static str,
    /// What happened.
    pub(crate) outcome: WitnessOutcome,
}

/// What the shell passes to each `run`.
pub(crate) struct PhaseCtx<'g> {
    /// `Some` only for `In`: the contents of the D guard the shell holds. The
    /// executor never locks D itself.
    pub(crate) dictation: Option<&'g mut DictationState>,
    /// The recording the decision is about, for the `ST` trace. After an `ID+`
    /// the executor uses the id it allocated instead (X4).
    pub(crate) recording_id: u64,
}

/// A structural defect found when a decision is handed to the executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutorError {
    /// The decision's own structural rules (C2.1) do not hold.
    Decision(DecisionError),
    /// An Awaited effect is not the last effect of its placement (C4.1a).
    AwaitedNotLast {
        /// Index of the Awaited effect.
        index: usize,
    },
}

// ------------------------------------------------------------------ executor

/// One decision's effects, driven phase by phase by the shell.
pub(crate) struct Executor<'a> {
    ports: &'a dyn Ports,
    decision: Decision,
    kept_preview: bool,
    cursor: usize,
    last_phase: Option<Placement>,
    awaiting: Option<usize>,
    witness: Vec<Witness>,
    emit_ok: Vec<Option<bool>>,
    bound_recording_id: Option<u64>,
    id_mismatches: u32,
    /// The shadow capture of this decision (appendix C8.2).
    #[cfg(feature = "dictation-shadow")]
    shadow: super::shadow::Flight,
    /// The live-field write and the ticker attempt of the effect being recorded.
    #[cfg(feature = "dictation-shadow")]
    noted_mirror: Option<crate::dictation_shadow::types::MirrorWitness>,
    #[cfg(feature = "dictation-shadow")]
    noted_ticker: Option<super::shadow::TickerNote>,
}

/// An executor parked across an `.await`: everything but the (non-`Send`) ports
/// borrow. The shell drops its ports and executor before the await, then
/// [`Suspended::reattach`]es fresh ports and resumes with the actual outcome
/// (Stage 2a-2: the pipeline cursor survives the pipeline await).
pub(crate) struct Suspended {
    decision: Decision,
    kept_preview: bool,
    cursor: usize,
    last_phase: Option<Placement>,
    awaiting: Option<usize>,
    witness: Vec<Witness>,
    emit_ok: Vec<Option<bool>>,
    bound_recording_id: Option<u64>,
    id_mismatches: u32,
    #[cfg(feature = "dictation-shadow")]
    shadow: super::shadow::Flight,
}

impl Suspended {
    /// Rebuild the executor over `ports`, exactly where it was parked.
    pub(crate) fn reattach(self, ports: &dyn Ports) -> Executor<'_> {
        Executor {
            ports,
            decision: self.decision,
            kept_preview: self.kept_preview,
            cursor: self.cursor,
            last_phase: self.last_phase,
            awaiting: self.awaiting,
            witness: self.witness,
            emit_ok: self.emit_ok,
            bound_recording_id: self.bound_recording_id,
            id_mismatches: self.id_mismatches,
            #[cfg(feature = "dictation-shadow")]
            shadow: self.shadow,
            #[cfg(feature = "dictation-shadow")]
            noted_mirror: None,
            #[cfg(feature = "dictation-shadow")]
            noted_ticker: None,
        }
    }
}

impl<'a> Executor<'a> {
    /// Hand a decision to the executor. Fails if its structure is wrong, or if an
    /// Awaited effect is not the last of its placement (C4.1a). `kept_preview`
    /// selects the awaited form of `CS`.
    pub(crate) fn new(
        ports: &'a dyn Ports,
        decision: Decision,
        kept_preview: bool,
    ) -> Result<Self, ExecutorError> {
        decision.validate().map_err(ExecutorError::Decision)?;
        for (index, placed) in decision.effects.iter().enumerate() {
            let awaited = dispatch_kind(&placed.effect, kept_preview) == DispatchKind::Awaited;
            let followed_in_phase = decision
                .effects
                .get(index + 1)
                .is_some_and(|next| next.at == placed.at);
            if awaited && followed_in_phase {
                return Err(ExecutorError::AwaitedNotLast { index });
            }
        }
        let emit_ok = vec![None; decision.effects.len()];
        Ok(Self {
            ports,
            decision,
            kept_preview,
            cursor: 0,
            last_phase: None,
            awaiting: None,
            witness: Vec::new(),
            emit_ok,
            bound_recording_id: None,
            id_mismatches: 0,
            #[cfg(feature = "dictation-shadow")]
            shadow: super::shadow::Flight::none(),
            #[cfg(feature = "dictation-shadow")]
            noted_mirror: None,
            #[cfg(feature = "dictation-shadow")]
            noted_ticker: None,
        })
    }

    /// Run the effects of exactly one placement, in returned order. The shell
    /// calls this at the site's own boundary; phases must come in order
    /// (`Pre ≤ In ≤ PostDT ≤ PostT`).
    pub(crate) fn run(&mut self, phase: Placement, ctx: &mut PhaseCtx<'_>) -> PhaseReport {
        let _entry = match CoreEntry::enter() {
            Ok(entry) => entry,
            Err(kind) => return PhaseReport::Invalid(kind),
        };
        crate::core_history::assert_not_held();
        let impossible = PhaseReport::Invalid(InvalidKind::Impossible(self.decision.row));
        if self.awaiting.is_some()
            || self.last_phase.is_some_and(|last| phase < last)
            || self
                .decision
                .effects
                .get(self.cursor)
                .is_some_and(|next| next.at < phase)
        {
            return impossible;
        }
        self.last_phase = Some(phase);
        while let Some(placed) = self.decision.effects.get(self.cursor) {
            if placed.at != phase {
                break;
            }
            let index = self.cursor;
            let effect = placed.effect.clone();
            if dispatch_kind(&effect, self.kept_preview) == DispatchKind::Awaited {
                self.awaiting = Some(index);
                self.record(index, &effect, WitnessOutcome::Awaiting);
                return PhaseReport::Awaiting(AwaitedEffect { index, effect });
            }
            if let Err(invalid) = self.execute(index, &effect, ctx) {
                return PhaseReport::Invalid(invalid);
            }
            self.cursor += 1;
        }
        #[cfg(feature = "dictation-shadow")]
        if self.is_finished() {
            self.shadow.finish();
        }
        PhaseReport::Done
    }

    /// Record the outcome of the Awaited effect the shell has just driven and
    /// move past it. The shell then builds the continuation input in
    /// straight-line code and calls the next decide.
    pub(crate) fn resume(&mut self, outcome: AwaitedOutcome) -> PhaseReport {
        let _entry = match CoreEntry::enter() {
            Ok(entry) => entry,
            Err(kind) => return PhaseReport::Invalid(kind),
        };
        let Some(index) = self.awaiting.take() else {
            return PhaseReport::Invalid(InvalidKind::Impossible(self.decision.row));
        };
        let code = self.decision.effects[index].effect.code();
        self.witness.push(Witness {
            index,
            code,
            outcome: WitnessOutcome::Resumed(outcome),
        });
        self.cursor = index + 1;
        #[cfg(feature = "dictation-shadow")]
        {
            // The awaited effect ran (its outcome is the continuation's input, not the
            // effect's result): witness it now, where the shell drove it.
            let placed = &self.decision.effects[index];
            self.shadow.effect(super::shadow::Noted {
                index,
                placement: placed.at,
                effect: &placed.effect,
                taken: true,
                result: crate::dictation_shadow::types::EffectResult::Ran,
                mirror: None,
                ticker: None,
            });
            if self.is_finished() {
                self.shadow.finish();
            }
        }
        PhaseReport::Done
    }

    /// Park the executor (its cursor and witness) so the shell can drop the
    /// ports and await. See [`Suspended`].
    pub(crate) fn suspend(self) -> Suspended {
        Suspended {
            decision: self.decision,
            kept_preview: self.kept_preview,
            cursor: self.cursor,
            last_phase: self.last_phase,
            awaiting: self.awaiting,
            witness: self.witness,
            emit_ok: self.emit_ok,
            bound_recording_id: self.bound_recording_id,
            id_mismatches: self.id_mismatches,
            #[cfg(feature = "dictation-shadow")]
            shadow: self.shadow,
        }
    }

    /// What the executor did so far, in order.
    pub(crate) fn witness(&self) -> &[Witness] {
        &self.witness
    }

    /// The recording id an `ID+` allocated (a shell local, X4).
    pub(crate) fn bound_recording_id(&self) -> Option<u64> {
        self.bound_recording_id
    }

    /// Times an `ID+` allocated an id other than the one the decision predicted
    /// (`core.next_id()`); counted, never ignored (C0.2, C8.6).
    pub(crate) fn id_mismatches(&self) -> u32 {
        self.id_mismatches
    }

    /// `true` when every effect has run.
    pub(crate) fn is_finished(&self) -> bool {
        self.awaiting.is_none() && self.cursor >= self.decision.effects.len()
    }

    fn record(&mut self, index: usize, effect: &Effect, outcome: WitnessOutcome) {
        self.witness.push(Witness {
            index,
            code: effect.code(),
            outcome,
        });
        #[cfg(feature = "dictation-shadow")]
        self.note_shadow(index, effect, outcome);
    }

    /// Hand one recorded effect to the shadow flight (appendix C8.2). An awaited
    /// effect is witnessed when the shell resumes it, with the rest of its phase.
    #[cfg(feature = "dictation-shadow")]
    fn note_shadow(&mut self, index: usize, effect: &Effect, outcome: WitnessOutcome) {
        use crate::dictation_shadow::types::EffectResult;
        let mirror = self.noted_mirror.take();
        let ticker = self.noted_ticker.take();
        if !self.shadow.is_active() {
            return;
        }
        let (taken, ok) = match outcome {
            WitnessOutcome::Ran => (true, true),
            WitnessOutcome::Failed => (true, false),
            WitnessOutcome::Skipped => (false, false),
            WitnessOutcome::Awaiting | WitnessOutcome::Resumed(_) => return,
        };
        // What a result means depends on the effect: an emit's `Ok`, whether a
        // terminal claim won (D-S2-9). Any other port failure is not a result.
        let result = match effect {
            Effect::EmitStatus { .. } | Effect::EmitEvent(_) | Effect::SmartDestinationEmit => {
                EffectResult::Emitted { ok }
            }
            Effect::Terminal(_) => EffectResult::Claimed { won: ok },
            _ => EffectResult::Ran,
        };
        let placement = self.decision.effects[index].at;
        self.shadow.effect(super::shadow::Noted {
            index,
            placement,
            effect,
            taken,
            result,
            mirror,
            ticker,
        });
    }

    /// Attach the shadow flight to this decision, which was taken under D (D-less
    /// entries: at decision time) from `input` against `pre` (appendix C8.2). The
    /// header is assembled from values the live path holds; **no core work happens
    /// here**.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn attach_shadow(
        &mut self,
        mut flight: super::shadow::Flight,
        input: murmur_dictation_core::inputs::Input,
        pre: crate::dictation_shadow::types::Projection,
        now: murmur_dictation_core::ids::Now,
    ) {
        flight.decided(
            input,
            pre,
            now,
            self.decision.row,
            self.decision.reply.as_ref(),
            self.decision.effects.len(),
        );
        self.shadow = flight;
    }

    /// Adopt a flight whose header the shell already sealed (a refusal's header is
    /// sealed from the decision under D, before any executor exists).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn adopt_shadow(&mut self, flight: super::shadow::Flight) {
        self.shadow = flight;
    }

    /// Attach the flight to an executor that runs only a leading part of the decision
    /// (its `Pre` effects, or a prefix that runs ahead of the decision): the
    /// witnesses buffer inline until the header exists.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn attach_shadow_leading(&mut self, mut flight: super::shadow::Flight) {
        flight.enter_pre();
        self.shadow = flight;
    }

    /// Take the flight back (for the executor of the rest of the decision). The
    /// effects this executor consumed keep their indices.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn take_shadow(&mut self) -> super::shadow::Flight {
        let mut flight = std::mem::replace(&mut self.shadow, super::shadow::Flight::none());
        flight.advance(self.decision.effects.len());
        flight
    }

    /// The flight, for an H1 / H2 operation or an observed fact the shell records.
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn shadow_flight(&mut self) -> &mut super::shadow::Flight {
        &mut self.shadow
    }

    /// Take the header's `seq` (still under D) and read the persisted M1/M2 as the
    /// `In` effects committed them. `dictation` is the held guard (`None` for a D-less
    /// entry). The shell releases D and then submits ([`super::shadow::Sealed::submit`]).
    #[cfg(feature = "dictation-shadow")]
    pub(crate) fn seal_shadow(
        &self,
        dictation: Option<&DictationState>,
    ) -> super::shadow::Sealed<'_> {
        use crate::dictation_admission::core_status;
        use crate::dictation_shadow::types::Projection;
        use murmur_dictation_core::state::LiveObs;
        let post = dictation.map(|dictation| Projection {
            status: core_status(dictation.status),
            current_id: RecordingId(
                self.ports
                    .app_state()
                    .recording_id
                    .load(std::sync::atomic::Ordering::SeqCst),
            ),
            obs: LiveObs::default(),
        });
        self.shadow.seal(post)
    }

    fn record_port(&mut self, index: usize, effect: &Effect, result: PortResult) {
        let outcome = if result.is_ok() {
            WitnessOutcome::Ran
        } else {
            WitnessOutcome::Failed
        };
        self.record(index, effect, outcome);
    }

    /// Execute one Sync, Blocking or Detached effect. `Err` only for an effect
    /// that needs the D guard (`ST`, `ID+`) outside `In`.
    fn execute(
        &mut self,
        index: usize,
        effect: &Effect,
        ctx: &mut PhaseCtx<'_>,
    ) -> Result<(), InvalidKind> {
        let ports = self.ports;
        let impossible = InvalidKind::Impossible(self.decision.row);
        match effect {
            // ---- field effects: committed through the adapter (C1.4.2)
            Effect::SetStatus(to) => {
                let Some(dictation) = ctx.dictation.as_deref_mut() else {
                    return Err(impossible);
                };
                let recording_id = self.bound_recording_id.unwrap_or(ctx.recording_id);
                FieldAdapter::set_status(recording_id, dictation, dictation_status(*to));
                self.record(index, effect, WitnessOutcome::Ran);
            }
            Effect::AllocId => {
                let Some(dictation) = ctx.dictation.as_deref_mut() else {
                    return Err(impossible);
                };
                let predicted = dictation.core.current_id().next();
                let allocated = FieldAdapter::alloc_id(ports.app_state(), dictation);
                if predicted != RecordingId(allocated) {
                    self.id_mismatches += 1;
                }
                self.bound_recording_id = Some(allocated);
                self.record(index, effect, WitnessOutcome::Ran);
            }
            Effect::MarkCancelled(id) => {
                // With the feature compiled and the shadow not armed for this flight,
                // the live path is the default build's, exactly (D-S2c-7 c).
                #[cfg(feature = "dictation-shadow")]
                if self.shadow.is_active() {
                    self.noted_mirror = Some(FieldAdapter::cancel_through_witnessed(
                        ports.app_state(),
                        id.0,
                    ));
                } else {
                    FieldAdapter::cancel_through(ports.app_state(), id.0);
                }
                #[cfg(not(feature = "dictation-shadow"))]
                FieldAdapter::cancel_through(ports.app_state(), id.0);
                self.record(index, effect, WitnessOutcome::Ran);
            }
            Effect::ClearContext(id) => {
                #[cfg(feature = "dictation-shadow")]
                if self.shadow.is_active() {
                    self.noted_mirror = Some(FieldAdapter::clear_context_witnessed(
                        ports.app_state(),
                        id.0,
                    ));
                } else {
                    FieldAdapter::clear_context(ports.app_state(), id.0);
                }
                #[cfg(not(feature = "dictation-shadow"))]
                FieldAdapter::clear_context(ports.app_state(), id.0);
                self.record(index, effect, WitnessOutcome::Ran);
            }
            Effect::SetContext(id) => match ports.context_snapshot(*id) {
                Some(snapshot) => {
                    #[cfg(feature = "dictation-shadow")]
                    if self.shadow.is_active() {
                        self.noted_mirror = Some(FieldAdapter::set_context_witnessed(
                            ports.app_state(),
                            id.0,
                            snapshot,
                        ));
                    } else {
                        FieldAdapter::set_context(ports.app_state(), id.0, snapshot);
                    }
                    #[cfg(not(feature = "dictation-shadow"))]
                    FieldAdapter::set_context(ports.app_state(), id.0, snapshot);
                    self.record(index, effect, WitnessOutcome::Ran);
                }
                None => self.record(index, effect, WitnessOutcome::Failed),
            },
            // ---- emits and the traces paired with them
            Effect::EmitStatus { cond, .. } => {
                if *cond == Cond::UnlessSpeculativeFlowing && ports.speculative_flowing() {
                    self.record(index, effect, WitnessOutcome::Skipped);
                } else {
                    let result = match ctx.dictation.as_deref() {
                        Some(dictation) => ports.emit_under_d(effect, dictation),
                        None => ports.emit(effect),
                    };
                    self.emit_ok[index] = Some(result.is_ok());
                    self.record_port(index, effect, result);
                }
            }
            Effect::EmitEvent(_) | Effect::SmartDestinationEmit => {
                let result = ports.emit(effect);
                self.emit_ok[index] = Some(result.is_ok());
                self.record_port(index, effect, result);
            }
            Effect::Presentation { cond, .. } => {
                let paired_ok = match cond {
                    Cond::EmitOk(paired) => self
                        .emit_ok
                        .get(usize::from(*paired))
                        .copied()
                        .flatten()
                        .unwrap_or(false),
                    Cond::Always => true,
                    Cond::UnlessSpeculativeFlowing => !ports.speculative_flowing(),
                };
                if paired_ok {
                    let result = ports.trace(effect);
                    self.record_port(index, effect, result);
                } else {
                    self.record(index, effect, WitnessOutcome::Skipped);
                }
            }
            Effect::TraceRequested => {
                let result = ports.trace(effect);
                self.record_port(index, effect, result);
            }
            // ---- everything else is one port call
            Effect::Terminal(_) => {
                let result = ports.terminal(effect);
                self.record_port(index, effect, result);
            }
            Effect::PerfAbort
            | Effect::PerfRegister
            | Effect::PerfBegin
            | Effect::PerfComplete(_)
            | Effect::PipelineEnter(_)
            | Effect::PipelineGuardFinish(_)
            | Effect::ProductionCaptureFailure(_) => {
                let result = ports.perf(effect);
                self.record_port(index, effect, result);
            }
            Effect::KeyboardProcessing(flag) => {
                // The flag the port actually committed, read from the live flag itself
                // (not derived from the request or the port's result), before and after;
                // only for an armed flight (D-S2c-7 c, f).
                #[cfg(feature = "dictation-shadow")]
                let before = self.shadow.is_active().then(crate::keyboard::is_processing);
                let result = ports.keyboard(effect);
                #[cfg(feature = "dictation-shadow")]
                if let Some(before) = before {
                    use crate::dictation_shadow::types::{MirrorField, MirrorValue, MirrorWitness};
                    self.noted_mirror = Some(MirrorWitness {
                        field: MirrorField::KeyboardProcessing,
                        before: MirrorValue::Flag(before),
                        after: MirrorValue::Flag(crate::keyboard::is_processing()),
                    });
                }
                let _ = flag;
                self.record_port(index, effect, result);
            }
            Effect::TouchLastTranscription
            | Effect::GestureAccepted
            | Effect::PrepareCaptureClaimed
            | Effect::PreviewHideClaim
            | Effect::TakeInterrupted { .. }
            | Effect::TakeStopStamps
            | Effect::ReleaseClaims
            | Effect::Observe(_)
            | Effect::AcceptTelemetry
            | Effect::SpeculativeConsume => {
                let result = ports.claims(effect);
                self.record_port(index, effect, result);
            }
            Effect::AxSnapshot(_)
            | Effect::CaptureStart(_)
            | Effect::CaptureStop(_)
            | Effect::CaptureCancel { .. } => {
                let result = ports.blocking(effect);
                self.record_port(index, effect, result);
            }
            Effect::StartSideTasks | Effect::ModelPreparation | Effect::SelfStop { .. } => {
                let result = match ctx.dictation.as_deref() {
                    Some(dictation) => ports.detached_under_d(effect, dictation),
                    None => ports.detached(effect),
                };
                #[cfg(feature = "dictation-shadow")]
                if *effect == Effect::StartSideTasks {
                    // The gate the spawner asked, or the skip it returned (D-S2c-6, 7 d):
                    // exactly one, taken from this port's own slot.
                    self.noted_ticker = ports.take_ticker_note();
                }
                self.record_port(index, effect, result);
            }
            // Awaited: `run` returns `Awaiting` before reaching `execute`.
            Effect::PipelineSpawn(_) => return Err(impossible),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_dictation_core::status::{ErrCode, Status, TerminalOutcome};

    /// One of every effect the legend has, so a new variant must be classified
    /// by `dispatch_kind` (its match is exhaustive) and listed here.
    fn every_effect() -> Vec<Effect> {
        use murmur_dictation_core::effects::*;
        use murmur_dictation_core::ids::Handle;
        let id = RecordingId(1);
        vec![
            Effect::SetStatus(Status::Idle),
            Effect::AllocId,
            Effect::EmitStatus {
                status: Status::Idle,
                cond: Cond::Always,
            },
            Effect::EmitEvent(EventKind::Cancelled),
            Effect::Presentation {
                kind: PresentationKind::InitFailed,
                cond: Cond::Always,
            },
            Effect::Terminal(Terminal::new(TerminalOutcome::Success, ErrCode::None)),
            Effect::PerfAbort,
            Effect::PerfRegister,
            Effect::PerfBegin,
            Effect::PerfComplete(PerfComplete {
                outcome: PerfOutcome::Failed,
                stage: PerfStage::CaptureFinalization,
                cause: None,
                with_audio_duration: false,
            }),
            Effect::ClearContext(id),
            Effect::SetContext(id),
            Effect::KeyboardProcessing(false),
            Effect::MarkCancelled(id),
            Effect::TouchLastTranscription,
            Effect::AxSnapshot(AxMode::Stop),
            Effect::GestureAccepted,
            Effect::PrepareCaptureClaimed,
            Effect::PreviewHideClaim,
            Effect::TakeInterrupted {
                id,
                mode: TakeMode::Use,
            },
            Effect::TakeStopStamps,
            Effect::StartSideTasks,
            Effect::ModelPreparation,
            Effect::CaptureStart(id),
            Effect::CaptureStop(id),
            Effect::CaptureCancel {
                id,
                reason: murmur_dictation_core::status::AudioCancelReason::User,
            },
            Effect::SelfStop {
                id,
                trigger: murmur_dictation_core::status::StopTrigger::Other,
            },
            Effect::PipelineSpawn(id),
            Effect::ReleaseClaims,
            Effect::TraceRequested,
            Effect::Observe(murmur_dictation_core::decision::ObsKind::SmartAutoSelection),
            Effect::AcceptTelemetry,
            Effect::ProductionCaptureFailure(Handle(0)),
            Effect::SpeculativeConsume,
            Effect::PipelineEnter(PipelineEnter::Vad),
            Effect::PipelineGuardFinish(PerfOutcome::Completed),
            Effect::SmartDestinationEmit,
        ]
    }

    #[test]
    fn dispatch_kinds_follow_c4_1a() {
        use DispatchKind::*;
        let kinds: Vec<(&str, DispatchKind)> = every_effect()
            .iter()
            .map(|effect| (effect.code(), dispatch_kind(effect, false)))
            .collect();
        let of = |code: &str| kinds.iter().find(|(c, _)| *c == code).expect(code).1;
        for code in [
            "ST", "ID+", "EM", "EV", "SDE", "PRES", "TRQ", "CX-", "CX+", "KB", "MC", "TL", "GA",
            "PCC", "PH", "TS", "TI", "SPEC", "REL", "ACC", "PCF", "PA", "PR", "PE", "PGF", "TERM",
            "OBS",
        ] {
            assert_eq!(of(code), Sync, "{code}");
        }
        for code in ["AX", "CP", "CC", "CS"] {
            assert_eq!(of(code), Blocking, "{code} (direct)");
        }
        for code in ["SD", "PM", "PB", "PC", "RS"] {
            assert_eq!(of(code), Detached, "{code}");
        }
        assert_eq!(of("PS"), Awaited);
        // `CS` is the only effect whose kind depends on the path.
        assert_eq!(
            dispatch_kind(&Effect::CaptureStart(RecordingId(1)), true),
            Awaited
        );
        for effect in every_effect() {
            if !matches!(effect, Effect::CaptureStart(_)) {
                assert_eq!(
                    dispatch_kind(&effect, false),
                    dispatch_kind(&effect, true),
                    "{}",
                    effect.code()
                );
            }
        }
    }

    #[test]
    fn exactly_two_effects_are_ever_awaited() {
        let awaited: Vec<&str> = every_effect()
            .iter()
            .filter(|effect| dispatch_kind(effect, true) == DispatchKind::Awaited)
            .map(|effect| effect.code())
            .collect();
        assert_eq!(awaited, ["CS", "PS"]);
    }

    #[test]
    fn a_refused_entry_is_invalid_with_no_effects() {
        let decision = refused(InvalidKind::Reentrant);
        assert_eq!(decision.verdict, Verdict::Invalid(InvalidKind::Reentrant));
        assert!(decision.effects.is_empty() && decision.transition.is_none());
        assert_eq!(decision.validate(), Ok(()));
    }
}
