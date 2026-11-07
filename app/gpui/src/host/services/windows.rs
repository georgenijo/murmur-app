//! Only snapshots cross threads. AppKit identities live in a weak main-thread
//! registry and every queued command carries the original registration epoch.
use gpui::{AnyWindowHandle, App};
use murmur_services::native_services::{
    self as service, Monitor, PhysicalPosition as Position, PhysicalSize as Size, WindowCommand,
    WindowPort, WindowSnapshot, WorkArea,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
unsafe extern "C" {
    fn murmur_services_window_register(
        view: *mut c_void,
        label: *const c_char,
        generation: u64,
    ) -> bool;
    fn murmur_services_window_unregister(label: *const c_char, generation: u64);
    fn murmur_services_window(label: *const c_char, generation: u64) -> *mut c_void;
    fn murmur_services_window_snapshot(label: *const c_char, generation: u64) -> *mut c_char;
    fn murmur_services_displays() -> *mut c_char;
    fn murmur_services_free(value: *mut c_char);
    fn murmur_services_window_command(
        label: *const c_char,
        generation: u64,
        command: *const c_char,
    ) -> bool;
}
fn main() {
    objc2::MainThreadMarker::new().expect("native window main thread");
}
unsafe fn take(raw: *mut c_char) -> Option<Value> {
    if raw.is_null() {
        return None;
    }
    let value = serde_json::from_slice(unsafe { CStr::from_ptr(raw) }.to_bytes()).ok();
    unsafe { murmur_services_free(raw) };
    value
}
fn rect(v: &Value) -> Option<(Position<i32>, Size<u32>)> {
    let a = v.as_array()?;
    let get = |i: usize| a.get(i)?.as_f64().filter(|v| v.is_finite());
    let (x, y, w, h) = (get(0)?, get(1)?, get(2)?, get(3)?);
    if x < i32::MIN as f64
        || x > i32::MAX as f64
        || y < i32::MIN as f64
        || y > i32::MAX as f64
        || w < 0.
        || w > u32::MAX as f64
        || h < 0.
        || h > u32::MAX as f64
    {
        return None;
    }
    Some((
        Position::new(x.round() as i32, y.round() as i32),
        Size::new(w.round() as u32, h.round() as u32),
    ))
}
fn monitor(v: &Value) -> Option<Monitor> {
    let (position, size) = rect(&v["frame"])?;
    let (p, s) = rect(&v["work"])?;
    Some(Monitor {
        position,
        size,
        work: WorkArea {
            position: p,
            size: s,
        },
        name: v["name"].as_str().map(String::from),
        scale: v["scale"].as_f64()?,
    })
}
fn snapshot(v: &Value) -> Option<WindowSnapshot> {
    let (position, outer_size) = rect(&v["outer"])?;
    let (inner_position, size) = rect(&v["inner"])?;
    Some(WindowSnapshot {
        visible: v["visible"].as_bool()?,
        minimized: v["minimized"].as_bool()?,
        fullscreen: v["fullscreen"].as_bool()?,
        maximized: v["maximized"].as_bool()?,
        size,
        inner_position,
        outer_size,
        position,
        scale: v["scale"].as_f64()?,
        monitor: monitor(&v["monitor"]),
    })
}
pub struct Command {
    label: String,
    generation: u64,
    command: WindowCommand,
}
pub struct Port {
    label: String,
    c_label: CString,
    generation: u64,
    snapshot: Mutex<WindowSnapshot>,
    send: mpsc::SyncSender<Command>,
}
impl WindowPort for Port {
    fn label(&self) -> &str {
        &self.label
    }
    fn snapshot(&self) -> WindowSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    fn command(&self, command: WindowCommand) -> Result<(), service::Error> {
        if matches!(self.label.as_str(), "overlay" | "dictation-preview")
            && matches!(
                command,
                WindowCommand::Size(_)
                    | WindowCommand::Position(_)
                    | WindowCommand::Show
                    | WindowCommand::Hide
                    | WindowCommand::Background(_)
            )
        {
            // The native Overlay/Live Words owners apply Rust geometry and
            // visibility from generation-fenced events on the main thread.
            return Ok(());
        }
        self.send
            .try_send(Command {
                label: self.label.clone(),
                generation: self.generation,
                command,
            })
            .map_err(|_| service::Error("Window queue unavailable".into()))
    }
    fn ns_window(&self) -> Result<*mut c_void, service::Error> {
        main();
        let ptr = unsafe { murmur_services_window(self.c_label.as_ptr(), self.generation) };
        if ptr.is_null() {
            Err(service::Error("Window unavailable".into()))
        } else {
            Ok(ptr)
        }
    }
}
#[derive(Default)]
pub struct Registry {
    pub ports: Mutex<BTreeMap<String, Arc<Port>>>,
    pub displays: Mutex<(Vec<Monitor>, Option<Monitor>)>,
    next: AtomicU64,
}
impl Registry {
    pub fn register(
        &self,
        label: &str,
        window: &gpui::Window,
        send: mpsc::SyncSender<Command>,
    ) -> Result<(), String> {
        main();
        let RawWindowHandle::AppKit(raw) = HasWindowHandle::window_handle(window)
            .map_err(|_| "Window unavailable")?
            .as_raw()
        else {
            return Err("Window unavailable".into());
        };
        let label_c = CString::new(label).map_err(|_| "Invalid window")?;
        let generation = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        if !unsafe {
            murmur_services_window_register(raw.ns_view.as_ptr(), label_c.as_ptr(), generation)
        } {
            return Err("Window unavailable".into());
        }
        let snapshot = unsafe {
            take(murmur_services_window_snapshot(
                label_c.as_ptr(),
                generation,
            ))
        }
        .and_then(|v| snapshot(&v))
        .ok_or("Window snapshot unavailable")?;
        self.ports.lock().unwrap_or_else(|p| p.into_inner()).insert(
            label.into(),
            Arc::new(Port {
                label: label.into(),
                c_label: label_c,
                generation,
                snapshot: Mutex::new(snapshot),
                send,
            }),
        );
        self.refresh_displays();
        Ok(())
    }
    pub fn refresh_displays(&self) {
        main();
        if let Some(v) = unsafe { take(murmur_services_displays()) } {
            let all = v["all"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(monitor)
                .collect();
            *self.displays.lock().unwrap_or_else(|p| p.into_inner()) =
                (all, monitor(&v["primary"]));
        }
    }
    pub fn refresh(&self, label: &str) -> Option<(WindowSnapshot, WindowSnapshot)> {
        main();
        let port = self
            .ports
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(label)
            .cloned()?;
        let new = unsafe {
            take(murmur_services_window_snapshot(
                port.c_label.as_ptr(),
                port.generation,
            ))
        }
        .and_then(|v| snapshot(&v))?;
        let old = std::mem::replace(
            &mut *port.snapshot.lock().unwrap_or_else(|p| p.into_inner()),
            new.clone(),
        );
        Some((old, new))
    }
    pub fn apply(&self, command: Command, handle: AnyWindowHandle, cx: &mut App) {
        main();
        let port = self
            .ports
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&command.label)
            .cloned();
        let Some(port) = port.filter(|p| p.generation == command.generation) else {
            return;
        };
        if matches!(&command.command, WindowCommand::Destroy) {
            unsafe { murmur_services_window_unregister(port.c_label.as_ptr(), port.generation) };
            self.ports
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&command.label);
            let _ = handle.update(cx, |_, w, _| w.remove_window());
            return;
        }
        if let WindowCommand::Zoom(zoom) = command.command {
            if zoom.is_finite() && (0.5..=3.).contains(&zoom) {
                let _ = handle.update(cx, |_, w, cx| {
                    w.set_ui_zoom(zoom as f32, cx);
                });
            }
            return;
        }
        let value = match command.command {
            WindowCommand::Show => json!({"kind":"show"}),
            WindowCommand::Hide => json!({"kind":"hide"}),
            WindowCommand::Focus => json!({"kind":"focus"}),
            WindowCommand::Minimize => json!({"kind":"minimize"}),
            WindowCommand::Unminimize => json!({"kind":"unminimize"}),
            WindowCommand::Maximize => json!({"kind":"maximize"}),
            WindowCommand::Center => json!({"kind":"center"}),
            WindowCommand::Fullscreen(v) => json!({"kind":"fullscreen","value":v}),
            WindowCommand::Focusable(v) => json!({"kind":"focusable","value":v}),
            WindowCommand::IgnoreCursor(v) => json!({"kind":"cursor","value":v}),
            WindowCommand::Background(v) => {
                json!({"kind":"background","rgba":v.map(|c|[c.0,c.1,c.2,c.3])})
            }
            WindowCommand::Size(service::Size::Physical(v)) => {
                json!({"kind":"size","physical":true,"x":v.width,"y":v.height})
            }
            WindowCommand::Size(service::Size::Logical(v)) => {
                json!({"kind":"size","physical":false,"x":v.width,"y":v.height})
            }
            WindowCommand::Position(service::Position::Physical(v)) => {
                json!({"kind":"position","physical":true,"x":v.x,"y":v.y})
            }
            WindowCommand::Position(service::Position::Logical(v)) => {
                json!({"kind":"position","physical":false,"x":v.x,"y":v.y})
            }
            _ => return,
        };
        if let Ok(json) = CString::new(value.to_string()) {
            let _ = handle.update(cx, |_, w, cx| {
                if unsafe {
                    murmur_services_window_command(
                        port.c_label.as_ptr(),
                        port.generation,
                        json.as_ptr(),
                    )
                } {
                    w.bounds_changed(cx);
                }
            });
        }
    }
    pub fn command_label(command: &Command) -> &str {
        &command.label
    }
}
