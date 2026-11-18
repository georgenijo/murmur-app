//! `murmur-dictation-core`: the pure dictation decision vocabulary and
//! admission policy.
//!
//! Stage 1a of the recording refactor (plan §6, appendix C). The crate holds
//! ids, typed inputs, effects with their placement classes, replies,
//! [`admission::Occupancy`], [`admission::admit`] and the C1 state vocabulary
//! ([`state::PersistedCore`], [`state::DictationCore`], [`state::LockFreeFacts`]).
//! It has **no dependency** on tauri or any app crate, performs **no I/O**, takes **no locks** and reads
//! **no clock**: time enters as [`ids::Now`] values and every fact enters as a
//! plain field. The shell (`ui_lib`) owns the locks, the executor and the
//! production wiring; none of that is here yet (Stage 1b, 2a).
//!
//! See `README.md` for the module → appendix C map, the C6.4 claim protocols
//! and the places the code overrides the appendix.

#![forbid(unsafe_code)]

pub mod admission;
pub mod cancel;
pub mod decision;
pub mod effects;
pub mod ids;
pub mod inputs;
pub mod lifecycle;
pub mod replies;
pub mod rows;
pub mod start;
pub mod state;
pub mod status;
pub mod stop;
