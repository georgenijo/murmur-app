//! The generator: a seed becomes a scenario, the commands the frontend and the
//! hotkeys issue (C7.5 "each with `id ∈ {current, current−1, current+1}`"), and
//! for every start the *word* of the C7.3 `Attempt` grammar the supervisor will
//! produce. The supervisor's own steps are not generated one by one: they are
//! the word, so no non-grammatical event order can exist.

use super::flights::{Blockers, PipelineCfg, PipelineEnd, StopCfg};
use super::model::*;
use super::rng::Rng;
use crate::inputs::{CaptureErr, PipelineTerminal};
use crate::status::{AudioCancelReason, ErrCode};

/// Which id a stop or cancel names (C7.5): the current one, the previous, the next
/// (all relative to the id current when the command is issued), or no id at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdVar {
    Current,
    Prev,
    Next,
    Unscoped,
}

impl IdVar {
    pub fn resolve(self, current: Rid) -> Option<Rid> {
        match self {
            Self::Current => Some(current),
            Self::Prev => Some(current.saturating_sub(1)),
            Self::Next => Some(current + 1),
            Self::Unscoped => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Cmd {
    Start {
        plan: AttemptPlan,
        blockers: Blockers,
        app_disabled: bool,
        smart: bool,
    },
    Stop {
        id: IdVar,
        cfg: StopCfg,
    },
    Cancel {
        id: IdVar,
    },
}

#[derive(Clone, Debug)]
pub struct Scenario {
    /// Commands in issue order, each with the scheduler step it becomes due at.
    pub cmds: Vec<(u32, Cmd)>,
}

/// How hard a seed leans on one kind of actor (a "profile"): the scheduler re-rolls
/// these every few steps, which is what moves a start into the gap between two
/// supervisor steps.
#[derive(Clone, Copy, Debug)]
pub struct Bias {
    pub clients: u64,
    pub supervisor: u64,
    pub bridge: u64,
    pub pipeline: u64,
    pub ticker: u64,
}

impl Bias {
    pub fn roll(rng: &mut Rng) -> Self {
        let mut pick = |hi: u64| 1 + rng.below(hi);
        Self {
            clients: pick(6),
            supervisor: pick(6),
            bridge: pick(6),
            pipeline: pick(4),
            ticker: pick(2),
        }
    }
}

fn reason(rng: &mut Rng) -> AudioCancelReason {
    use AudioCancelReason::*;
    rng.pick(&[
        DeviceChanged,
        SystemSleep,
        SystemWake,
        RuntimeFailure,
        RuntimeFailure,
        HardDeadline,
    ])
}

pub fn plan(rng: &mut Rng) -> AttemptPlan {
    let kind = match rng.weighted(&[70, 10, 6, 6, 4]) {
        0 => StartKind::Live,
        1 => StartKind::Fail,
        2 => StartKind::PendingCancelled,
        3 => StartKind::Reject(CaptureErr::AlreadyStarting),
        _ => StartKind::Reject(CaptureErr::Other),
    };
    let end = match rng.weighted(&[45, 38, 10]) {
        0 => End::External,
        1 => {
            let reason = reason(rng);
            End::Recover {
                reason,
                init_failed: rng.chance(1, 4),
                stalled: rng.below(2) as u8,
                interrupted: (reason == AudioCancelReason::RuntimeFailure && rng.chance(3, 4))
                    .then(|| rng.pick(&[2_000, 7_999, 8_000, 12_000])),
            }
        }
        _ => End::Crash,
    };
    AttemptPlan {
        kind,
        connecting: rng.below(3) as u8,
        ready: rng.chance(9, 10),
        stalled: rng.below(2) as u8,
        end,
        stop_delivery: match rng.weighted(&[50, 30, 20]) {
            0 => Delivery::Sealed,
            1 => Delivery::Unsealed,
            _ => Delivery::Undelivered,
        },
        stop_samples: rng.pick(&[0, 3_000, 4_799, 4_800, 9_000, 20_000]),
        cancel_err: rng.chance(1, 12),
    }
}

fn pipeline(rng: &mut Rng) -> PipelineCfg {
    PipelineCfg {
        work: rng.below(4) as u8,
        end: match rng.weighted(&[70, 12, 8, 6, 4]) {
            0 => PipelineEnd::Ok(PipelineTerminal::Success),
            1 => PipelineEnd::Ok(PipelineTerminal::NoSpeech(ErrCode::VadNoSpeech)),
            2 => PipelineEnd::Ok(PipelineTerminal::Cancelled),
            3 => PipelineEnd::Err,
            _ => PipelineEnd::Panic,
        },
    }
}

fn stop_cfg(rng: &mut Rng) -> StopCfg {
    StopCfg {
        panic_after_cp: rng.chance(1, 20),
        pipeline: pipeline(rng),
    }
}

fn id_var(rng: &mut Rng) -> IdVar {
    match rng.weighted(&[72, 9, 9, 10]) {
        0 => IdVar::Current,
        1 => IdVar::Prev,
        2 => IdVar::Next,
        _ => IdVar::Unscoped,
    }
}

fn blockers(rng: &mut Rng) -> Blockers {
    if !rng.chance(1, 12) {
        return Blockers::default();
    }
    let mut blockers = Blockers::default();
    match rng.below(5) {
        0 => blockers.relaunch = true,
        1 => blockers.meeting = true,
        2 => blockers.file = true,
        3 => blockers.benchmark = true,
        _ => blockers.corpus = true,
    }
    blockers
}

fn start(rng: &mut Rng) -> Cmd {
    Cmd::Start {
        plan: plan(rng),
        blockers: blockers(rng),
        app_disabled: rng.chance(1, 25),
        smart: rng.chance(1, 3),
    }
}

pub fn scenario(rng: &mut Rng) -> Scenario {
    let mut cmds: Vec<(u32, Cmd)> = Vec::new();
    cmds.push((0, start(rng)));
    let mut at = 0u32;
    let starts = 1 + rng.below(3);
    for index in 0..starts {
        if index > 0 {
            at += rng.below(40) as u32;
            cmds.push((at, start(rng)));
        }
        // Most starts get a stop (L4); some a cancel; a few both.
        if rng.chance(4, 5) {
            cmds.push((
                at + 4 + rng.below(45) as u32,
                Cmd::Stop {
                    id: id_var(rng),
                    cfg: stop_cfg(rng),
                },
            ));
        }
        if rng.chance(1, 4) {
            cmds.push((
                at + 2 + rng.below(45) as u32,
                Cmd::Cancel { id: id_var(rng) },
            ));
        }
    }
    // A late start a stop or a cancel can run into (the PD-18 windows).
    if rng.chance(1, 2) {
        let base = cmds.iter().map(|(at, _)| *at).max().unwrap_or(0);
        cmds.push((base.saturating_sub(rng.below(30) as u32), start(rng)));
    }
    cmds.sort_by_key(|(at, _)| *at);
    Scenario { cmds }
}
