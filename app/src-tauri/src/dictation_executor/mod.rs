//! The shell side of the dictation core (appendix C1.4.2, C4): the common field
//! adapter, the conversions between shell and core types, and the effect
//! executor.
//!
//! The shell owns every lock scope. Nothing here takes D or T.

pub(crate) mod adapter;
pub(crate) mod convert;
pub(crate) mod executor;
#[cfg(test)]
pub(crate) mod probe;
#[cfg(feature = "dictation-shadow")]
// Shadow layer, removed in #1154: `Flight::none`, `observed` and `is_submitted` have no
// caller in some builds. The allow sits here so `shadow.rs` stays untouched until then.
#[allow(dead_code)]
pub(crate) mod shadow;

pub(crate) use adapter::FieldAdapter;

#[cfg(test)]
mod ratchet_tests;
