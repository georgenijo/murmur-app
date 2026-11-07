//! Host-independent services and leaves shared by the Tauri and future GPUI hosts.
pub mod audio_level;
pub mod completion;
pub mod context;
pub mod dictation;
pub mod events;
pub mod hold_promotion;
pub mod history;
pub mod keyboard;
pub mod memory;
pub mod model_artifact;
pub mod model_digest;

pub mod state;

pub mod shutdown;

pub mod source_order;

pub mod updater;

pub mod overlay_geometry;
