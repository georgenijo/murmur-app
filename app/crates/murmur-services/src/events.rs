//! Native transport for the core's named event protocol. No broadcast target is
//! used: view labels and native consumer registrations are separate routes.
use crate::host::ServiceAccess;
use crate::MutexExt;
use murmur_core::events::{Consumer, EventKind, EventSink, Router, WaveformConsumer};
use serde::Serialize;
use std::sync::{Arc, Mutex};

type MainTask = Box<dyn FnOnce() + Send>;
#[cfg(test)]
type Dispatch = Arc<dyn Fn(MainTask) -> Result<(), String> + Send + Sync>;
#[cfg(test)]
type Visibility = Arc<dyn Fn(&str) -> bool + Send + Sync>;
type Callback = Arc<dyn Fn(&str) + Send + Sync>;
#[derive(Default)]
pub(crate) struct Events {
    router: Mutex<Router>,
    levels: Mutex<PendingLevel>,
    native: Mutex<Vec<(Consumer, EventKind, Callback)>>,
    #[cfg(test)]
    observers: Mutex<Vec<(EventKind, Callback)>>,
    #[cfg(test)]
    visibility: Mutex<Option<Visibility>>,
    #[cfg(test)]
    dispatch: Mutex<Option<Dispatch>>,
    #[cfg(test)]
    before_level_transport: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// At most one queued main-thread task and one latest sample. No window or
/// main-thread work is performed while this mutex is held.
#[derive(Default)]
struct PendingLevel {
    latest: Option<QueuedLevel>,
    scheduled: bool,
}
struct QueuedLevel {
    payload: String,
    generation: crate::audio::CaptureGeneration,
}

impl PendingLevel {
    fn replace(&mut self, payload: QueuedLevel) -> bool {
        self.latest = Some(payload);
        !std::mem::replace(&mut self.scheduled, true)
    }
    fn take(&mut self) -> Option<QueuedLevel> {
        self.scheduled = false;
        self.latest.take()
    }
}

pub(crate) fn initialize<R: crate::host::Runtime>(app: &crate::host::Context<R>) {
    app.install_event_owner(Events::default());
}
pub(crate) fn subscribe<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    consumer: Consumer,
    event: EventKind,
    callback: impl Fn(&str) + Send + Sync + 'static,
) {
    assert!(consumer.window_label().is_none() && event.consumers().contains(&consumer));
    app.event_owner()
        .native
        .lock_or_recover()
        .push((consumer, event, Arc::new(callback)));
}
pub(crate) fn clear_window<R: crate::host::Runtime>(app: &crate::host::Context<R>, label: &str) {
    if let Some(events) = app.try_event_owner() {
        events.router.lock_or_recover().clear_window(label);
    }
}

pub(crate) struct NativeEventSink<'a, R: crate::host::Runtime>(&'a crate::host::Context<R>);

pub(crate) trait EventsExt<R: crate::host::Runtime> {
    fn events(&self) -> NativeEventSink<'_, R>;
}
impl<R: crate::host::Runtime> EventsExt<R> for crate::host::Context<R> {
    fn events(&self) -> NativeEventSink<'_, R> {
        // Standalone synthetic mock hosts do not run the application's setup.
        if self.try_event_owner().is_none() {
            initialize(self);
        }
        NativeEventSink(self)
    }
}

impl<R: crate::host::Runtime> NativeEventSink<'_, R> {
    /// Existing explicitly targeted notifications retain their payloads/routes.
    /// Callers supply a named window consumer, never an arbitrary EventTarget.
    pub(crate) fn send_to<P: Serialize + Clone>(
        &self,
        consumer: Consumer,
        event: &str,
        payload: P,
    ) -> crate::host::Result<()> {
        let label = consumer.window_label().expect("window consumer required");
        crate::host::send(self.0, label, event, &payload)
    }

    pub(crate) fn publish_level(
        &self,
        generation: crate::audio::CaptureGeneration,
        level: f32,
    ) -> Result<(), String> {
        let payload = QueuedLevel {
            payload: serde_json::to_string(&level)
                .map_err(|_| "Event serialization failed".to_string())?,
            generation,
        };
        #[cfg(test)]
        if let Some(dispatch) = self.0.event_owner().dispatch.lock_or_recover().clone() {
            return self.queue_level_with(payload, |task| dispatch(task));
        }
        self.queue_level_with(payload, |task| {
            self.0.run_on_main_thread(task).map_err(|e| e.to_string())
        })
    }

    fn queue_level_with(
        &self,
        payload: QueuedLevel,
        schedule: impl FnOnce(MainTask) -> Result<(), String>,
    ) -> Result<(), String> {
        let app = self.0;
        if !payload.generation.is_current() {
            return Ok(());
        }
        if !app.event_owner().levels.lock_or_recover().replace(payload) {
            return Ok(());
        }
        let handle = app.clone();
        if let Err(error) = schedule(Box::new(move || {
            let payload = handle.event_owner().levels.lock_or_recover().take();
            if let Some(sample) = payload {
                // This callback runs on the main thread, with no mailbox/router
                // lock held. Visibility is fresh even after show/hide/restore.
                let _ = handle.events().deliver(
                    EventKind::AudioLevel,
                    &sample.payload,
                    Some(&sample.generation),
                );
            }
        })) {
            app.event_owner().levels.lock_or_recover().take();
            return Err(error.to_string());
        }
        Ok(())
    }

    fn waveform_visible(&self, window: &crate::host::Window<R>) -> bool {
        #[cfg(test)]
        if let Some(query) = self.0.event_owner().visibility.lock_or_recover().clone() {
            return query(window.label());
        }
        window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(true)
    }

    #[cfg(test)]
    fn before_level_transport(&self, waveform: bool) {
        if waveform {
            let hook = self
                .0
                .event_owner()
                .before_level_transport
                .lock_or_recover()
                .clone();
            if let Some(hook) = hook {
                hook();
            }
        }
    }

    fn deliver(
        &self,
        event: EventKind,
        payload: &str,
        generation: Option<&crate::audio::CaptureGeneration>,
    ) -> Result<(), String> {
        let app = self.0;
        let events = app.event_owner();
        let routing = events.router.lock_or_recover().clone();
        let recipients = routing.recipients(event, |consumer, waveform| {
            consumer.window_label().is_none_or(|label| {
                app.get_window(label)
                    .is_some_and(|window| !waveform || self.waveform_visible(&window))
            })
        });
        // Recheck on main after window queries, immediately before delivery.
        // Stop/cancel/end revoke the capture epoch; standby/new capture cannot
        // make a queued sample current again. The final permit below serializes
        // delivery with revocation, including the transport call itself.
        if generation.is_some_and(|generation| !generation.is_current()) {
            return Ok(());
        }
        // No registry/router lock is held across callbacks: a consumer may
        // publish another event. Native delivery works without any view.
        let callbacks: Vec<_> = events
            .native
            .lock_or_recover()
            .iter()
            .filter(|(consumer, kind, _)| *kind == event && recipients.contains(consumer))
            .map(|(_, _, callback)| callback.clone())
            .collect();
        #[cfg(test)]
        let callbacks = {
            let mut callbacks = callbacks;
            callbacks.extend(
                events
                    .observers
                    .lock_or_recover()
                    .iter()
                    .filter(|(kind, _)| *kind == event)
                    .map(|(_, callback)| callback.clone()),
            );
            callbacks
        };
        if !callbacks.is_empty() {
            for callback in callbacks {
                let _permit = match generation.map(|g| g.delivery_permit()) {
                    Some(None) => return Ok(()),
                    permit => permit,
                };
                #[cfg(test)]
                self.before_level_transport(generation.is_some());
                callback(payload);
            }
        }
        let mut failure = None;
        for consumer in recipients {
            if let Some(label) = consumer.window_label() {
                // The window can disappear after the availability check. An
                // absent/recreated companion is a normal, silent no-op.
                if app.get_window(label).is_some() {
                    let _permit = match generation.map(|g| g.delivery_permit()) {
                        Some(None) => return Ok(()),
                        permit => permit,
                    };
                    #[cfg(test)]
                    self.before_level_transport(generation.is_some());
                    if let Err(error) = crate::host::send_str(app, label, event.name(), payload) {
                        failure = Some(error.to_string());
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl<R: crate::host::Runtime> EventSink for NativeEventSink<'_, R> {
    fn publish<P: Serialize + Clone>(&self, event: EventKind, payload: P) -> Result<(), String> {
        let payload = serde_json::to_string(&payload)
            .map_err(|_| "Event serialization failed".to_string())?;
        if event == EventKind::AudioLevel {
            Err("Audio levels require capture generation ownership".into())
        } else {
            self.deliver(event, &payload, None)
        }
    }
}

pub(crate) fn set_waveform_subscription<R: crate::host::Runtime>(
    app: crate::host::Context<R>,
    window: crate::host::Window<R>,
    consumer: WaveformConsumer,
    active: bool,
) -> Result<(), String> {
    if consumer.consumer().window_label() != Some(window.label()) {
        return Err("Waveform consumer does not belong to this window".into());
    }
    app.event_owner()
        .router
        .lock_or_recover()
        .set_waveform(consumer, active);
    Ok(())
}

// The only concrete Tauri transport remains in this constrained sink. The host
// capability facade forwards here; it cannot expose Emitter or EventTarget.

#[cfg(test)]
pub(crate) fn observe_for_test<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    event: EventKind,
    callback: impl Fn(&str) + Send + Sync + 'static,
) {
    let _ = app.events();
    app.event_owner()
        .observers
        .lock_or_recover()
        .push((event, Arc::new(callback)));
}

#[cfg(test)]
pub(crate) mod native_tests {
    use super::*;
    use crate::host::{Monitor, NativePaths, NativePorts, Result as HostResult, WindowPort};
    use murmur_core::context::*;
    use std::path::PathBuf;
    struct TestWindow(String);
    impl WindowPort for TestWindow {
        fn label(&self) -> &str {
            &self.0
        }
        fn snapshot(&self) -> crate::host::WindowSnapshot {
            crate::host::WindowSnapshot {
                visible: true,
                minimized: false,
                fullscreen: false,
                maximized: false,
                size: dpi::PhysicalSize::new(100, 100),
                inner_position: dpi::PhysicalPosition::new(0, 0),
                outer_size: dpi::PhysicalSize::new(100, 100),
                position: dpi::PhysicalPosition::new(0, 0),
                scale: 1.,
                monitor: None,
            }
        }
        fn command(&self, _: crate::host::WindowCommand) -> HostResult<()> {
            panic!("no window effects")
        }
        fn ns_window(&self) -> HostResult<*mut std::ffi::c_void> {
            Err("no native window".into())
        }
    }
    struct NoIo {
        main: bool,
        deliveries: Arc<Mutex<Vec<(String, String, String)>>>,
    }
    impl Paths for NoIo {
        fn app_data_dir(&self) -> Result<PathBuf, ContextError> {
            Err(ContextError::PathUnavailable)
        }
        fn models_dir(&self) -> Option<PathBuf> {
            None
        }
    }
    impl Permissions for NoIo {
        fn microphone_status(&self) -> PermissionStatus {
            PermissionStatus::Denied
        }
    }
    impl BackgroundWork for NoIo {
        fn spawn(&self, _: BackgroundFuture) -> Result<(), ContextError> {
            Err(ContextError::SchedulingFailed)
        }
        fn spawn_blocking(&self, _: Task) -> Result<(), ContextError> {
            Err(ContextError::SchedulingFailed)
        }
    }
    impl MainThread for NoIo {
        fn dispatch(&self, _: Task) -> Result<(), ContextError> {
            Err(ContextError::SchedulingFailed)
        }
    }
    impl NativePorts for NoIo {
        fn window(&self, label: &str) -> Option<Arc<dyn WindowPort>> {
            (label != "main" || self.main)
                .then(|| Arc::new(TestWindow(label.into())) as Arc<dyn WindowPort>)
        }
        fn monitors(&self) -> Vec<Monitor> {
            vec![]
        }
        fn primary_monitor(&self) -> Option<Monitor> {
            None
        }
        fn send(&self, label: &str, event: &str, payload: &str) -> HostResult<()> {
            self.deliveries
                .lock_or_recover()
                .push((label.into(), event.into(), payload.into()));
            Ok(())
        }
        fn terminal(&self, _: murmur_core::shutdown::Action) {
            panic!("no terminal effect")
        }
        fn notify_update(&self, _: &str) {}
        fn dock_context(&self) -> Option<(crate::dock_watch::PillDisplay, bool)> {
            None
        }
        fn dock_changed(&self) {}
        fn mode_status(&self, _: serde_json::Value) {}
        fn open_path(&self, _: PathBuf) -> HostResult<()> {
            Err("no OS effect".into())
        }
        fn tray_update(&self, _: Option<String>) -> HostResult<()> {
            Ok(())
        }
    }
    pub(crate) struct Deferred {
        pub(crate) app: crate::host::Context,
        tasks: Arc<Mutex<Vec<MainTask>>>,
        deliveries: Arc<Mutex<Vec<(String, String, String)>>>,
        pub(crate) received: Arc<Mutex<Vec<String>>>,
    }
    impl Deferred {
        pub(crate) fn new() -> Self {
            Self::with_main(false)
        }
        pub(crate) fn with_main(main: bool) -> Self {
            let deliveries = Arc::new(Mutex::new(Vec::new()));
            let ports = Arc::new(NoIo {
                main,
                deliveries: deliveries.clone(),
            });
            let context = AppContext {
                paths: ports.clone(),
                permissions: ports.clone(),
                background: ports.clone(),
                main_thread: ports.clone(),
            };
            let app = crate::host::Context::new(
                context,
                NativePaths {
                    home: "/synthetic-home".into(),
                    app: "/synthetic-home/app".into(),
                    resources: "/synthetic-home/resources".into(),
                    cache: "/synthetic-home/cache".into(),
                },
                "synthetic.events".into(),
                ports,
            );
            initialize(&app);
            app.event_owner()
                .router
                .lock_or_recover()
                .set_waveform(WaveformConsumer::Overlay, true);
            let tasks = Arc::new(Mutex::new(Vec::new()));
            let queue = tasks.clone();
            *app.event_owner().dispatch.lock_or_recover() = Some(Arc::new(move |task| {
                queue.lock_or_recover().push(task);
                Ok(())
            }));
            let received = Arc::new(Mutex::new(Vec::new()));
            Self {
                app,
                tasks,
                deliveries,
                received,
            }
        }
        pub(crate) fn publish(&self, activity: &Arc<crate::audio::CaptureActivity>, level: f32) {
            crate::audio::publish_waveform(&self.app, activity.generation().unwrap(), level)
                .unwrap();
        }
        pub(crate) fn drain(&self) {
            for task in std::mem::take(&mut *self.tasks.lock_or_recover()) {
                task();
            }
            self.received.lock_or_recover().extend(
                std::mem::take(&mut *self.deliveries.lock_or_recover())
                    .into_iter()
                    .filter(|(label, event, _)| label == "overlay" && event == "audio-level")
                    .map(|(_, _, payload)| payload),
            );
        }
    }
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[test]
    fn standby_reactivation_cannot_revive_a_queued_waveform() {
        let levels = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        levels.publish(&activity, 0.25);
        activity.set_active(false);
        activity.set_active(true);
        levels.drain();
        assert!(levels.received.lock_or_recover().is_empty());
        levels.publish(&activity, 0.75);
        levels.drain();
        assert_eq!(*levels.received.lock_or_recover(), vec!["0.75"]);
    }
    #[test]
    fn capture_revoked_during_visibility_query_is_dropped_before_delivery() {
        let levels = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let revoked = activity.clone();
        *levels.app.event_owner().visibility.lock_or_recover() = Some(Arc::new(move |_| {
            revoked.set_active(false);
            true
        }));
        levels.publish(&activity, 0.25);
        levels.drain();
        assert!(levels.received.lock_or_recover().is_empty());
    }
    #[test]
    fn revocation_waits_between_final_check_and_transport_or_callback_entry() {
        use std::sync::mpsc;
        use std::time::Duration;
        for callback_path in [false, true] {
            let levels = Deferred::new();
            let activity = Arc::new(crate::audio::CaptureActivity::default());
            activity.set_active(true);
            let callback_count = Arc::new(AtomicUsize::new(0));
            if callback_path {
                let count = callback_count.clone();
                observe_for_test(&levels.app, EventKind::AudioLevel, move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                });
            }
            let (go, start) = mpsc::channel();
            let (attempting, attempted) = mpsc::channel();
            let (done, finished) = mpsc::channel();
            let revoked = activity.clone();
            let worker = std::thread::spawn(move || {
                start.recv().unwrap();
                attempting.send(()).unwrap();
                revoked.set_active(false);
                done.send(()).unwrap();
            });
            let first = AtomicBool::new(true);
            let blocked = Arc::new(AtomicBool::new(false));
            let held = blocked.clone();
            let attempted = Mutex::new(attempted);
            let finished = Mutex::new(finished);
            *levels
                .app
                .event_owner()
                .before_level_transport
                .lock_or_recover() = Some(Arc::new(move || {
                if first.swap(false, Ordering::SeqCst) {
                    // This hook is after the final check, before transport
                    // entry, while the production delivery permit is alive.
                    go.send(()).unwrap();
                    attempted.lock_or_recover().recv().unwrap();
                    held.store(
                        matches!(
                            finished
                                .lock_or_recover()
                                .recv_timeout(Duration::from_millis(50)),
                            Err(mpsc::RecvTimeoutError::Timeout)
                        ),
                        Ordering::SeqCst,
                    );
                }
            }));
            levels.publish(&activity, 0.25);
            levels.drain();
            worker.join().unwrap();
            assert!(
                blocked.load(Ordering::SeqCst),
                "revocation acknowledged before transport entry"
            );
            if callback_path {
                assert_eq!(callback_count.load(Ordering::SeqCst), 1);
            } else {
                assert_eq!(*levels.received.lock_or_recover(), vec!["0.25"]);
            }
            // A stale token cannot enter either path after revocation returns.
            assert!(activity.generation().is_none());
        }
    }
    #[test]
    fn production_subscriptions_and_producers_deliver_headlessly_with_synthetic_state() {
        use murmur_core::completion::{Command, CompletionService};
        use murmur_core::dictation::{Controller, Effect, Input, LaunchGate, Origin, Status};
        let fixture = Deferred::new();
        let app = &fixture.app;
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        crate::completion::initialize_with_service(app, service.clone());
        let overlay = app.get_window("overlay").unwrap();
        set_waveform_subscription(
            app.clone(),
            overlay.clone(),
            WaveformConsumer::Overlay,
            true,
        )
        .unwrap();
        // Initialize the production subscription registrations, substituting only
        // their OS/worker effects. No fabricated router, routes or manual emits.
        let receipts = Arc::new(Mutex::new(Vec::new()));
        let r = receipts.clone();
        crate::dictation_control::initialize_event_subscription(app, move |payload| {
            r.lock_or_recover()
                .push((Consumer::DictationControl, payload.to_owned()))
        });
        let r = receipts.clone();
        crate::dictation_cues::initialize_event_subscriptions(app, move |_, payload| {
            r.lock_or_recover()
                .push((Consumer::SoundCues, payload.to_owned()))
        });
        let r = receipts.clone();
        crate::smart_auto_probe::initialize_event_subscriptions(app, move || {
            r.lock_or_recover()
                .push((Consumer::SmartAuto, String::new()))
        });
        let r = receipts.clone();
        crate::meeting_suggestions::initialize_event_subscriptions(app, move || {
            r.lock_or_recover()
                .push((Consumer::MeetingSuggestions, String::new()))
        });
        let notifications = Arc::new(Mutex::new(Vec::new()));
        for event in ["history-changed", "statistics-changed"] {
            let r = notifications.clone();
            let kind = if event == "history-changed" {
                EventKind::HistoryChanged
            } else {
                EventKind::StatisticsChanged
            };
            observe_for_test(app, kind, move |payload| {
                assert_eq!(payload, "null");
                r.lock_or_recover().push(event);
            });
        }
        let mut controller = Controller::default();
        controller.gate = LaunchGate {
            shutdown: false,
            configured: true,
            onboarding_complete: true,
            model_installed: true,
            accessibility: true,
            disabled: false,
        };
        assert!(matches!(
            controller.input(Input::Toggle(None)),
            Some(Effect::Start { .. })
        ));
        controller.observe(1, Status::Recording, Origin::Toggle);
        crate::commands::recording::publish_status(app, "recording").unwrap();
        crate::audio::publish_waveform(app, activity.generation().unwrap(), 0.25).unwrap();
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.25"]);
        assert!(matches!(
            controller.input(Input::Toggle(None)),
            Some(Effect::Stop { generation: 1, .. })
        ));
        // Production completion adapter publishes notifications as part of the
        // producer operation, including duplicate attachment and retention paths.
        for (generation, retain) in [(1, true), (1, true), (2, false)] {
            crate::completion::produce(app, |producer| {
                producer.dictation(generation,
                serde_json::json!({"id":generation.to_string(), "text":"synthetic", "duration":1,"timestamp":0}), retain, None, "2026-11-06".into())
            });
        }
        crate::commands::recording::publish_status(app, "idle").unwrap();
        controller.observe(1, Status::Idle, Origin::Toggle);
        for consumer in [
            Consumer::DictationControl,
            Consumer::SoundCues,
            Consumer::SmartAuto,
            Consumer::MeetingSuggestions,
        ] {
            assert_eq!(
                receipts
                    .lock_or_recover()
                    .iter()
                    .filter(|(c, _)| *c == consumer)
                    .count(),
                2,
                "missing/duplicate production subscription {consumer:?}"
            );
        }
        assert_eq!(
            notifications
                .lock_or_recover()
                .iter()
                .filter(|e| **e == "history-changed")
                .count(),
            3
        );
        assert_eq!(
            notifications
                .lock_or_recover()
                .iter()
                .filter(|e| **e == "statistics-changed")
                .count(),
            3
        );
        let snapshot = service.execute(Command::Snapshot).unwrap();
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.statistics.total_recordings, 2);
        assert!(app.get_window("main").is_none());
    }

    #[test]
    fn adapter_delivers_only_to_named_windows_and_native_consumers_without_main() {
        let fixture = Deferred::new();
        let app = &fixture.app;
        app.events().publish(EventKind::AppEvent, ()).unwrap();
        let deliveries = fixture.deliveries.lock_or_recover();
        assert!(!deliveries.is_empty());
        assert!(deliveries
            .iter()
            .all(|(label, _, _)| label == "diagnostics"));
        drop(deliveries);
        let count = Arc::new(AtomicUsize::new(0));
        let received = count.clone();
        subscribe(
            app,
            Consumer::DictationControl,
            EventKind::RecordingStatusChanged,
            move |payload| {
                assert_eq!(payload, "\"recording\"");
                received.fetch_add(1, Ordering::SeqCst);
            },
        );
        app.events()
            .publish(EventKind::RecordingStatusChanged, "recording")
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(app.get_window("main").is_none());
    }

    #[test]
    fn capture_publication_never_queries_windows_and_coalesces_a_busy_main_thread() {
        let fixture = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        *fixture.app.event_owner().visibility.lock_or_recover() =
            Some(Arc::new(|_| panic!("capture queried a window")));
        for sample in 0..5000 {
            fixture.publish(&activity, sample as f32);
        }
        assert_eq!(fixture.tasks.lock_or_recover().len(), 1);
        assert!(fixture.deliveries.lock_or_recover().is_empty());
        *fixture.app.event_owner().visibility.lock_or_recover() = Some(Arc::new(|_| true));
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["4999.0"]);
        for visible in [false, true, false, true] {
            *fixture.app.event_owner().visibility.lock_or_recover() =
                Some(Arc::new(move |_| visible));
            fixture.publish(&activity, 0.25);
            fixture.drain();
        }
        assert_eq!(
            *fixture.received.lock_or_recover(),
            vec!["4999.0", "0.25", "0.25"]
        );
        assert!(!fixture.app.event_owner().levels.lock_or_recover().scheduled);
    }

    #[test]
    fn failed_native_dispatch_can_retry() {
        let fixture = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        assert!(fixture
            .app
            .events()
            .queue_level_with(
                QueuedLevel {
                    payload: "0.1".into(),
                    generation: activity.generation().unwrap(),
                },
                |_| Err("closed".into())
            )
            .is_err());
        assert!(!fixture.app.event_owner().levels.lock_or_recover().scheduled);
        fixture.publish(&activity, 0.2);
        assert_eq!(fixture.tasks.lock_or_recover().len(), 1);
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.2"]);
    }

    #[test]
    fn native_mailbox_coalesces_and_never_delivers_a_revoked_generation() {
        let fixture = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        fixture.publish(&activity, 0.25);
        fixture.publish(&activity, 0.75);
        assert_eq!(fixture.tasks.lock_or_recover().len(), 1);
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.75"]);
        fixture.publish(&activity, 0.5);
        activity.set_active(false);
        activity.set_active(true);
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.75"]);
        fixture.publish(&activity, 0.125);
        fixture.drain();
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.75", "0.125"]);
    }
    #[test]
    fn revocation_waits_for_the_final_native_callback_delivery_permit() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        use std::time::Duration;
        let fixture = Deferred::new();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let (go, begin) = mpsc::channel();
        let (attempting, attempted) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let revoked = activity.clone();
        let worker = std::thread::spawn(move || {
            begin.recv().unwrap();
            attempting.send(()).unwrap();
            revoked.set_active(false);
            done.send(()).unwrap();
        });
        let blocked = Arc::new(AtomicBool::new(false));
        let held = blocked.clone();
        let attempted = Mutex::new(attempted);
        let finished = Mutex::new(finished);
        *fixture
            .app
            .event_owner()
            .before_level_transport
            .lock_or_recover() = Some(Arc::new(move || {
            go.send(()).unwrap();
            attempted.lock_or_recover().recv().unwrap();
            held.store(
                matches!(
                    finished
                        .lock_or_recover()
                        .recv_timeout(Duration::from_millis(50)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ),
                Ordering::SeqCst,
            );
        }));
        fixture.publish(&activity, 0.25);
        fixture.drain();
        worker.join().unwrap();
        assert!(blocked.load(Ordering::SeqCst));
        assert_eq!(*fixture.received.lock_or_recover(), vec!["0.25"]);
        assert!(activity.generation().is_none());
    }
}
