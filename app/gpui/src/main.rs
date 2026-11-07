//! No GPUI platform, NSApplication or Metal object exists before worker dispatch.
use murmur_ui::host::{
    services,
    worker::{self, Workers},
};
use std::process::ExitCode;
#[global_allocator]
static ALLOCATOR: murmur_services::native_services::RustZoneAllocator =
    murmur_services::native_services::RustZoneAllocator;
struct NativeWorkers;
impl Workers for NativeWorkers {
    fn diarization(&self) -> Option<i32> {
        murmur_services::meeting_diarization::run_cli_if_requested()
    }
    fn coreml_install(&self) -> Option<i32> {
        murmur_services::coreml_installer::run_cli_if_requested()
    }
    fn capture_agent(&self) -> Option<i32> {
        murmur_services::capture_agent_probe::run_cli_if_requested()
    }
    fn capture_helper(&self) -> Option<i32> {
        murmur_services::capture_helper_probe::run_cli_if_requested()
    }
}
fn main() -> ExitCode {
    let identifier = services::os::identity();
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return ExitCode::FAILURE;
    };
    if murmur_services::native_services::initialize_identity(&identifier, &home).is_err() {
        return ExitCode::FAILURE;
    }
    if let Some(code) = services::legacy::worker().or_else(|| worker::dispatch(&NativeWorkers)) {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    services::run(identifier, home)
}
