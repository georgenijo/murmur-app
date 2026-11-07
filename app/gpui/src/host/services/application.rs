//! GPUI is the process host. Cross-thread ports carry snapshots and bounded
//! intents; GPUI entities and AppKit identities remain on the main thread.
use super::{
    backend::{Backend, Events, Route, Stop},
    os::{self, OsServices},
    root,
    windows::{Command, Registry},
};
use crate::host::{
    menu::{Intent, TrayState},
    native,
    paths::DataPaths,
    shell::ApplicationHost,
};
use gpui::*;
use murmur_core::{
    context::{
        AppContext as CoreContext, BackgroundFuture, BackgroundWork, ContextError,
        PermissionStatus, Permissions, Task as CoreTask,
    },
    dictation::Status,
    events::{Consumer, WaveformConsumer},
    overlay_geometry::*,
};
use murmur_services::native_services::{
    self as service, DockDisplay, Monitor, NativePorts, Services, WindowPort,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};
struct Runtime(tokio::runtime::Handle);
impl BackgroundWork for Runtime {
    fn spawn(&self, f: BackgroundFuture) -> Result<(), ContextError> {
        self.0.spawn(f);
        Ok(())
    }
    fn spawn_blocking(&self, t: CoreTask) -> Result<(), ContextError> {
        self.0.spawn_blocking(t);
        Ok(())
    }
}
struct Permission;
impl Permissions for Permission {
    fn microphone_status(&self) -> PermissionStatus {
        service::microphone_status()
    }
}
struct Ports {
    registry: Arc<Registry>,
    events: Arc<Events>,
    os: OsServices,
    terminal: mpsc::Sender<murmur_core::shutdown::Action>,
    services: OnceLock<Services>,
    tray: Mutex<TrayState>,
    dock: Mutex<Option<(DockDisplay, bool)>>,
    lifecycle: AtomicU64,
    revision: AtomicU64,
    overlay_clock: OnceLock<std::time::Instant>,
    refresh: AtomicBool,
    overlay: OnceLock<crate::overlay::inbox::Inbox>,
    preview: OnceLock<crate::preview::inbox::Inbox>,
    downloads: Arc<super::downloads::Downloads>,
    microphone: Arc<super::microphone::Preview>,
    playback: Arc<super::meetings::Presentation>,
    scan: Arc<Mutex<Option<(String, Value)>>>,
    levels: Arc<Mutex<Option<(u64, f32)>>>,
}
fn status(v: &Value) -> Status {
    match v.as_str() {
        Some("starting") => Status::Starting,
        Some("recording") => Status::Recording,
        Some("processing") => Status::Processing,
        Some("recovering") => Status::Recovering,
        _ => Status::Idle,
    }
}
impl NativePorts for Ports {
    fn window(&self, label: &str) -> Option<Arc<dyn WindowPort>> {
        self.registry
            .ports
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(label)
            .cloned()
            .map(|p| p as Arc<dyn WindowPort>)
    }
    fn monitors(&self) -> Vec<Monitor> {
        self.registry
            .displays
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .0
            .clone()
    }
    fn primary_monitor(&self) -> Option<Monitor> {
        self.registry
            .displays
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .1
            .clone()
    }
    fn send(&self, label: &str, event: &str, payload: &str) -> Result<(), service::Error> {
        self.events.publish(label, event, payload);
        if label == "main"
            && let Ok(value) = serde_json::from_str::<Value>(payload)
        {
            if event == "audio-level"
                && let Some(level) = value.as_f64().filter(|v| v.is_finite())
                && let Some(services) = self.services.get()
            {
                *self.levels.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some((services.recording_generation(), level.clamp(0., 1.) as f32));
            }
            if event == "microphone-preview-level"
                && let Some(id) = value["previewId"].as_u64()
                && let Some(rms) = value["rms"].as_f64().filter(|v| v.is_finite())
                && let Some(peak) = value["peak"].as_f64().filter(|v| v.is_finite())
            {
                *self
                    .microphone
                    .meter
                    .lock()
                    .unwrap_or_else(|p| p.into_inner()) =
                    Some((id, rms.clamp(0., 1.) as f32, peak.clamp(0., 1.) as f32));
            }
        }
        if event == "vocab-scan-progress"
            && let Ok(value) = serde_json::from_str::<Value>(payload)
            && let Some((id, progress)) =
                self.scan.lock().unwrap_or_else(|p| p.into_inner()).as_mut()
            && value["scanId"] == *id
        {
            *progress = value;
        }
        if event == "download-progress"
            && let Ok(value) = serde_json::from_str::<Value>(payload)
        {
            let mut downloads = self.downloads.0.lock().unwrap_or_else(|p| p.into_inner());
            for a in downloads
                .values_mut()
                .filter(|a| value["requestId"] == a.id && value["modelName"] == a.model)
            {
                a.progress(&value);
            }
        }
        if label == "overlay" || label == "dictation-preview" {
            let value: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
            let generation = self
                .services
                .get()
                .map_or(0, Services::recording_generation);
            if matches!(
                event,
                "recording-status-changed"
                    | "dictation-generation-started"
                    | "recording-cancelled"
                    | "dictation-lock-changed"
                    | "app-disabled-changed"
                    | "mode-runtime-changed"
                    | "live-words-consent-persisted"
                    | "overlay-settings-persisted"
                    | "meeting-status-changed"
            ) {
                self.lifecycle.fetch_add(1, Ordering::SeqCst);
                self.refresh.store(true, Ordering::Release);
            }
            let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
            if label == "overlay"
                && let Some(inbox) = self.overlay.get()
            {
                use crate::overlay::model::{Delivery, Event, Failure};
                let notification = match event {
                    "audio-level" => {
                        if let Some(level) = value.as_f64() {
                            inbox.level(generation, level as f32);
                        }
                        None
                    }
                    "recording-status-changed" => Some(Event::Lifecycle {
                        generation,
                        status: status(&value),
                        at_ms: self
                            .overlay_clock
                            .get()
                            .map_or(0, |start| start.elapsed().as_millis() as u64),
                    }),
                    "recording-cancelled" => Some(Event::Cancelled(
                        value["recordingId"].as_u64().unwrap_or(generation),
                    )),
                    "hotkey-tap-rejected" => Some(Event::TapMissed),
                    "audio-initialization-stalled" => Some(Event::Stalled(generation)),
                    "recording-initialization-failed" => {
                        Some(Event::Failure(generation, Failure::Retry))
                    }
                    "dictation-delivery-outcome" => Some(Event::Delivery(
                        generation,
                        match value["outcome"].as_str() {
                            Some("autoPastePosted") => Delivery::Pasted,
                            Some("clipboardOnly") => Delivery::Clipboard,
                            _ => Delivery::Retry,
                        },
                    )),
                    "overlay-visible-changed" => value.as_bool().map(Event::Visible),
                    "meeting-suggestion-changed" => {
                        Some(Event::Suggestion(value.as_object().and_then(|_| {
                            Some(crate::overlay::model::Suggestion {
                                token: value["token"].as_str()?.into(),
                                title: value["title"].as_str()?.into(),
                                busy: false,
                                failed: false,
                            })
                        })))
                    }
                    _ => None,
                };
                if let Some(event) = notification
                    && inbox.publish(event).is_err()
                {
                    self.refresh.store(true, Ordering::Release);
                }
            }
            if label == "dictation-preview"
                && let Some(inbox) = self.preview.get()
            {
                use crate::preview::model::{Event, LifecycleSnapshot};
                let notification = match event {
                    "dictation-partial" => value["recordingId"]
                        .as_u64()
                        .zip(value["text"].as_str())
                        .map(|(generation, text)| Event::Partial {
                            generation,
                            text: text.into(),
                        }),
                    "recording-status-changed" => Some(Event::Lifecycle(LifecycleSnapshot {
                        revision,
                        generation,
                        status: status(&value),
                    })),
                    "recording-cancelled" => Some(Event::Cancelled(
                        value["recordingId"].as_u64().unwrap_or(generation),
                    )),
                    "live-words-consent-persisted" => Some(Event::ConsentChanged {
                        opted_out: value != true,
                    }),
                    _ => None,
                };
                if let Some(event) = notification
                    && inbox.publish(event).is_err()
                {
                    inbox.invalidate();
                    // Fail closed under lifecycle backpressure. A fresh read
                    // rehydrates consent/lifecycle; provisional text is discarded.
                    self.refresh.store(true, Ordering::Release);
                }
            }
        }
        Ok(())
    }
    fn quiesce(&self, deadline: std::time::Instant) -> bool {
        self.playback.quiesce(deadline)
    }
    fn terminal(&self, action: murmur_core::shutdown::Action) {
        let _ = self.terminal.send(action);
    }
    fn notify_update(&self, version: &str) {
        self.os.notify(version, true);
    }
    fn dock_context(&self) -> Option<(DockDisplay, bool)> {
        *self.dock.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn dock_changed(&self) {
        self.refresh.store(true, Ordering::Release);
    }
    fn mode_status(&self, value: Value) {
        let mut tray = self.tray.lock().unwrap_or_else(|p| p.into_inner());
        tray.revision = value["pendingRevision"].as_u64().unwrap_or(0);
        tray.mode = value["name"].as_str().unwrap_or("Everyday").into();
        tray.pending_mode = value["pending"]["id"].as_str().map(String::from);
        tray.available_modes = value["available"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| Some((m["id"].as_str()?.into(), m["name"].as_str()?.into())))
            .collect();
    }
    fn open_path(&self, path: PathBuf) -> Result<(), service::Error> {
        self.os.open_path(path).map_err(service::Error)
    }
    fn open_url(&self, url: String, lease: service::EffectLease) -> Result<(), service::Error> {
        let runtime = super::application::native_runtime()
            .ok_or_else(|| service::Error("Native runtime unavailable".into()))?;
        let os = self.os.clone();
        runtime.spawn(async move {
            let _ = os.open(url, false, lease).await;
        });
        Ok(())
    }
    fn tray_update(&self, version: Option<String>) -> Result<(), service::Error> {
        self.tray.lock().unwrap_or_else(|p| p.into_inner()).update = version;
        Ok(())
    }
}
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();
fn native_runtime() -> Option<tokio::runtime::Handle> {
    RUNTIME.get().cloned()
}
static NOTIFICATION_ROUTE: OnceLock<mpsc::SyncSender<Route>> = OnceLock::new();
extern "C" fn notification() {
    if let Some(send) = NOTIFICATION_ROUTE.get() {
        let _ = send.try_send(Route::Updates);
    }
}
struct Host {
    backend: Backend,
    ports: Arc<Ports>,
    command_send: mpsc::SyncSender<Command>,
    commands: mpsc::Receiver<Command>,
    routes: mpsc::Receiver<Route>,
    handles: RefCell<BTreeMap<&'static str, AnyWindowHandle>>,
    main: RefCell<Option<Entity<root::Main>>>,
    focused: Cell<bool>,
    diagnostics: RefCell<Option<Entity<crate::diagnostics::Diagnostics>>>,
    overlay: RefCell<Option<Entity<crate::overlay::Overlay>>>,
    preview: RefCell<Option<Entity<crate::preview::Preview>>>,
    _subscriptions: RefCell<Vec<Stop>>,
}
impl Host {
    fn register(&self, label: &'static str, w: &Window) {
        self.ports
            .registry
            .register(label, w, self.command_send.clone())
            .expect("native window registration");
        self.handles.borrow_mut().insert(label, w.window_handle());
    }
    fn companions(self: &Rc<Self>, cx: &mut App) {
        let host = self.clone();
        let overlay = cx
            .open_window(
                WindowOptions {
                    kind: WindowKind::PopUp,
                    titlebar: None,
                    show: false,
                    focus: false,
                    is_resizable: false,
                    is_movable: false,
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(300.), px(60.)),
                    ))),
                    ..Default::default()
                },
                move |w, cx| {
                    host.register("overlay", w);
                    let view = cx.new(|cx| {
                        let view = crate::overlay::Overlay::new(
                            Default::default(),
                            Rc::new(OverlayHost {
                                backend: host.backend.clone(),
                                ports: host.ports.clone(),
                            }),
                        );
                        view.follow(w, cx);
                        view
                    });
                    let _ = host.ports.overlay_clock.set(std::time::Instant::now());
                    let _ = host.ports.overlay.set(view.read(cx).inbox.clone());
                    host.overlay.replace(Some(view.clone()));
                    cx.new(|cx| gpui_base::Root::new(view, w, cx).bg(transparent_black()))
                },
            )
            .expect("native overlay");
        let host = self.clone();
        let preview = cx
            .open_window(
                crate::preview::Preview::window_options(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(460.), px(168.)),
                )),
                move |w, cx| {
                    host.register("dictation-preview", w);
                    let view = cx.new(|cx| {
                        let view = crate::preview::Preview::new(Rc::new(PreviewHost(
                            host.backend.services.clone(),
                        )));
                        view.follow(w, cx);
                        view
                    });
                    let _ = host.ports.preview.set(view.read(cx).inbox.clone());
                    host.preview.replace(Some(view.clone()));
                    cx.new(|cx| crate::preview::Preview::root(view, w, cx))
                },
            )
            .expect("native Live Words window");
        let _ = (overlay, preview);
        self.hydrate_companions(cx);
    }
    fn hydrate_companions(&self, cx: &App) {
        let observed_overlay = self
            .overlay
            .borrow()
            .as_ref()
            .map_or(0, |v| v.read(cx).model.hydration_epoch());
        let observed_consent = self
            .preview
            .borrow()
            .as_ref()
            .map_or(0, |v| v.read(cx).model.consent_epoch());
        let observed_lifecycle = self
            .preview
            .borrow()
            .as_ref()
            .map_or(0, |v| v.read(cx).model.lifecycle_epoch());
        let b = self.backend.clone();
        let ports = self.ports.clone();
        self.backend.spawn(async move {
            let epoch = ports.lifecycle.load(Ordering::Acquire);
            let result = async {
                let (settings_revision, settings) = b.services.settings_snapshot().await?;
                let state = b.call("get_status", json!({})).await?;
                let meeting = b.call("get_meeting_status", json!({})).await?;
                let generation = state["recordingId"].as_u64().unwrap_or(0);
                if ports.lifecycle.load(Ordering::Acquire) != epoch
                    || b.services.recording_generation() != generation
                    || b.services.settings_revision() != settings_revision
                {
                    return Err("Presentation superseded".to_owned());
                }
                ports
                    .tray
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .disabled = settings["disabled"] == true;
                let revision = ports.revision.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(inbox) = ports.overlay.get() {
                    let tray = ports.tray.lock().unwrap_or_else(|p| p.into_inner()).clone();
                    let _ = inbox.publish(crate::overlay::model::Event::Hydrate {
                        observed_epoch: observed_overlay,
                        snapshot: crate::overlay::model::Snapshot {
                            revision,
                            generation,
                            elapsed_ms: state["recordingStartedAtMs"].as_i64().map_or(0, |start| {
                                chrono::Utc::now()
                                    .timestamp_millis()
                                    .saturating_sub(start)
                                    .max(0) as u64
                            }),
                            status: status(&state["state"]),
                            locked: state["locked"] == true,
                            disabled: settings["disabled"] == true,
                            auto_paste: settings["autoPaste"] == true,
                            file_output: settings["saveAudio"] == true
                                || settings["saveTranscript"] == true,
                            hotkey_feedback: settings["hotkeyMissFeedback"] == true,
                            placement: if settings["overlayPlacement"] == "floating_pill" {
                                OverlayPlacement::FloatingPill
                            } else {
                                OverlayPlacement::Notch
                            },
                            offset: settings["overlayVerticalOffset"].as_f64().unwrap_or(0.),
                            mode: tray.mode,
                            next_mode: tray.pending_mode.as_ref().and_then(|id| {
                                tray.available_modes
                                    .iter()
                                    .find(|(key, _)| key == id)
                                    .map(|(_, name)| name.clone())
                            }),
                            meeting: match meeting["phase"].as_str() {
                                Some("starting") => crate::overlay::model::Meeting::Starting,
                                Some("recording") => crate::overlay::model::Meeting::Recording,
                                Some("stopping") => crate::overlay::model::Meeting::Stopping,
                                Some("processing") => crate::overlay::model::Meeting::Processing,
                                Some("failed") => crate::overlay::model::Meeting::Failed,
                                _ => crate::overlay::model::Meeting::Idle,
                            },
                            ..Default::default()
                        },
                    });
                }
                if let Some(inbox) = ports.preview.get() {
                    let _ = inbox.publish(crate::preview::model::Event::HydrateLifecycle {
                        observed_epoch: observed_lifecycle,
                        snapshot: crate::preview::model::LifecycleSnapshot {
                            revision,
                            generation,
                            status: status(&state["state"]),
                        },
                    });
                    let _ = inbox.publish(crate::preview::model::Event::ConsentRead {
                        epoch: observed_consent,
                        enabled: settings["overlayLiveWords"] == true,
                    });
                }
                Ok::<(), String>(())
            }
            .await;
            if result.is_err() {
                ports.refresh.store(true, Ordering::Release);
            }
        });
    }
}
impl ApplicationHost for Rc<Host> {
    fn main_view(&self, w: &mut Window, cx: &mut App) -> AnyView {
        self.register("main", w);
        let view = cx.new(|cx| root::Main::new(self.backend.clone(), w, cx));
        self.main.replace(Some(view.clone()));
        let host = self.clone();
        cx.defer(move |cx| host.companions(cx));
        cx.bind_keys([
            KeyBinding::new("cmd-k", root::CommandPalette, Some("MurmurWorkspace")),
            KeyBinding::new("cmd-1", root::HomePage, Some("MurmurWorkspace")),
            KeyBinding::new("cmd-2", root::MeetingsPage, Some("MurmurWorkspace")),
            KeyBinding::new("escape", root::Back, Some("MurmurWorkspace")),
        ]);
        view.into()
    }
    fn diagnostics_view(&self, w: &mut Window, cx: &mut App) -> AnyView {
        self.register("diagnostics", w);
        let mut backend = self.backend.clone();
        backend.consumer = Consumer::Diagnostics;
        backend.activity = Default::default();
        let view = cx.new(|cx| crate::diagnostics::Diagnostics::new(Rc::new(backend), w, cx));
        self.diagnostics.replace(Some(view.clone()));
        view.into()
    }
    fn window_hidden(&self, label: &'static str, w: &mut Window, cx: &mut App) {
        if label == "main" {
            if let Some(view) = self.main.borrow().clone() {
                view.update(cx, |s, cx| s.hide(w, cx));
            }
            self.backend.services.main_hidden();
        } else if let Some(view) = self.diagnostics.borrow().clone() {
            view.update(cx, |s, _| s.detach());
        }
    }
    fn tick(&self, cx: &mut App) {
        if let Some(handle) = self.handles.borrow().get("main").copied()
            && let Some(view) = self.main.borrow().clone()
        {
            let _ = handle.update(cx, |_, w, cx| {
                let active = w.is_window_active();
                if active
                    && !self.focused.replace(active)
                    && self.backend.booted.load(Ordering::Acquire)
                {
                    let b = self.backend.clone();
                    self.backend.spawn(async move {
                        let _ = b
                            .call(
                                "updater_action",
                                json!({"action":{"kind":"check","background":true}}),
                            )
                            .await;
                    });
                } else {
                    self.focused.set(active);
                }
                view.update(cx, |s, cx| s.tick(w, cx));
            });
        }
        let handles = self.handles.borrow().clone();
        while let Ok(command) = self.commands.try_recv() {
            if let Some(handle) = handles.get(Registry::command_label(&command)) {
                self.ports.registry.apply(command, *handle, cx);
            }
        }
        self.ports.registry.refresh_displays();
        for (label, handle) in &handles {
            if let Some((old, new)) = self.ports.registry.refresh(label) {
                let consumer = match *label {
                    "main" => Consumer::Main,
                    "diagnostics" => Consumer::Diagnostics,
                    "overlay" => Consumer::Overlay,
                    _ => Consumer::DictationPreview,
                };
                if old.position != new.position {
                    self.backend
                        .services
                        .window_event(consumer, service::WindowEvent::Moved(new.position));
                }
                if old.size != new.size {
                    self.backend
                        .services
                        .window_event(consumer, service::WindowEvent::Resized(new.size));
                }
                if old.scale != new.scale {
                    self.backend.services.window_event(
                        consumer,
                        service::WindowEvent::ScaleFactorChanged {
                            scale_factor: new.scale,
                        },
                    );
                }
                if old.visible != new.visible {
                    self.backend
                        .services
                        .window_event(consumer, service::WindowEvent::Focused(new.visible));
                    let _ = handle.update(cx, |_, w, cx| {
                        if *label == "main"
                            && let Some(view) = self.main.borrow().clone()
                        {
                            view.update(cx, |s, cx| {
                                if new.visible {
                                    s.show(w, cx)
                                } else {
                                    s.hide(w, cx)
                                }
                            });
                        }
                        if *label == "diagnostics"
                            && let Some(view) = self.diagnostics.borrow().clone()
                        {
                            view.update(cx, |s, cx| {
                                if new.visible {
                                    s.attach(cx);
                                    s.follow(cx)
                                } else {
                                    s.detach()
                                }
                            });
                        }
                    });
                }
            }
        }
        while let Ok(route) = self.routes.try_recv() {
            if let Route::Calibration { offset, finish } = &route {
                if let Some(overlay) = self.overlay.borrow().clone() {
                    overlay.update(cx, |view, cx| {
                        view.model
                            .event(crate::overlay::model::Event::Calibration(!finish), 0);
                        if let Some(offset) = offset {
                            view.model.snapshot.offset = *offset as f64;
                        }
                        cx.notify();
                    });
                }
                if *finish {
                    self.hydrate_companions(cx);
                }
                continue;
            }
            if matches!(
                route,
                Route::Updates
                    | Route::About
                    | Route::Teach(_)
                    | Route::Settings(_)
                    | Route::Meetings
                    | Route::MeetingToggle
                    | Route::MeetingSuggested(_)
                    | Route::Setup
                    | Route::SetupReplay
            ) && let Some(handle) = handles.get("main")
            {
                let _ = handle.update(cx, |_, w, _| native::show(w, true));
            }
            if matches!(route, Route::Diagnostics) {
                if let Some(handle) = handles.get("diagnostics") {
                    let _ = handle.update(cx, |_, w, _| native::show(w, true));
                }
                continue;
            }
            if matches!(route, Route::Sources(_)) {
                self.hydrate_companions(cx);
            }
            if let Some(handle) = handles.get("main")
                && let Some(view) = self.main.borrow().clone()
            {
                let _ = handle.update(cx, |_, w, cx| {
                    view.update(cx, |s, cx| s.route(route, w, cx))
                });
            }
        }
        if let Some(screen) = crate::overlay::native::screen() {
            let display = DockDisplay {
                left: screen.bounds.0,
                top: screen.bounds.1,
                right: screen.bounds.2,
                bottom: screen.bounds.3,
                work_bottom: screen.work.3,
            };
            let visible = self
                .ports
                .window("overlay")
                .is_some_and(|p| p.snapshot().visible);
            *self.ports.dock.lock().unwrap_or_else(|p| p.into_inner()) = Some((display, visible));
            if let Some(inbox) = self.ports.overlay.get() {
                let _ = inbox.publish(crate::overlay::model::Event::Dock(
                    self.backend.services.dock_limit(display),
                ));
            }
        }
        if self.ports.refresh.swap(false, Ordering::AcqRel) {
            self.hydrate_companions(cx);
        }
    }
    fn intent(&self, intent: Intent) {
        if !self.backend.booted.load(Ordering::Acquire)
            && !matches!(intent, Intent::Show | Intent::Quit | Intent::Diagnostics)
        {
            return;
        }
        let b = self.backend.clone();
        match intent {
            Intent::Settings => b.navigate(Route::Settings(crate::settings::schema::Page::General)),
            Intent::About => b.navigate(Route::About),
            Intent::CheckUpdates => {
                b.navigate(Route::Updates);
                self.backend.spawn(async move {
                    let _ = b
                        .call(
                            "updater_action",
                            json!({"action":{"kind":"check","background":false}}),
                        )
                        .await;
                });
            }
            Intent::PasteLast => b.services.retry_delivery(),
            Intent::ToggleDisabled => {
                let _ = b.services.toggle_disabled();
            }
            Intent::CycleMode => self.backend.spawn(async move {
                let _ = b.call("cycle_mode", json!({})).await;
            }),
            Intent::NextMode { revision, id } => self.backend.spawn(async move {
                let _ = b.services.next_mode(revision, id.as_deref());
            }),
            _ => {}
        }
    }
    fn tray(&self) -> TrayState {
        self.ports
            .tray
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    fn relaunch(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), &'static str>>>> {
        Box::pin(super::process::relaunch(
            self.backend.services.clone(),
            self.backend.runtime.clone(),
        ))
    }
}
struct PreviewHost(Services);
impl crate::preview::Host for PreviewHost {
    fn is_current(&self, generation: u64) -> bool {
        self.0.recording_is_current(generation)
    }
}
struct OverlayHost {
    backend: Backend,
    ports: Arc<Ports>,
}
impl crate::overlay::Host for OverlayHost {
    fn submit(&self, intent: crate::overlay::model::Intent) {
        use crate::overlay::model::{Control, Intent};
        let b = self.backend.clone();
        // Durable one-field preferences enter the source FIFO in this turn.
        if let Intent::Disabled(value) | Intent::AutoPaste(value) = &intent {
            let field = if matches!(intent, Intent::Disabled(_)) {
                "disabled"
            } else {
                "autoPaste"
            };
            let _ = b
                .services
                .accept_settings(json!({field:value}), b.services.settings_revision());
            return;
        }
        let ports = self.ports.clone();
        self.backend.spawn(async move {
            match intent {
                Intent::Control { generation, input } => {
                    let input = match input {
                        Control::Lock => murmur_core::dictation::Input::Lock,
                        Control::Stop => murmur_core::dictation::Input::HoldStop,
                        Control::Cancel => murmur_core::dictation::Input::Escape,
                    };
                    let _ = b.services.recording_control(generation, input);
                }
                Intent::RetryDelivery {
                    generation,
                    attempt,
                } => {
                    if b.services.recording_generation() == generation {
                        let result = b.call("retry_last_delivery", json!({})).await;
                        if b.services.recording_generation() == generation
                            && let Some(inbox) = ports.overlay.get()
                        {
                            use crate::overlay::model::{Delivery, Event};
                            let result = match result.as_ref().ok().and_then(|v| v["kind"].as_str())
                            {
                                Some("auto_pasted") => Delivery::Pasted,
                                Some("clipboard_only") => Delivery::Clipboard,
                                _ => Delivery::Retry,
                            };
                            let _ = inbox.publish(Event::RetryResult {
                                generation,
                                attempt,
                                result,
                            });
                        }
                    }
                }
                Intent::CycleMode => {
                    let _ = b.call("cycle_mode", json!({})).await;
                }
                Intent::OpenSettings => {
                    b.navigate(Route::Settings(crate::settings::schema::Page::General))
                }
                Intent::AcceptSuggestion(token) => b.navigate(Route::MeetingSuggested(token)),
                Intent::DismissSuggestion(token) => {
                    let _ = b
                        .call("dismiss_meeting_suggestion", json!({"token":token}))
                        .await;
                }
                Intent::Disabled(_) | Intent::AutoPaste(_) => {}
            }
        });
    }
    fn waveform_interest(&self, consumer: WaveformConsumer, active: bool) {
        self.backend.services.waveform_interest(consumer, active);
    }
    fn preview_anchor(&self, frame: OverlayFrame, above: bool, lift: f64) {
        if let Some(screen) = crate::overlay::native::screen()
            && let Some(inbox) = self.ports.preview.get()
        {
            let work = PopoverWorkArea {
                left: screen.work.0,
                top: screen.work.1,
                right: screen.work.2,
                bottom: screen.work.3,
            };
            let _ = inbox.publish(crate::preview::model::Event::Anchor(
                crate::preview::model::AnchorSnapshot {
                    revision: self.ports.revision.fetch_add(1, Ordering::SeqCst) + 1,
                    anchor: crate::preview::model::Anchor {
                        frame,
                        above,
                        lift,
                        work,
                    },
                },
            ));
        }
    }
}
pub fn run(identifier: String, home: PathBuf) -> std::process::ExitCode {
    #[cfg(feature = "native-host-qa")]
    if std::env::args_os().any(|a| a == "--native-host-legacy-fixture") {
        if identifier != "com.localdictation.gpui-host-qa110"
            || std::env::var_os("MURMUR_NATIVE_TEST_HOME")
                .map(PathBuf::from)
                .as_ref()
                != Some(&home)
            || home
                .file_name()
                .is_none_or(|name| !name.to_string_lossy().starts_with("murmur-gpui-test-"))
        {
            return std::process::ExitCode::FAILURE;
        }
        if std::fs::create_dir_all(home.join("Library/WebKit").join(&identifier)).is_err() {
            return std::process::ExitCode::FAILURE;
        }
    }

    let Ok(paths) = DataPaths::new(&home, &identifier) else {
        return std::process::ExitCode::FAILURE;
    };
    let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        return std::process::ExitCode::FAILURE;
    };
    if service::install_runtime(runtime.handle().clone()).is_err() {
        return std::process::ExitCode::FAILURE;
    }
    let _ = RUNTIME.set(runtime.handle().clone());
    let handle = runtime.handle().clone();
    crate::host::shell::run_with(move |cx| {
        let registry = Arc::new(Registry::default());
        registry.refresh_displays();
        let events = Arc::new(Events::default());
        let os = OsServices::default();
        let (terminal, receive) = mpsc::channel();
        let (routes, route_receive) = mpsc::sync_channel(64);
        let (commands, command_receive) = mpsc::sync_channel(512);
        let downloads = Arc::new(super::downloads::Downloads::default());
        let microphone = Arc::new(super::microphone::Preview::default());
        let levels = Arc::new(Mutex::new(None));
        let playback = Arc::new(super::meetings::Presentation::default());
        let scan = Arc::new(Mutex::new(None));
        let ports = Arc::new(Ports {
            registry,
            events: events.clone(),
            os: os.clone(),
            terminal,
            services: OnceLock::new(),
            tray: Default::default(),
            dock: Default::default(),
            lifecycle: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            overlay_clock: OnceLock::new(),
            refresh: AtomicBool::new(false),
            overlay: OnceLock::new(),
            preview: OnceLock::new(),
            downloads: downloads.clone(),
            microphone: microphone.clone(),
            playback: playback.clone(),
            scan: scan.clone(),
            levels: levels.clone(),
        });
        let bundle = os::bundle_path();
        let resources = if bundle.extension().is_some_and(|s| s == "app") {
            bundle.join("Contents/Resources")
        } else {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_default()
        };
        let context = CoreContext {
            paths: Arc::new(paths.clone()),
            permissions: Arc::new(Permission),
            background: Arc::new(Runtime(handle.clone())),
            main_thread: Arc::new(service::MainThreadQueue::default()),
        };
        let services = Services::start(
            context,
            service::NativePaths {
                home: home.clone(),
                app: paths.app.clone(),
                resources,
                cache: home.join("Library/Caches").join(&identifier),
            },
            identifier.clone(),
            ports.clone(),
        )
        .expect("native service composition");
        let _ = ports.services.set(services.clone());
        super::install_clipboard(services.clone(), cx);
        let login = super::login::Login::new(os.clone(), home, paths.app, identifier);
        let backend = Backend {
            services: services.clone(),
            activity: Default::default(),
            booting: Default::default(),
            booted: Default::default(),
            startup_stage: Default::default(),
            theme_snapshot: Default::default(),
            scan: scan.clone(),
            calendar_access: Arc::new(Mutex::new(crate::onboarding::model::Access::Unknown)),
            consumer: Consumer::Main,
            runtime: handle,
            events,
            os: os.clone(),
            routes,
            login,
            downloads,
            preview: microphone,
            levels,
            history_keys: Default::default(),
            knowledge_page: Default::default(),
            knowledge_reviews: Default::default(),
            meetings: playback,
            diagnostics: Default::default(),
        };
        let _ = NOTIFICATION_ROUTE.set(backend.routes.clone());
        let _ = os.notifications(notification);
        let host = Rc::new(Host {
            backend: backend.clone(),
            ports: ports.clone(),
            command_send: commands,
            commands: command_receive,
            routes: route_receive,
            handles: Default::default(),
            main: Default::default(),
            focused: Default::default(),
            diagnostics: Default::default(),
            overlay: Default::default(),
            preview: Default::default(),
            _subscriptions: Default::default(),
        });
        let b = backend.clone();
        let ports2 = ports.clone();
        let stop = backend.events.listen(
            Consumer::Main,
            Arc::new(move |event, payload| {
                if matches!(
                    event,
                    "updater-background-check-requested" | "check-for-updates-requested"
                ) {
                    let update = b.clone();
                    let background = event == "updater-background-check-requested";
                    b.spawn(async move {
                        let _ = update
                            .call(
                                "updater_action",
                                json!({"action":{"kind":"check","background":background}}),
                            )
                            .await;
                    });
                    if !background {
                        b.navigate(Route::Updates);
                    }
                }
                if event == "delivery-retry-feedback"
                    && let Ok(value) = serde_json::from_str::<Value>(payload)
                {
                    let message = match value["kind"].as_str() {
                        Some("auto_pasted") => "Pasted the latest dictation.",
                        Some("clipboard_only") => "Copied the latest dictation.",
                        Some("empty") => "There is no dictation to paste yet.",
                        Some("busy") => "Finish the current recording before retrying delivery.",
                        _ => "Delivery could not finish. Retry when the destination is available.",
                    };
                    b.navigate(Route::Notice(message));
                }
                if event == "audio-input-inventory-changed" && b.booted.load(Ordering::Acquire) {
                    let pending = b.clone();
                    b.spawn(async move {
                        let _ = migrate_microphone(&pending).await;
                    });
                }
                if event == "updater-state-changed"
                    && let Ok(snapshot) = serde_json::from_str(payload)
                {
                    b.navigate(Route::Update(snapshot));
                }
                if matches!(
                    event,
                    "overlay-settings-persisted"
                        | "dictation-settings-changed"
                        | "native-sources-changed"
                        | "native-services-changed"
                ) {
                    ports2.refresh.store(true, Ordering::Release);
                    let source = b.clone();
                    b.spawn(async move {
                        if let Ok(sources) = source.sources().await {
                            if source.booted.load(Ordering::Acquire)
                                && let Ok(settings) = super::projections::settings(&source).await
                            {
                                configure_background(
                                    &source,
                                    &settings,
                                    sources["murmur_onboarding_complete"] == true,
                                )
                                .await;
                            }
                            source.navigate(Route::Sources(sources));
                        }
                    });
                }
            }),
        );
        host._subscriptions.borrow_mut().push(stop);
        bootstrap(backend.clone());
        (
            Rc::new(host) as Rc<dyn ApplicationHost>,
            Arc::new(services) as Arc<dyn crate::host::lifecycle::ProcessLifecycle>,
            receive,
        )
    });
    // All native terminal paths have already passed the shared drain gate.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    std::process::ExitCode::SUCCESS
}

pub(super) fn bootstrap(b: Backend) {
    if b.booting
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let task = b.clone();
    b.spawn(async move {
        let b = task;
        let result = async {
            b.startup_stage.store(10, Ordering::Release);
            b.services.retry_initialization().await?;
            let synthetic = cfg!(feature = "native-host-qa")
                && std::env::args_os().any(|a| a == "--native-host-legacy-fixture")
                && os::identity() == "com.localdictation.gpui-host-qa110";
            b.startup_stage.store(20, Ordering::Release);
            super::legacy::migrate(&b, synthetic).await?;
            b.startup_stage.store(30, Ordering::Release);
            let sources = b.sources().await?;
            b.navigate(Route::Sources(sources.clone()));
            b.startup_stage.store(40, Ordering::Release);
            let settings = migrate_microphone(&b).await?;
            b.startup_stage.store(50, Ordering::Release);
            let models = super::projections::models(b.clone()).await?;
            let installed = models
                .model(&models.selected)
                .is_some_and(|m| m.install == crate::models::model::Install::Installed);
            let complete = sources["murmur_onboarding_complete"] == true && installed;
            b.startup_stage.store(60, Ordering::Release);
            b.call("init_dictation", json!({})).await?;
            b.startup_stage.store(70, Ordering::Release);
            b.call("configure_dictation", json!({"options":settings}))
                .await?;
            b.startup_stage.store(80, Ordering::Release);
            b.call("configure_dictation_control", json!({"settings":settings}))
                .await?;
            b.startup_stage.store(90, Ordering::Release);
            configure_background(&b, &settings, complete).await;
            if b.login
                .migrate(settings["launchAtLogin"].as_bool())
                .await
                .is_err()
            {
                b.navigate(Route::Notice(
                    "Launch at login could not be updated. Review it in Settings.",
                ));
            }
            Ok::<bool, String>(!complete)
        }
        .await;
        b.booting.store(false, Ordering::Release);
        match result {
            Ok(setup) => {
                b.startup_stage.store(110, Ordering::Release);
                b.booted.store(true, Ordering::Release);
                // Close the inventory-event race between the initial passive
                // read and publishing readiness. Later updates retry via listener.
                let _ = migrate_microphone(&b).await;
                b.navigate(Route::BootstrapReady(setup));
            }
            Err(_) => b.navigate(Route::BootstrapFailed),
        }
        #[cfg(feature = "native-host-qa")]
        contract(&b).await;
    });
}

async fn configure_background(b: &Backend, settings: &Value, complete: bool) {
    let complete = complete
        && b.call(
            "check_specific_model_exists",
            json!({"modelName":settings["model"]}),
        )
        .await
        .is_ok_and(|installed| installed == true);
    let _ = b.call("updater_action", json!({"action":{"kind":"configure","automatic":complete && settings["autoUpdateEnabled"] != false}})).await;
    let _ = b
        .call(
            "configure_meeting_suggestions",
            json!({"enabled":complete && settings["meetingSuggestionsEnabled"] == true}),
        )
        .await;
}

/// Explicit, secretless CLI verification of production projections. It never
/// requests permissions, captures input, reads a clipboard or starts a model.
#[cfg(feature = "native-host-qa")]
async fn contract(b: &Backend) {
    use crate::workspace::Projection;
    let args: Vec<_> = std::env::args_os().collect();
    let Some(index) = args.iter().position(|v| v == "--native-host-contract") else {
        return;
    };
    let Some(path) = args.get(index + 1).map(PathBuf::from) else {
        return;
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if os::identity() != "com.localdictation.gpui-host-qa110"
        || path.parent() != Some(home.as_path())
    {
        return;
    }
    let mut results = serde_json::Map::new();
    macro_rules! check {
        ($name:ident) => {
            results.insert(
                stringify!($name).into(),
                json!(
                    super::projections::$name(b.clone())
                        .await
                        .is_ok_and(|s| s.valid())
                ),
            );
        };
    }
    check!(home);
    results.insert(
        "preferences".into(),
        json!(
            super::projections::preferences(b.clone())
                .await
                .is_ok_and(|s| crate::settings::model::valid(&s))
        ),
    );
    check!(onboarding);
    check!(insights);
    check!(modes);
    check!(models);
    check!(knowledge);
    check!(customize);
    check!(themes);
    check!(palette);
    results.insert(
        "meetings".into(),
        json!(
            super::meetings::read(b.clone())
                .await
                .is_ok_and(|s| s.valid())
        ),
    );
    results.insert("identity".into(), json!(os::identity()));
    results.insert("nsApplication".into(), json!(true));
    results.insert(
        "startupStage".into(),
        json!(b.startup_stage.load(Ordering::Acquire)),
    );
    results.insert(
        "startupReady".into(),
        json!(b.booted.load(Ordering::Acquire)),
    );
    if args.iter().any(|arg| arg == "--native-host-legacy-fixture") {
        let migrated = async {
            let settings = super::projections::settings(b).await?;
            let (_, snapshot) = b.services.history_snapshot().await?;
            Ok::<bool, String>(
                settings["language"] == "es"
                    && snapshot.history.len() == 1
                    && b.call("webkit_state_snapshot", json!({})).await?["transcriptKeysRemoved"]
                        == true,
            )
        }
        .await
        .unwrap_or(false);
        results.insert("legacyFixture".into(), json!(migrated));
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
    {
        let _ = f
            .write_all(Value::Object(results).to_string().as_bytes())
            .and_then(|_| f.sync_all());
    }
    b.services.shutdown(murmur_core::shutdown::Request::Quit);
}

async fn migrate_microphone(b: &Backend) -> Result<Value, String> {
    let (revision, mut settings) = b.services.settings_snapshot().await?;
    if settings["microphoneIdMigrationComplete"] != true {
        let inventory = b.call("get_audio_input_inventory", json!({})).await?;
        if let Some(patch) = pending_microphone_patch(&settings, &inventory) {
            // The revision captured before the inventory await must still own
            // the setting when the source FIFO admits this field intent.
            b.patch(patch, revision).await?;
            settings = super::projections::settings(b).await?;
        }
    }
    Ok(settings)
}
fn pending_microphone_patch(settings: &Value, inventory: &Value) -> Option<Value> {
    if settings["microphoneIdMigrationComplete"] == true || inventory["status"] != "available" {
        return None;
    }
    let id = legacy_microphone(settings["microphone"].as_str()?, &inventory["devices"])?;
    Some(json!({"microphone":id,"microphoneIdMigrationComplete":true}))
}
fn legacy_microphone(current: &str, devices: &Value) -> Option<String> {
    let devices = devices.as_array()?;
    if devices.iter().any(|d| d["id"] == current) {
        return Some(current.into());
    }
    let matches: Vec<_> = devices.iter().filter(|d| d["name"] == current).collect();
    if matches.len() == 1 {
        matches[0]["id"].as_str().map(String::from)
    } else {
        None
    }
}
#[cfg(test)]
mod startup_tests {
    use super::{legacy_microphone, pending_microphone_patch};
    use serde_json::json;
    #[test]
    fn pending_migration_retries_available_inventory_without_overriding_manual_choice() {
        let pending = json!({"microphone":"Synthetic mic","microphoneIdMigrationComplete":false});
        let available =
            json!({"status":"available","devices":[{"id":"uid-a","name":"Synthetic mic"}]});
        assert_eq!(
            pending_microphone_patch(&pending, &json!({"status":"initializing"})),
            None
        );
        assert_eq!(
            pending_microphone_patch(&pending, &json!({"status":"available","devices":[]})),
            None
        );
        assert_eq!(
            pending_microphone_patch(&pending, &available),
            Some(json!({"microphone":"uid-a","microphoneIdMigrationComplete":true}))
        );
        assert_eq!(
            pending_microphone_patch(
                &json!({"microphone":"uid-b","microphoneIdMigrationComplete":true}),
                &available
            ),
            None
        );
    }
    #[test]
    fn microphone_migration_requires_an_existing_uid_or_unique_name() {
        let devices = json!([{"id":"uid-a","name":"Synthetic mic"},{"id":"uid-b","name":"Repeated"},{"id":"uid-c","name":"Repeated"}]);
        assert_eq!(legacy_microphone("uid-a", &devices), Some("uid-a".into()));
        assert_eq!(
            legacy_microphone("Synthetic mic", &devices),
            Some("uid-a".into())
        );
        assert_eq!(legacy_microphone("Repeated", &devices), None);
        assert_eq!(legacy_microphone("Missing", &devices), None);
    }
}
