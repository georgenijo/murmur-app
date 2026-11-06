//! Tauri transport for the core's named event protocol. No broadcast target is
//! used: webview labels and native consumer registrations are separate routes.
use crate::MutexExt;
use murmur_core::events::{Consumer, EventKind, EventSink, Router, WaveformConsumer};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

type Callback = Arc<dyn Fn(&str) + Send + Sync>;
#[derive(Default)]
struct Events {
    router: Mutex<Router>,
    native: Mutex<Vec<(Consumer, EventKind, Callback)>>,
}

pub(crate) fn initialize<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    app.manage(Events::default());
}
pub(crate) fn subscribe<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    consumer: Consumer,
    event: EventKind,
    callback: impl Fn(&str) + Send + Sync + 'static,
) {
    assert!(consumer.window_label().is_none() && event.consumers().contains(&consumer));
    app.state::<Events>()
        .native
        .lock_or_recover()
        .push((consumer, event, Arc::new(callback)));
}
pub(crate) fn clear_window(app: &tauri::AppHandle, label: &str) {
    if let Some(events) = app.try_state::<Events>() {
        events.router.lock_or_recover().clear_window(label);
    }
}

pub(crate) struct TauriEventSink<'a, R: tauri::Runtime>(&'a tauri::AppHandle<R>);

pub(crate) trait EventsExt<R: tauri::Runtime> {
    fn events(&self) -> TauriEventSink<'_, R>;
}
impl<R: tauri::Runtime> EventsExt<R> for tauri::AppHandle<R> {
    fn events(&self) -> TauriEventSink<'_, R> {
        TauriEventSink(self)
    }
}

impl<R: tauri::Runtime> EventSink for TauriEventSink<'_, R> {
    fn publish<P: Serialize + Clone>(&self, event: EventKind, payload: P) -> Result<(), String> {
        let app = self.0;
        let events = app.state::<Events>();
        let recipients = events
            .router
            .lock_or_recover()
            .recipients(event, |consumer, waveform| {
                consumer.window_label().is_none_or(|label| {
                    app.get_webview_window(label).is_some_and(|window| {
                        !waveform
                            || (window.is_visible().unwrap_or(false)
                                && !window.is_minimized().unwrap_or(true))
                    })
                })
            });
        // No registry/router lock is held across callbacks: a consumer may
        // publish another event. Native delivery works without any webview.
        let callbacks: Vec<_> = events
            .native
            .lock_or_recover()
            .iter()
            .filter(|(consumer, kind, _)| *kind == event && recipients.contains(consumer))
            .map(|(_, _, callback)| callback.clone())
            .collect();
        if !callbacks.is_empty() {
            let serialized = serde_json::to_string(&payload)
                .map_err(|_| "Event serialization failed".to_string())?;
            for callback in callbacks {
                callback(&serialized);
            }
        }
        let mut failure = None;
        for consumer in recipients {
            if let Some(label) = consumer.window_label() {
                // The window can disappear after the availability check. An
                // absent/recreated companion is a normal, silent no-op.
                if app.get_webview_window(label).is_some() {
                    if let Err(error) = app.emit_to(
                        tauri::EventTarget::webview_window(label),
                        event.name(),
                        payload.clone(),
                    ) {
                        failure = Some(error.to_string());
                    }
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[tauri::command]
pub(crate) fn set_waveform_subscription(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    consumer: WaveformConsumer,
    active: bool,
) -> Result<(), String> {
    if consumer.consumer().window_label() != Some(window.label()) {
        return Err("Waveform consumer does not belong to this window".into());
    }
    app.state::<Events>()
        .router
        .lock_or_recover()
        .set_waveform(consumer, active);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::Listener;

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
}
