pub(crate) mod detectors;

#[cfg(target_os = "macos")]
mod sys;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{start_listener, stop_listener, set_target_key, set_recording_state, set_processing};

#[cfg(not(target_os = "macos"))]
mod linux;
#[cfg(not(target_os = "macos"))]
pub use linux::{start_listener, stop_listener, set_target_key, set_recording_state, set_processing};
