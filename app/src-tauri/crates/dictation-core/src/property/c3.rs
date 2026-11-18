//! The C3 effect table the prefix check compares every status write, terminal and
//! performance completion against (SP3, D-S2b-7 (d)): for each row, the
//! `(from, to)` it may write, the `(outcome, code)` it may claim and the outcome it
//! may complete performance with. An effect whose row does not list it fails, so a
//! row that grows an extra write (a second `Idle -> Idle`, a different terminal
//! outcome) fails the suite even when the row's own decision stays valid.
//!
//! The rows were generated from the unmodified core over 300,000 seeds and then
//! audited against the C3 tables of appendix C (R101.12, R102.2-.6, R201.1,
//! R201.7-.8, R202-R204, R205.5, R206.2, R207, R209, R210, R211, R213, R215, R216):
//! each listed shape is the row's own `ST`, `TERM`, `PC` or guard-finish effect.
//! A row that never writes a status has no `St` entry, so any write from it fails.

use crate::effects::PerfOutcome;
use crate::status::{ErrCode, Status, TerminalOutcome};

/// Which inputs a terminal row claims for: the current recording, an older one, or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Fresh,
    Stale,
    Both,
}

/// What a row's effect may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `ST` from `.0` to `.1`.
    St(Status, Status),
    /// `TERM(outcome, code)` for an input that is (not) the current recording.
    Term(TerminalOutcome, ErrCode, Input),
    /// `PC(outcome)`.
    Pc(PerfOutcome),
    /// The pipeline guard's own completion.
    Pgf(PerfOutcome),
}

pub const C3_EFFECTS: &[(&str, Shape)] = &[
    ("R101.12", Shape::St(Status::Idle, Status::Recovering)),
    ("R101.15", Shape::St(Status::Idle, Status::Starting)),
    ("R102.2", Shape::St(Status::Starting, Status::Idle)),
    (
        "R102.2",
        Shape::Term(
            TerminalOutcome::CaptureInitFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    ("R102.4", Shape::St(Status::Idle, Status::Idle)),
    (
        "R102.4",
        Shape::Term(
            TerminalOutcome::CaptureInitFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    ("R104.2", Shape::St(Status::Starting, Status::Recording)),
    (
        "R201.1",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Both,
        ),
    ),
    ("R201.7", Shape::St(Status::Recovering, Status::Processing)),
    ("R201.8", Shape::St(Status::Recording, Status::Processing)),
    ("R202.3", Shape::Pc(PerfOutcome::Failed)),
    ("R202.3", Shape::St(Status::Processing, Status::Idle)),
    ("R202.4", Shape::Pc(PerfOutcome::Failed)),
    ("R202.4", Shape::St(Status::Idle, Status::Idle)),
    (
        "R202.pre",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::MissingContext,
            Input::Both,
        ),
    ),
    ("R203.1", Shape::Pc(PerfOutcome::Failed)),
    ("R203.1", Shape::St(Status::Processing, Status::Idle)),
    ("R203.2", Shape::Pc(PerfOutcome::Failed)),
    ("R203.2", Shape::St(Status::Idle, Status::Idle)),
    ("R203.2r", Shape::Pc(PerfOutcome::Failed)),
    ("R203.2r", Shape::St(Status::Recovering, Status::Idle)),
    ("R203.3a", Shape::Pc(PerfOutcome::Failed)),
    ("R203.3b", Shape::Pc(PerfOutcome::Failed)),
    ("R203.3c", Shape::Pc(PerfOutcome::Failed)),
    ("R203.3d", Shape::Pc(PerfOutcome::Failed)),
    ("R203.3e", Shape::Pc(PerfOutcome::Failed)),
    ("R203.4", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.4", Shape::St(Status::Processing, Status::Idle)),
    ("R203.5", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.5", Shape::St(Status::Idle, Status::Idle)),
    ("R203.5r", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.5r", Shape::St(Status::Recovering, Status::Idle)),
    ("R203.6", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.6", Shape::St(Status::Processing, Status::Idle)),
    ("R203.7", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.7", Shape::St(Status::Idle, Status::Idle)),
    ("R203.7r", Shape::Pgf(PerfOutcome::NoSpeech)),
    ("R203.7r", Shape::St(Status::Recovering, Status::Idle)),
    ("R203.8b", Shape::Pgf(PerfOutcome::NoSpeech)),
    (
        "R203.pre",
        Shape::Term(TerminalOutcome::NoSpeech, ErrCode::EmptyAudio, Input::Both),
    ),
    (
        "R203.pre",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::StreamInvalidated,
            Input::Both,
        ),
    ),
    (
        "R203.pre",
        Shape::Term(
            TerminalOutcome::StopFailure,
            ErrCode::StopFinalizationFailed,
            Input::Both,
        ),
    ),
    (
        "R203.pre",
        Shape::Term(
            TerminalOutcome::TooShort,
            ErrCode::BelowMinimumDuration,
            Input::Both,
        ),
    ),
    ("R204a.1", Shape::St(Status::Processing, Status::Idle)),
    ("R204a.2", Shape::St(Status::Idle, Status::Idle)),
    ("R204a.3", Shape::St(Status::Recovering, Status::Idle)),
    ("R204a.6", Shape::St(Status::Idle, Status::Idle)),
    ("R204a.6", Shape::St(Status::Processing, Status::Idle)),
    ("R204a.6", Shape::St(Status::Recovering, Status::Idle)),
    (
        "R204b.5",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::TranscriptionFailed,
            Input::Both,
        ),
    ),
    (
        "R204b.5",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::StreamInvalidated,
            Input::Both,
        ),
    ),
    ("R204b.6", Shape::Pgf(PerfOutcome::Cancelled)),
    ("R204b.6", Shape::Pgf(PerfOutcome::Completed)),
    ("R204b.6", Shape::Pgf(PerfOutcome::NoSpeech)),
    (
        "R204b.6",
        Shape::Term(TerminalOutcome::NoSpeech, ErrCode::VadNoSpeech, Input::Both),
    ),
    (
        "R204b.6",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::StreamInvalidated,
            Input::Both,
        ),
    ),
    (
        "R204b.6",
        Shape::Term(TerminalOutcome::Success, ErrCode::None, Input::Both),
    ),
    (
        "R204b.6",
        Shape::Term(
            TerminalOutcome::UserCancelledProcessing,
            ErrCode::CancelledProcessing,
            Input::Both,
        ),
    ),
    ("R205.5", Shape::Pc(PerfOutcome::Cancelled)),
    ("R205.5", Shape::St(Status::Processing, Status::Idle)),
    (
        "R205.5",
        Shape::Term(
            TerminalOutcome::UserCancelledProcessing,
            ErrCode::CancelledProcessing,
            Input::Both,
        ),
    ),
    (
        "R206.2",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
            Input::Both,
        ),
    ),
    (
        "R207.2",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Fresh,
        ),
    ),
    (
        "R207.3",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Fresh,
        ),
    ),
    ("R207.5", Shape::St(Status::Starting, Status::Recording)),
    (
        "R209.1",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Stale,
        ),
    ),
    (
        "R209.2",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Fresh,
        ),
    ),
    ("R209.3", Shape::Pc(PerfOutcome::Cancelled)),
    ("R209.3", Shape::St(Status::Starting, Status::Recovering)),
    (
        "R209.3",
        Shape::Term(
            TerminalOutcome::UserCancelledStarting,
            ErrCode::CancelledStarting,
            Input::Both,
        ),
    ),
    ("R209.4", Shape::Pc(PerfOutcome::Cancelled)),
    ("R209.4", Shape::St(Status::Recording, Status::Recovering)),
    (
        "R209.4",
        Shape::Term(
            TerminalOutcome::UserCancelledRecording,
            ErrCode::CancelledRecording,
            Input::Both,
        ),
    ),
    ("R209.5", Shape::Pc(PerfOutcome::Cancelled)),
    ("R209.5", Shape::St(Status::Recording, Status::Recovering)),
    ("R209.5", Shape::St(Status::Starting, Status::Recovering)),
    (
        "R209.5",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::DeviceChanged,
            Input::Both,
        ),
    ),
    (
        "R209.5",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::SystemSleep,
            Input::Both,
        ),
    ),
    (
        "R209.5",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::SystemWake,
            Input::Both,
        ),
    ),
    ("R209.6", Shape::St(Status::Recording, Status::Recovering)),
    ("R209.6", Shape::St(Status::Starting, Status::Recovering)),
    ("R209.7", Shape::St(Status::Recording, Status::Recovering)),
    ("R209.7", Shape::St(Status::Starting, Status::Recovering)),
    ("R210.2", Shape::St(Status::Idle, Status::Idle)),
    ("R210.3", Shape::St(Status::Recovering, Status::Idle)),
    (
        "R211.1",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Stale,
        ),
    ),
    (
        "R211.2",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Fresh,
        ),
    ),
    ("R211.3", Shape::Pc(PerfOutcome::Failed)),
    ("R211.3", Shape::St(Status::Processing, Status::Idle)),
    (
        "R211.3",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    ("R211.4", Shape::Pc(PerfOutcome::Failed)),
    ("R211.4", Shape::St(Status::Starting, Status::Idle)),
    (
        "R211.4",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    ("R211.5", Shape::Pc(PerfOutcome::Failed)),
    ("R211.5", Shape::St(Status::Recording, Status::Idle)),
    (
        "R211.5",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    ("R211.6", Shape::Pc(PerfOutcome::Failed)),
    ("R211.6", Shape::St(Status::Recovering, Status::Idle)),
    (
        "R211.6",
        Shape::Term(
            TerminalOutcome::PipelineFailure,
            ErrCode::RuntimeFailure,
            Input::Fresh,
        ),
    ),
    (
        "R213.1",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Both,
        ),
    ),
    ("R213.2", Shape::Pc(PerfOutcome::Failed)),
    (
        "R213.2",
        Shape::Term(
            TerminalOutcome::CaptureInitFailure,
            ErrCode::BackendError,
            Input::Both,
        ),
    ),
    (
        "R213.2",
        Shape::Term(
            TerminalOutcome::CaptureInitFailure,
            ErrCode::HostUnavailable,
            Input::Both,
        ),
    ),
    (
        "R213.2",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::HostUnavailable,
            Input::Both,
        ),
    ),
    ("R215.1", Shape::Pc(PerfOutcome::Failed)),
    (
        "R215.1",
        Shape::Term(
            TerminalOutcome::Superseded,
            ErrCode::StaleOwner,
            Input::Both,
        ),
    ),
    ("R216.1", Shape::St(Status::Recovering, Status::Idle)),
    ("R216.2", Shape::St(Status::Idle, Status::Idle)),
    ("R216.4", Shape::St(Status::Processing, Status::Idle)),
    ("R216.pre", Shape::Pc(PerfOutcome::Failed)),
    (
        "R216.pre",
        Shape::Term(
            TerminalOutcome::RuntimeInterruption,
            ErrCode::RuntimeFailure,
            Input::Both,
        ),
    ),
];

/// Whether `row` may write `from -> to`.
pub fn status_write_listed(row: &str, from: Status, to: Status) -> bool {
    C3_EFFECTS
        .iter()
        .any(|(r, shape)| *r == row && *shape == Shape::St(from, to))
}

/// Whether `row` may claim `(outcome, code)` for an input that is `stale` or not.
///
/// `stale` is `None` for a terminal that does not run under D (its input may have
/// gone stale between the decision and the effect), where only the outcome and code
/// are checked.
pub fn terminal_listed(
    row: &str,
    outcome: TerminalOutcome,
    code: ErrCode,
    stale: Option<bool>,
) -> bool {
    C3_EFFECTS.iter().any(|(r, shape)| {
        *r == row
            && matches!(shape, Shape::Term(o, c, input)
            if *o == outcome
                && *c == code
                && match (input, stale) {
                    (_, None) | (Input::Both, _) => true,
                    (Input::Stale, Some(stale)) => stale,
                    (Input::Fresh, Some(stale)) => !stale,
                })
    })
}

/// Whether `row` may complete performance with `outcome` (`pc`: a `PC` effect,
/// else the guard finish).
pub fn perf_listed(row: &str, outcome: PerfOutcome, pc: bool) -> bool {
    C3_EFFECTS.iter().any(|(r, shape)| {
        *r == row
            && match shape {
                Shape::Pc(o) => pc && *o == outcome,
                Shape::Pgf(o) => !pc && *o == outcome,
                _ => false,
            }
    })
}
