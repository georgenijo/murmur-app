//! Main-thread-only AppKit capabilities. No private user content is logged.
use super::{
    lifecycle::Lifecycle,
    menu::{Intent, TrayState},
};
use objc2::{MainThreadMarker, class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSStatusBar, NSStatusItem};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    sync::mpsc,
};
unsafe extern "C" {
    fn murmur_host_install_termination(
        callback: extern "C" fn(*mut c_void) -> isize,
        context: *mut c_void,
    ) -> bool;
    fn murmur_host_remove_termination(context: *mut c_void);
    fn murmur_host_reply_termination() -> bool;
    fn murmur_host_target(
        callback: extern "C" fn(*mut c_void, *const c_char),
        context: *mut c_void,
    ) -> *mut AnyObject;
    fn murmur_host_menu(json: *const c_char, target: *mut AnyObject) -> *mut AnyObject;
    fn murmur_host_attach_tray(item: *mut c_void, menu: *mut AnyObject, target: *mut AnyObject);
    fn murmur_host_window_visible(view: *mut c_void, visible: bool);
    fn murmur_host_window_is_visible(view: *mut c_void) -> bool;
}
fn main_thread() {
    MainThreadMarker::new().expect("native host requires AppKit main thread");
}
extern "C" fn termination(context: *mut c_void) -> isize {
    // SAFETY: the installation retains this box until the native hook is removed.
    let lifecycle = unsafe { &*(context as *const Lifecycle) };
    if lifecycle.permitted() {
        1
    } else {
        lifecycle.request(murmur_core::shutdown::Request::System);
        2
    }
}
pub struct Termination {
    context: Box<Lifecycle>,
}
impl Termination {
    pub fn install(lifecycle: Lifecycle) -> Result<Self, &'static str> {
        main_thread();
        let mut context = Box::new(lifecycle);
        // SAFETY: native adds one missing selector, never replaces the delegate.
        if !unsafe {
            murmur_host_install_termination(termination, (&mut *context as *mut Lifecycle).cast())
        } {
            return Err("Native termination admission hook unavailable");
        }
        Ok(Self { context })
    }
    pub fn reply(&self) -> bool {
        main_thread();
        unsafe { murmur_host_reply_termination() }
    }
}
impl Drop for Termination {
    fn drop(&mut self) {
        main_thread();
        unsafe { murmur_host_remove_termination((&mut *self.context as *mut Lifecycle).cast()) };
    }
}
extern "C" fn menu_action(context: *mut c_void, id: *const c_char) {
    // SAFETY: target and sender are retained together by Tray; the ID is borrowed
    // for the synchronous callback. A full inbox refuses further user actions.
    let sender = unsafe { &*(context as *const mpsc::SyncSender<Intent>) };
    if !id.is_null()
        && let Ok(id) = unsafe { CStr::from_ptr(id) }.to_str()
        && let Some(intent) = Intent::parse(id)
    {
        let _ = sender.try_send(intent);
    }
}
pub struct Tray {
    item: Retained<NSStatusItem>,
    target: Retained<AnyObject>,
    _sender: Box<mpsc::SyncSender<Intent>>,
    state: Option<TrayState>,
}
impl Tray {
    pub fn new() -> (Self, mpsc::Receiver<Intent>) {
        main_thread();
        let (sender, receiver) = mpsc::sync_channel(32);
        let mut sender = Box::new(sender);
        // SAFETY: all AppKit work is on main; native target adopts no Rust pointer.
        let target = unsafe {
            Retained::from_raw(murmur_host_target(
                menu_action,
                (&mut *sender as *mut mpsc::SyncSender<Intent>).cast(),
            ))
        }
        .expect("tray target");
        let item: Retained<NSStatusItem> =
            unsafe { msg_send![&*NSStatusBar::systemStatusBar(), statusItemWithLength: -1.0f64] };
        let button: *mut AnyObject = unsafe { msg_send![&*item, button] };
        let symbol = NSString::from_str("mic.fill");
        let label = NSString::from_str("Murmur");
        unsafe {
            let image: *mut AnyObject = msg_send![class!(NSImage), imageWithSystemSymbolName:&*symbol, accessibilityDescription:&*label];
            let _: () = msg_send![image, setTemplate:true];
            let _: () = msg_send![button,setImage:image];
            let _: () = msg_send![button,setToolTip:&*label];
        }
        (
            Self {
                item,
                target,
                _sender: sender,
                state: None,
            },
            receiver,
        )
    }
    pub fn update(&mut self, state: TrayState) -> bool {
        main_thread();
        if self
            .state
            .as_ref()
            .is_some_and(|old| old.revision > state.revision)
        {
            return false;
        }
        if self.state.as_ref() == Some(&state) {
            return true;
        }
        let Ok(json) = CString::new(state.menu().to_string()) else {
            return false;
        };
        let Some(menu) = (unsafe {
            Retained::from_raw(murmur_host_menu(
                json.as_ptr(),
                Retained::as_ptr(&self.target).cast_mut(),
            ))
        }) else {
            return false;
        };
        unsafe {
            murmur_host_attach_tray(
                Retained::as_ptr(&self.item).cast_mut().cast(),
                Retained::as_ptr(&menu).cast_mut(),
                Retained::as_ptr(&self.target).cast_mut(),
            );
        }
        self.state = Some(state);
        true
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        main_thread();
        unsafe {
            let _: () = msg_send![&*NSStatusBar::systemStatusBar(),removeStatusItem:&*self.item];
        }
    }
}
fn view(window: &gpui::Window) -> Option<*mut c_void> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr()),
        _ => None,
    }
}
pub fn visible(window: &gpui::Window) -> bool {
    main_thread();
    view(window).is_some_and(|v| unsafe { murmur_host_window_is_visible(v) })
}
pub fn show(window: &gpui::Window, visible: bool) {
    main_thread();
    if let Some(v) = view(window) {
        unsafe { murmur_host_window_visible(v, visible) }
    }
}
