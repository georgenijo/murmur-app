//! Permanent standalone legacy importer. No Tauri, wry, or React dependency.
#[cfg(target_os = "macos")]
mod importer;
#[cfg(target_os = "macos")]
pub use importer::*;

#[cfg(target_os = "macos")]
mod overlay_retirement;
#[cfg(target_os = "macos")]
pub use overlay_retirement::OverlayRetirement;

/// The same deallocation barrier for the independently replaced preview.
#[cfg(target_os = "macos")]
pub use overlay_retirement::OverlayRetirement as PreviewRetirement;
