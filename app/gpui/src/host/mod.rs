//! Native host boundary. Gallery binaries never construct this host.
pub mod lifecycle;
pub mod paths;
pub mod worker;

pub mod menu;
#[cfg(target_os = "macos")]
pub mod native;

#[cfg(target_os = "macos")]
pub mod shell;
