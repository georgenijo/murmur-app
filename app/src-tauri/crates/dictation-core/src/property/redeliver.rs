//! A duplicate or late delivery of a continuation token, as a task of its own
//! (D-S2b-7 (e)): some steps after the real delivery retired the token, the site
//! decides for it again in whatever world it finds. Two things must hold (SP7,
//! C1.4.1 rule 3): the retired token takes the same row with the same effects as an
//! outstanding one would in that world, and the delivery retires nothing, neither the
//! token's own absent entry nor any other outstanding op (a token that names the right
//! op id with the wrong recording or kind preserves the entry it names).
//!
//! The delivery's own effects do not run: in the shell a token has one consumer, the
//! flight that registered it, and its guard retires the op when the flight ends, so a
//! duplicate or late delivery is a decision the owner no longer acts on.

use super::model::*;
use super::task::*;
use crate::decision::{Continuation, OpStatus, SiteId};
use crate::ids::{OperationId, RecordingId};

pub struct Redeliver {
    token: Continuation,
    site: SiteId,
    build: Build,
    delay: u8,
}

impl Redeliver {
    pub fn new(token: Continuation, site: SiteId, _rid: Rid, build: Build, delay: u8) -> Self {
        Self {
            token,
            site,
            build,
            delay,
        }
    }
}

impl Task for Redeliver {
    fn name(&self) -> &'static str {
        "redeliver"
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Poll {
        if self.delay > 0 {
            self.delay -= 1;
            return Poll::Progress;
        }
        if cx.w.op_status(self.token) != OpStatus::Unknown {
            cx.out.violations.push(format!(
                "SP7: a retired token of {:?} is outstanding again",
                self.site
            ));
            return Poll::Done;
        }
        let ops_before = cx.w.history.ops.clone();
        let unknown = (self.build)(cx.w, OpStatus::Unknown);
        let outstanding = (self.build)(cx.w, OpStatus::Outstanding);
        if unknown != outstanding {
            cx.out.violations.push(format!(
                "SP7: the late delivery of {:?} differs by op status in a later world: {} ({:?}) vs {} ({:?})",
                self.site, unknown.row.0, unknown.verdict, outstanding.row.0, outstanding.verdict
            ));
        }
        if let Err(error) = unknown.validate() {
            cx.out.violations.push(format!(
                "{}: invalid late decision {error:?}",
                unknown.row.0
            ));
        }
        let mut retired = cx.w.retire(self.token);
        for (op, (rid, kind)) in &ops_before {
            // The right op id with the wrong recording: never retires the entry.
            retired |= cx.w.retire(Continuation {
                recording_id: RecordingId(rid + 1_000),
                op: OperationId(*op),
                kind: *kind,
            });
        }
        if retired || cx.w.history.ops != ops_before {
            cx.out.violations.push(format!(
                "SP7: the late delivery of {} changed the outstanding ops",
                unknown.row.0
            ));
        }
        Poll::Done
    }
}
