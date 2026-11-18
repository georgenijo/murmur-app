//! Shadow mode tests (appendix C8.7, decision D-S2c-1), driven by synthetic headers
//! and messages.
//!
//! A [`harness::Flight`] is a *simulated live path*: it takes a view and an input,
//! asks the core what it decides, and plays that decision back as the messages a
//! faithful live path would send (a header with the `Pre` and `In` witnesses, the
//! context slot before and after, the ticker gate inputs and the history operations,
//! then one incremental message per later effect, the reply and the close). Each test
//! then damages exactly one thing and asserts the first failing level.
//!
//! Tests that need the real capture sites (a live executor feeding the recorder, the
//! E7 golden replay through the synchronous mode) are owned by Stage 2c-2; the ones
//! here drive the comparison and the transport with synthetic data only.

mod capture;
mod closing;
mod comparison;
mod conditionals;
mod harness;
mod history;
mod loss;
mod output;
mod pd_credit;
mod policy;
mod ring;
mod wrong_slot;
