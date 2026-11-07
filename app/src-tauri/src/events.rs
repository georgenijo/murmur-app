//! Tauri transport for the core's named event protocol. No broadcast target is
//! used: webview labels and native consumer registrations are separate routes.
use crate::host::ServiceAccess;
use crate::MutexExt;
use murmur_core::events::{Consumer, EventKind, EventSink, Router, WaveformConsumer};
use serde::Serialize;
use std::sync::{Arc, Mutex};
#[cfg(feature = "tauri-host")]
use tauri::Manager;

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

pub(crate) struct TauriEventSink<'a, R: crate::host::Runtime>(&'a crate::host::Context<R>);

pub(crate) trait EventsExt<R: crate::host::Runtime> {
    fn events(&self) -> TauriEventSink<'_, R>;
}
impl<R: crate::host::Runtime> EventsExt<R> for crate::host::Context<R> {
    fn events(&self) -> TauriEventSink<'_, R> {
        // Standalone synthetic mock hosts do not run the application's setup.
        if self.try_event_owner().is_none() {
            initialize(self);
        }
        TauriEventSink(self)
    }
}

impl<R: crate::host::Runtime> TauriEventSink<'_, R> {
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
                app.get_webview_window(label)
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
        // publish another event. Native delivery works without any webview.
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
                if app.get_webview_window(label).is_some() {
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

impl<R: crate::host::Runtime> EventSink for TauriEventSink<'_, R> {
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

/// Cleanup must happen even when the existing native reload operation fails.
#[cfg(feature = "tauri-host")]
pub(crate) fn web_content_terminated<R: crate::host::Runtime>(webview: &tauri::Webview<R>) {
    clear_before_reload(webview.app_handle(), webview.label(), || {
        // Same native recovery used by pinned tauri-runtime-wry's default
        // termination handler. Do not introduce a second JS/retry recovery path.
        webview.reload()
    });
}

#[cfg(feature = "tauri-host")]
fn clear_before_reload<R: crate::host::Runtime, E>(
    app: &crate::host::Context<R>,
    label: &str,
    reload: impl FnOnce() -> Result<(), E>,
) {
    clear_window(app, label);
    let _ = reload();
}

#[cfg_attr(feature = "tauri-host", tauri::command)]
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

/// Synthetic regression observers replace Tauri's former listen_any test hooks.
/// They inspect the transport seam without creating a production broadcast route.
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

// The only concrete Tauri transport remains in this constrained sink. The host
// capability facade forwards here; it cannot expose Emitter or EventTarget.
#[cfg(feature = "tauri-host")]
pub(crate) fn transport_send<R: crate::host::Runtime, P: serde::Serialize>(
    app: &crate::host::Context<R>,
    label: &str,
    event: &str,
    payload: &P,
) -> crate::host::Result<()> {
    use tauri::Emitter;
    app.emit_to(tauri::EventTarget::webview_window(label), event, payload)
}
#[cfg(feature = "tauri-host")]
pub(crate) fn transport_send_str<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    label: &str,
    event: &str,
    payload: &str,
) -> crate::host::Result<()> {
    use tauri::Emitter;
    app.emit_str_to(
        tauri::EventTarget::webview_window(label),
        event,
        payload.to_owned(),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tauri::Listener;

    pub(crate) struct DeferredLevels {
        pub(crate) app: tauri::App<tauri::test::MockRuntime>,
        tasks: Arc<Mutex<Vec<MainTask>>>,
        pub(crate) received: Arc<Mutex<Vec<String>>>,
    }

    impl DeferredLevels {
        pub(crate) fn new() -> Self {
            let app = mock_app();
            let overlay = tauri::WebviewWindowBuilder::new(&app, "overlay", Default::default())
                .build()
                .unwrap();
            set_waveform_subscription(
                app.handle().clone(),
                overlay.clone(),
                WaveformConsumer::Overlay,
                true,
            )
            .unwrap();
            let tasks = Arc::new(Mutex::new(Vec::<MainTask>::new()));
            let pending = tasks.clone();
            *app.event_owner().dispatch.lock_or_recover() = Some(Arc::new(move |task| {
                pending.lock_or_recover().push(task);
                Ok(())
            }));
            *app.event_owner().visibility.lock_or_recover() = Some(Arc::new(|_| true));
            let received = Arc::new(Mutex::new(Vec::new()));
            let receipts = received.clone();
            overlay.listen("audio-level", move |event| {
                receipts.lock_or_recover().push(event.payload().to_owned())
            });
            Self {
                app,
                tasks,
                received,
            }
        }

        pub(crate) fn publish(&self, activity: &Arc<crate::audio::CaptureActivity>, level: f32) {
            crate::audio::publish_waveform(
                self.app.handle(),
                activity.generation().unwrap(),
                level,
            )
            .unwrap();
        }

        pub(crate) fn drain(&self) {
            let tasks = std::mem::take(&mut *self.tasks.lock_or_recover());
            assert_eq!(tasks.len(), 1, "expected one deferred capture task");
            for task in tasks {
                task();
            }
        }
    }

    #[test]
    fn adapter_delivers_only_to_named_windows_and_native_consumers_without_main() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        initialize(app.handle());
        let diagnostics = tauri::WebviewWindowBuilder::new(&app, "diagnostics", Default::default())
            .build()
            .unwrap();
        let unrelated = tauri::WebviewWindowBuilder::new(&app, "unrelated", Default::default())
            .build()
            .unwrap();
        let received = Arc::new(AtomicUsize::new(0));
        let count = received.clone();
        diagnostics.listen("app-event", move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        });
        unrelated.listen("app-event", |_| panic!("event escaped its named route"));
        app.handle()
            .events()
            .publish(EventKind::AppEvent, ())
            .unwrap();
        assert_eq!(received.load(Ordering::SeqCst), 1);
        app.handle()
            .events()
            .publish(EventKind::HistoryChanged, ())
            .unwrap();
        let native = Arc::new(AtomicUsize::new(0));
        let count = native.clone();
        subscribe(
            app.handle(),
            Consumer::DictationControl,
            EventKind::RecordingStatusChanged,
            move |payload| {
                assert_eq!(payload, "\"recording\"");
                count.fetch_add(1, Ordering::SeqCst);
            },
        );
        app.handle()
            .events()
            .publish(EventKind::RecordingStatusChanged, "recording")
            .unwrap();
        assert_eq!(native.load(Ordering::SeqCst), 1);
        assert!(app.get_webview_window("main").is_none());
    }

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        initialize(app.handle());
        app
    }

    #[test]
    fn capture_publication_never_queries_windows_and_coalesces_a_busy_main_thread() {
        let app = mock_app();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let overlay = tauri::WebviewWindowBuilder::new(&app, "overlay", Default::default())
            .build()
            .unwrap();
        set_waveform_subscription(
            app.handle().clone(),
            overlay.clone(),
            WaveformConsumer::Overlay,
            true,
        )
        .unwrap();
        let tasks = Arc::new(Mutex::new(Vec::<MainTask>::new()));
        let pending = tasks.clone();
        *app.event_owner().dispatch.lock_or_recover() = Some(Arc::new(move |task| {
            pending.lock_or_recover().push(task);
            Ok(())
        }));
        *app.event_owner().visibility.lock_or_recover() =
            Some(Arc::new(|_| panic!("capture queried a window")));
        let received = Arc::new(Mutex::new(Vec::new()));
        let receipts = received.clone();
        overlay.listen("audio-level", move |event| {
            receipts.lock_or_recover().push(event.payload().to_owned())
        });
        for sample in 0..5000 {
            // The exact helper used by capture, through the real EventSink.
            crate::audio::publish_waveform(
                app.handle(),
                activity.generation().unwrap(),
                sample as f32,
            )
            .unwrap();
        }
        assert_eq!(tasks.lock_or_recover().len(), 1);
        assert!(received.lock_or_recover().is_empty());
        // The latest value is delivered after main is unblocked. Query only now.
        *app.event_owner().visibility.lock_or_recover() = Some(Arc::new(|_| true));
        let task = tasks.lock_or_recover().pop().unwrap();
        task();
        assert_eq!(*received.lock_or_recover(), vec!["4999.0"]);
        // Hidden/minimized consumers get no delivery. Show/restore is read fresh
        // on the next sample, without changing interest or restarting capture.
        for visible in [false, true, false, true] {
            *app.event_owner().visibility.lock_or_recover() = Some(Arc::new(move |_| visible));
            crate::audio::publish_waveform(app.handle(), activity.generation().unwrap(), 0.25)
                .unwrap();
            let task = tasks.lock_or_recover().pop().unwrap();
            task();
        }
        assert_eq!(*received.lock_or_recover(), vec!["4999.0", "0.25", "0.25"]);
        assert!(!app.event_owner().levels.lock_or_recover().scheduled);
    }

    #[test]
    fn standby_reactivation_cannot_revive_a_queued_waveform() {
        let levels = DeferredLevels::new();
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
        let levels = DeferredLevels::new();
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
            let levels = DeferredLevels::new();
            let activity = Arc::new(crate::audio::CaptureActivity::default());
            activity.set_active(true);
            let callback_count = Arc::new(AtomicUsize::new(0));
            if callback_path {
                let count = callback_count.clone();
                observe_for_test(levels.app.handle(), EventKind::AudioLevel, move |_| {
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
    fn failed_dispatch_can_retry_and_failed_reload_still_clears_only_crashed_interest() {
        let app = mock_app();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let main = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let overlay = tauri::WebviewWindowBuilder::new(&app, "overlay", Default::default())
            .build()
            .unwrap();
        for (window, consumer) in [
            (main.clone(), WaveformConsumer::MainRecording),
            (main.clone(), WaveformConsumer::Corpus),
            (overlay, WaveformConsumer::Overlay),
        ] {
            set_waveform_subscription(app.handle().clone(), window, consumer, true).unwrap();
        }
        assert!(app
            .handle()
            .events()
            .queue_level_with(
                QueuedLevel {
                    payload: "0.1".into(),
                    generation: activity.generation().unwrap()
                },
                |_| Err("closed".into())
            )
            .is_err());
        assert!(!app.event_owner().levels.lock_or_recover().scheduled);
        let callbacks = AtomicUsize::new(0);
        app.handle()
            .events()
            .queue_level_with(
                QueuedLevel {
                    payload: "0.2".into(),
                    generation: activity.generation().unwrap(),
                },
                |_| {
                    callbacks.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(callbacks.load(Ordering::SeqCst), 1);
        clear_before_reload(app.handle(), "main", || {
            assert_eq!(
                app.event_owner()
                    .router
                    .lock_or_recover()
                    .recipients(EventKind::AudioLevel, |_, _| true),
                vec![Consumer::Overlay]
            );
            Err("synthetic reload failure")
        });
        assert_eq!(
            app.event_owner()
                .router
                .lock_or_recover()
                .recipients(EventKind::AudioLevel, |_, _| true),
            vec![Consumer::Overlay]
        );
        // Exercise the registered hook itself against a mock webview, too.
        set_waveform_subscription(
            app.handle().clone(),
            main.clone(),
            WaveformConsumer::Corpus,
            true,
        )
        .unwrap();
        web_content_terminated(main.as_ref());
        assert_eq!(
            app.event_owner()
                .router
                .lock_or_recover()
                .recipients(EventKind::AudioLevel, |_, _| true),
            vec![Consumer::Overlay]
        );
    }

    #[test]
    fn production_subscriptions_and_producers_deliver_headlessly_with_synthetic_state() {
        use murmur_core::completion::{Command, CompletionService};
        use murmur_core::dictation::{Controller, Effect, Input, LaunchGate, Origin, Status};
        let app = mock_app();
        let activity = Arc::new(crate::audio::CaptureActivity::default());
        activity.set_active(true);
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        crate::completion::initialize_with_service(app.handle(), service.clone());
        let overlay = tauri::WebviewWindowBuilder::new(&app, "overlay", Default::default())
            .build()
            .unwrap();
        let diagnostics = tauri::WebviewWindowBuilder::new(&app, "diagnostics", Default::default())
            .build()
            .unwrap();
        set_waveform_subscription(
            app.handle().clone(),
            overlay.clone(),
            WaveformConsumer::Overlay,
            true,
        )
        .unwrap();
        // Initialize the production subscription registrations, substituting only
        // their OS/worker effects. No fabricated router, routes or manual emits.
        let receipts = Arc::new(Mutex::new(Vec::new()));
        let r = receipts.clone();
        crate::dictation_control::initialize_event_subscription(app.handle(), move |payload| {
            r.lock_or_recover()
                .push((Consumer::DictationControl, payload.to_owned()))
        });
        let r = receipts.clone();
        crate::dictation_cues::initialize_event_subscriptions(app.handle(), move |_, payload| {
            r.lock_or_recover()
                .push((Consumer::SoundCues, payload.to_owned()))
        });
        let r = receipts.clone();
        crate::smart_auto_probe::initialize_event_subscriptions(app.handle(), move || {
            r.lock_or_recover()
                .push((Consumer::SmartAuto, String::new()))
        });
        let r = receipts.clone();
        crate::meeting_suggestions::initialize_event_subscriptions(app.handle(), move || {
            r.lock_or_recover()
                .push((Consumer::MeetingSuggestions, String::new()))
        });
        let notifications = Arc::new(Mutex::new(Vec::new()));
        for event in ["history-changed", "statistics-changed"] {
            let r = notifications.clone();
            diagnostics.listen(event, move |payload| {
                assert_eq!(payload.payload(), "null");
                r.lock_or_recover().push(event);
            });
        }
        let levels = Arc::new(Mutex::new(Vec::new()));
        let r = levels.clone();
        overlay.listen("audio-level", move |payload| {
            r.lock_or_recover().push(payload.payload().to_owned())
        });
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
        crate::commands::recording::publish_status(app.handle(), "recording").unwrap();
        crate::audio::publish_waveform(app.handle(), activity.generation().unwrap(), 0.25).unwrap();
        assert_eq!(*levels.lock_or_recover(), vec!["0.25"]);
        assert!(matches!(
            controller.input(Input::Toggle(None)),
            Some(Effect::Stop { generation: 1, .. })
        ));
        // Production completion adapter publishes notifications as part of the
        // producer operation, including duplicate attachment and retention paths.
        for (generation, retain) in [(1, true), (1, true), (2, false)] {
            crate::completion::produce(app.handle(), |producer| {
                producer.dictation(generation,
                serde_json::json!({"id":generation.to_string(), "text":"synthetic", "duration":1,"timestamp":0}), retain, None, "2026-11-06".into())
            });
        }
        crate::commands::recording::publish_status(app.handle(), "idle").unwrap();
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
        assert!(app.get_webview_window("main").is_none());
    }
}
