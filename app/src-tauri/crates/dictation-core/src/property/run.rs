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
use super::flights::{CancelFlight, StartFlight, StopFlight};
use super::gen::{self, Bias, Cmd};
use super::model::*;
use super::rng::Rng;
use super::sup_task::SupTask;
use super::task::*;

const MAX_STEPS: u32 = 6_000;
const REROLL_EVERY: u32 = 12;

struct Slot {
    tid: TaskId,
    task: Box<dyn Task>,
    daemon: bool,
}

fn weight(bias: &Bias, name: &str) -> u64 {
    match name {
        "supervisor" => bias.supervisor,
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
    pub filtered: bool,
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

fn run(seed: u64, trace: bool, strict: bool) -> (RunReport, Vec<String>) {
    let mut rng = Rng::new(seed);
    let mut scenario_rng = rng.fork();
    let scenario = gen::scenario(&mut scenario_rng);
    let mut w = World::new();
    let mut out = StepOut::default();
    let mut checker = Checker::default();
    if trace {
        checker.trace = Some(Vec::new());
    }
    let mut mailbox: Vec<(TaskId, PipelineReply)> = Vec::new();
    let mut slots: Vec<Slot> = vec![Slot {
        tid: 1,
        task: Box::new(SupTask),
        daemon: true,
    }];
    let mut next_tid: TaskId = 2;
    let mut pending: std::collections::VecDeque<(u32, Cmd)> = scenario.cmds.into();
    let mut bias = Bias::roll(&mut rng);
    let mut clock = 0u32;
    let mut steps = 0u32;

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
            let (_, cmd) = pending.pop_front().expect("checked above");
            let task: Box<dyn Task> = match cmd {
                Cmd::Start {
                    plan,
                    blockers,
                    app_disabled,
                    smart,
                } => Box::new(StartFlight::new(plan, blockers, app_disabled, smart)),
                Cmd::Stop { id, cfg } => {
                    Box::new(StopFlight::new(id.resolve(w.current_id), None, cfg))
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
    let filtered = checker.on_end(&w, &stuck);
    let lines = checker.trace.take().unwrap_or_default();
    (
        RunReport {
            seed,
            violations: std::mem::take(&mut checker.violations),
            hits: checker.hits.clone(),
            steps,
            filtered,
        },
        lines,
    )
}
