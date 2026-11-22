//! The event-name constants emitted outside the pinned host wrappers (D-S2b-14).
//!
//! The SP5 ratchet (`dictation_executor/ratchet_tests.rs`) resolves an emitted event
//! name only as a string literal, a `concat!` of literals, or a path written in full
//! as `crate::events::NAME`. This module therefore holds only
//! `pub const NAME: &str = "<literal>";` items: no `use`, no `cfg`, no macro, no
//! nested module, each name once.

/// Emitted to each owner window after every main-thread-watch change, with the new
/// status.
pub const MAIN_THREAD_WATCH_STATUS_EVENT: &str = "main-thread-watch-status";

/// The audio input inventory changed (emitted to `main` and `overlay`).
pub const INVENTORY_EVENT: &str = "audio-input-inventory-changed";

/// Raw RMS stream for the main window (Stop on Silence, the recording bar,
/// the corpus recorder). Emitted with `emit_to("main", ..)`.
pub const AUDIO_LEVEL_EVENT: &str = "audio-level";

/// Peak-held waveform feed for the overlay window only. It has its own name
/// because a frontend `listen()` registers for any target, and Tauri
/// delivers an `emit_to` for one label to every webview that has such a
/// listener: sharing `audio-level` would hand each window both streams.
pub const OVERLAY_AUDIO_LEVEL_EVENT: &str = "overlay-audio-level";

/// The selected transcription model cannot serve a dictation (not downloaded,
/// incomplete, downloading, unsupported, or failed to load). Payload:
/// `{ reason, phase }`; broadcast so the overlay can show it.
pub const MODEL_UNAVAILABLE_EVENT: &str = "dictation-model-unavailable";
