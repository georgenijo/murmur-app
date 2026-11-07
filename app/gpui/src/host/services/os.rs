//! Main-thread system services. Panel results and queued effects remain
//! revocable; no payload or native error text is logged.
use super::autostart::Registration;
use murmur_core::context::MainThread;
use murmur_services::native_services::MainThreadQueue;
use serde_json::Value;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::Arc,
};
pub type Live = Arc<dyn Fn() -> bool + Send + Sync>;
#[derive(Clone, Default)]
pub struct OsServices {
    queue: MainThreadQueue,
}
unsafe extern "C" {
    fn murmur_services_identity() -> *mut c_char;
    fn murmur_services_bundle_path() -> *mut c_char;
    fn murmur_services_free(value: *mut c_char);
    fn murmur_services_panel(
        request: *const c_char,
        live: extern "C" fn(*mut c_void) -> bool,
        reply: extern "C" fn(*mut c_void, *const c_char),
        context: *mut c_void,
    );
    fn murmur_services_open(value: *const c_char, file: bool) -> bool;
    fn murmur_services_login_status() -> i32;
    fn murmur_services_login_set(enabled: bool) -> bool;
    fn murmur_services_notifications(activate: extern "C" fn()) -> bool;
    fn murmur_services_notify(version: *const c_char, request_permission: bool);
}
unsafe fn take_string(raw: *mut c_char) -> String {
    if raw.is_null() {
        return String::new();
    }
    let value = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { murmur_services_free(raw) };
    value
}
/// Foundation metadata only. Call before any worker or GPUI platform setup.
pub fn identity() -> String {
    unsafe { take_string(murmur_services_identity()) }
}
pub fn bundle_path() -> std::path::PathBuf {
    unsafe { take_string(murmur_services_bundle_path()) }.into()
}
struct Panel {
    live: Live,
    reply: tokio::sync::oneshot::Sender<Value>,
}
extern "C" fn live(context: *mut c_void) -> bool {
    let context = unsafe { &*(context as *const Panel) };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (context.live)())).unwrap_or(false)
}
extern "C" fn reply(context: *mut c_void, text: *const c_char) {
    // The native completion consumes this allocation exactly once, including
    // cancellation. No Rust panic crosses the C boundary.
    let context = unsafe { Box::from_raw(context as *mut Panel) };
    let value = if text.is_null() {
        Value::Null
    } else {
        serde_json::from_slice(unsafe { CStr::from_ptr(text) }.to_bytes()).unwrap_or(Value::Null)
    };
    let _ = context.reply.send(value);
}
impl OsServices {
    pub async fn panel(&self, request: Value, current: Live) -> Result<Option<Value>, String> {
        if !current() {
            return Ok(None);
        }
        let request = CString::new(request.to_string()).map_err(|_| "Invalid panel request")?;
        let (send, receive) = tokio::sync::oneshot::channel();
        let context = Box::new(Panel {
            live: current,
            reply: send,
        });
        self.queue
            .dispatch(Box::new(move || unsafe {
                murmur_services_panel(request.as_ptr(), live, reply, Box::into_raw(context).cast());
            }))
            .map_err(|_| "Panel unavailable")?;
        let value = receive.await.map_err(|_| "Panel unavailable")?;
        Ok((!value.is_null()).then_some(value))
    }
    pub async fn open(&self, value: String, file: bool, current: Live) -> Result<(), String> {
        let text = CString::new(value).map_err(|_| "Invalid destination")?;
        self.on_main(move || current() && unsafe { murmur_services_open(text.as_ptr(), file) })
            .await?
            .then_some(())
            .ok_or_else(|| "Destination unavailable".into())
    }
    pub fn open_path(&self, path: std::path::PathBuf) -> Result<(), String> {
        let text = CString::new(path.to_str().ok_or("Invalid destination")?)
            .map_err(|_| "Invalid destination")?;
        self.queue
            .dispatch(Box::new(move || unsafe {
                murmur_services_open(text.as_ptr(), true);
            }))
            .map_err(|_| "Destination unavailable".into())
    }
    pub async fn login_status(&self) -> Result<Registration, String> {
        self.on_main(|| match unsafe { murmur_services_login_status() } {
            0 => Registration::Disabled,
            1 => Registration::Enabled,
            2 => Registration::RequiresApproval,
            _ => Registration::Unavailable,
        })
        .await
    }
    pub async fn login_set(&self, enabled: bool, current: Live) -> Result<(), String> {
        self.on_main(move || current() && unsafe { murmur_services_login_set(enabled) })
            .await?
            .then_some(())
            .ok_or_else(|| "Login registration unavailable".into())
    }
    pub fn notifications(&self, activate: extern "C" fn()) -> Result<(), String> {
        self.queue
            .dispatch(Box::new(move || unsafe {
                murmur_services_notifications(activate);
            }))
            .map_err(|_| "Notifications unavailable".into())
    }
    pub fn notify(&self, version: &str, request_permission: bool) {
        if let Ok(version) = CString::new(version) {
            let _ = self.queue.dispatch(Box::new(move || unsafe {
                murmur_services_notify(version.as_ptr(), request_permission)
            }));
        }
    }
    async fn on_main<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, String> {
        let (send, receive) = tokio::sync::oneshot::channel();
        self.queue
            .dispatch(Box::new(move || {
                let _ = send.send(work());
            }))
            .map_err(|_| "Main thread unavailable")?;
        receive.await.map_err(|_| "Main thread unavailable".into())
    }
}
