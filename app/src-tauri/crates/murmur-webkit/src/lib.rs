//! Permanent standalone legacy importer. No Tauri, wry, or React dependency.
#[cfg(target_os = "macos")]
mod importer;
#[cfg(target_os = "macos")]
pub use importer::*;
