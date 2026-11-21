//! `murmur-dictation-core`: the pure dictation decision core.
//!
//! The crate holds ids, typed inputs, effects with their placement classes,
//! replies, the C1 state vocabulary ([`state::PersistedCore`],
//! [`state::DictationCore`], [`state::LockFreeFacts`]), admission
//! ([`admission::Occupancy`], [`admission::admit`]), the decide functions per
//! family (start, stop, cancel, lifecycle, `process_audio`), and the single
//! entry point [`state::DictationCore::handle`] that routes an input to its
//! decide function. A property harness drives it with a deterministic seed
//! gate.
//!
//! It has **no dependency** on tauri or any app crate, performs **no I/O**,
//! takes **no locks** and reads **no clock**: time enters as [`ids::Now`]
//! values and every fact enters as a plain field. The shell (`ui_lib`) owns the
//! locks and the executor that runs the effects this crate decides.
//!
//! See `README.md` for the module → appendix C map, the C6.4 claim protocols
//! and the places the code overrides the appendix.

#![forbid(unsafe_code)]

pub mod admission;
pub mod cancel;
pub mod decision;
pub mod effects;
pub mod handle;
pub mod ids;
pub mod inputs;
pub mod lifecycle;
pub mod process_audio;
pub mod replies;
pub mod rows;
pub mod start;
pub mod state;
pub mod status;
pub mod stop;

#[cfg(test)]
mod property;
