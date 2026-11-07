use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use std::sync::atomic::{AtomicBool, Ordering};
use tao::platform::macos::{set_application_termination_handler, ApplicationTerminationReply};
static PENDING_REPLY: AtomicBool = AtomicBool::new(false);
pub(super) fn install(app: tauri::AppHandle) -> Result<(), String> {
    set_application_termination_handler(move || {
        if super::permitted(&app) {
            return ApplicationTerminationReply::Now;
        }
        PENDING_REPLY.store(true, Ordering::Release);
        // Native termination always wins over a pending restart, including
        // logout/restart/shutdown Apple events. No event content is retained.
        super::request(&app, murmur_core::shutdown::Request::System);
        ApplicationTerminationReply::Later
    })
    .map_err(str::to_owned)
}
pub(super) fn reply_if_pending() -> bool {
    if !PENDING_REPLY.swap(false, Ordering::AcqRel) {
        return false;
    }
    let mtm = MainThreadMarker::new().expect("shutdown reply requires main");
    let app = NSApplication::sharedApplication(mtm);
    app.replyToApplicationShouldTerminate(true);
    true
}
