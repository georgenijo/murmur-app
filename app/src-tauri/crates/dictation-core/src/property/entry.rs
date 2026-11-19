//! The harness's one way to decide: through [`DictationCore::handle`] (#1160).
//!
//! Every flight and every supervisor arm hands its typed [`Input`] to
//! `DictationCore::handle` on the view of the model world, so the D / D-less entry
//! choice (`202`/`202n`, `204b`, the `*_nolock` entries) and every
//! [`Verdict::NeedObservation`] round trip are exercised by the 10,000 gated seeds, not
//! only by the unit tests of `handle.rs`. A cutover PR that moves a shell site onto
//! `handle` is then covered by the same seeds that cover the sites today.
//!
//! The flights keep the `decide_*` call they made before as the *cross-check*: each
//! decision `handle` returns must equal what that direct call returns for the same
//! input and the same world. A difference is a property violation of the seed (the
//! harness's own check, reported with the seed like any other), never a panic, and the
//! `handle` decision is the one the flight acts on, so a routing defect also shows up
//! downstream as a wrong effect.
//!
//! The harness answers each observation from model state: the flight reads the fact
//! (`cleanup_pending`, the installed context, the preserved capture) exactly where the
//! shell reads it, puts it into the typed input and calls `handle` again.

use crate::decision::Decision;
use crate::ids::Now;
use crate::inputs::Input;
use crate::state::DictationCore;
use std::cell::RefCell;

/// Decide `input` through `view.handle` and compare the result with `direct`, the
/// decision the flight's own direct `decide_*` call returned for the same input in the
/// same world. Returns the `handle` decision. A difference is pushed to `faults`
/// (drained into the seed's violations by the checker).
pub fn once(
    faults: &RefCell<Vec<String>>,
    view: &DictationCore,
    input: Input,
    at: Now,
    direct: Decision,
) -> Decision {
    let site = input.site();
    let described = format!("{input:?}");
    let routed = view.handle(input, at);
    if routed != direct {
        faults.borrow_mut().push(format!(
            "handle: {site:?} {described} decided {} ({:?}) through `handle` but {} ({:?}) \
             through the direct `decide_*`",
            routed.row.0, routed.verdict, direct.row.0, direct.verdict
        ));
    }
    routed
}
