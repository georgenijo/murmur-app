//! One Tokio execution boundary for all shared commands. Event subscriptions
//! are installed synchronously; slow reads coalesce in a bounded mailbox.
use super::os::OsServices;
use murmur_core::events::Consumer;
use murmur_services::native_services::Services;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
type Listener = Arc<dyn Fn(&str, &str) + Send + Sync>;
#[derive(Default)]
pub struct Events {
    pub feedback: Mutex<super::dictation_feedback::Feedback>,
    next: AtomicU64,
    revision: AtomicU64,
    listeners: Mutex<BTreeMap<u64, (&'static str, Listener)>>,
}
impl Events {
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn publish(&self, label: &str, event: &str, payload: &str) {
        if label == "main" {
            self.feedback
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .event(event, payload);
        }
        if !matches!(
            event,
            "audio-level"
                | "performance-resource-sample"
                | "microphone-preview-status"
                | "microphone-preview-level"
        ) {
            self.revision.fetch_add(1, Ordering::AcqRel);
        }
        let listeners: Vec<_> = self
            .listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|(l, _)| *l == label)
            .map(|(_, f)| f.clone())
            .collect();
        for listener in listeners {
            listener(event, payload);
        }
    }
    pub fn listen(self: &Arc<Self>, consumer: Consumer, listener: Listener) -> Stop {
        let Some(label) = consumer.window_label() else {
            return Stop::default();
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, (label, listener));
        Stop {
            events: Some((self.clone(), id)),
            active: Arc::new(AtomicBool::new(true)),
            cleanup: None,
        }
    }
}
#[derive(Default)]
pub struct Stop {
    events: Option<(Arc<Events>, u64)>,
    active: Arc<AtomicBool>,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}
impl Stop {
    pub fn with_cleanup(mut self, cleanup: impl FnOnce() + Send + 'static) -> Self {
        self.cleanup = Some(Box::new(cleanup));
        self
    }
    fn stop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
        self.active.store(false, Ordering::Release);
        if let Some((events, id)) = self.events.take() {
            events
                .listeners
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
        }
    }
}
impl Drop for Stop {
    fn drop(&mut self) {
        self.stop();
    }
}
macro_rules! subscription {($($path:path),*)=>{$(impl $path for Stop{fn stop(&mut self){Stop::stop(self)}})*};}
subscription!(
    crate::section::Subscription,
    crate::home::host::Subscription,
    crate::settings::host::Subscription,
    crate::history::host::Subscription,
    crate::onboarding::host::Subscription,
    crate::meetings::host::Subscription,
    crate::diagnostics::host::Subscription
);
#[derive(Default)]
pub struct ProgressStop(pub Arc<AtomicBool>);
impl Drop for ProgressStop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
/// A service may publish its own commit before its action reply. Keep those
/// coalesced reads behind the action, so they cannot revoke that same ticket.
#[derive(Default)]
pub struct Activity {
    pending: AtomicUsize,
    idle: tokio::sync::Notify,
}
pub struct Active(Arc<Activity>);
impl Drop for Active {
    fn drop(&mut self) {
        if self.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
        }
    }
}
impl Activity {
    pub fn enter(self: &Arc<Self>) -> Active {
        self.pending.fetch_add(1, Ordering::AcqRel);
        Active(self.clone())
    }
    pub fn busy(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }
    async fn wait(&self) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}
#[derive(Clone)]
pub struct Backend {
    pub services: Services,
    pub activity: Arc<Activity>,
    pub booting: Arc<AtomicBool>,
    pub booted: Arc<AtomicBool>,
    pub startup_stage: Arc<AtomicU64>,
    pub scan: Arc<Mutex<Option<(String, Value)>>>,
    pub theme_snapshot: Arc<Mutex<Option<(crate::themes::model::Snapshot, Value)>>>,
    pub calendar_access: Arc<Mutex<crate::onboarding::model::Access>>,
    /// Last System Audio access receipt said macOS lists Murmur as allowed but
    /// Core Audio still refuses the tap. Only a relaunch clears it, so it is
    /// never derived from the passive status string.
    pub system_audio_relaunch: Arc<AtomicBool>,
    pub consumer: Consumer,
    pub runtime: tokio::runtime::Handle,
    pub events: Arc<Events>,
    pub os: OsServices,
    pub routes: std::sync::mpsc::SyncSender<Route>,
    pub login: super::login::Login,
    pub downloads: Arc<super::downloads::Downloads>,
    pub preview: Arc<super::microphone::Preview>,
    pub history_keys: Arc<super::history::Keys>,
    pub knowledge_page: Arc<Mutex<(u64, crate::knowledge::model::Filter, usize)>>,
    pub knowledge_reviews: Arc<super::knowledge::Reviews>,
    pub meetings: Arc<super::meetings::Presentation>,
    pub diagnostics: Arc<super::diagnostics::Clocks>,
    pub levels: Arc<Mutex<Option<(u64, f32)>>>,
}
pub type ReadFuture<S> = Pin<Box<dyn Future<Output = Result<S, String>> + Send>>;
pub type Read<S> = fn(Backend) -> ReadFuture<S>;
impl Backend {
    /// Consumers call spawn, never await a shared command on GPUI's executor.
    pub fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) {
        self.runtime.spawn(work);
    }
    pub async fn call(&self, command: &str, args: Value) -> Result<Value, String> {
        self.services.request(self.consumer, command, args).await
    }
    pub async fn sources(&self) -> Result<Value, String> {
        self.call("webkit_state_snapshot", json!({}))
            .await
            .map(|r| r["values"].clone())
    }
    pub fn watch<S: Send + 'static>(
        &self,
        consumer: Consumer,
        read: Read<S>,
        publish: impl Fn(S) + Send + Sync + 'static,
    ) -> Stop {
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let stop = self.events.listen(
            consumer,
            Arc::new(move |event, _| {
                if matches!(
                    event,
                    "native-sources-changed"
                        | "native-services-changed"
                        | "dictation-settings-changed"
                        | "recording-status-changed"
                        | "dictation-generation-started"
                        | "history-changed"
                        | "statistics-changed"
                        | "model-runtime-status-changed"
                        | "download-progress"
                        | "mode-runtime-changed"
                        | "app-disabled-changed"
                        | "completion-storage-error"
                        | "audio-input-inventory-changed"
                        | "smart-auto-microphone-changed"
                        | "meeting-status-changed"
                        | "meeting-segment-finalized"
                        | "meeting-segment-failed"
                        | "meeting-speakers-updated"
                        | "system-audio-permission-changed"
                        | "updater-state-changed"
                        | "benchmark-progress"
                        | "microphone-startup-benchmark-progress"
                ) {
                    let _ = send.try_send(());
                }
            }),
        );
        let active = stop.active.clone();
        let backend = self.clone();
        self.spawn(async move {
            while receive.recv().await.is_some() {
                if !active.load(Ordering::Acquire) {
                    break;
                }
                loop {
                    backend.activity.wait().await;
                    if !active.load(Ordering::Acquire) {
                        break;
                    }
                    let snapshot = read(backend.clone()).await;
                    if backend.activity.pending.load(Ordering::Acquire) != 0 {
                        continue;
                    }
                    if let Ok(snapshot) = snapshot
                        && active.load(Ordering::Acquire)
                    {
                        publish(snapshot);
                    }
                    break;
                }
            }
        });
        stop
    }
}

#[derive(Clone)]
pub enum Route {
    Home,
    BootstrapRetry,
    BootstrapReady(bool),
    BootstrapFailed,
    Meetings,
    MeetingToggle,
    MeetingSuggested(String),
    Insights,
    Settings(crate::settings::schema::Page),
    Models,
    Knowledge,
    Modes,
    Themes,
    Customize,
    CustomizePage(crate::customize::model::Page),
    BindApp(String),
    Setup,
    SetupReplay,
    Palette,
    About,
    Notice(&'static str),
    Updates,
    Diagnostics,
    Teach(murmur_core::history::Entry),
    HistoryAction(String),
    Sources(Value),
    Update(murmur_core::updater::Snapshot),
    TeachingResult { epoch: u64, result: Value },
    Calibration { offset: Option<i64>, finish: bool },
}
impl Backend {
    pub fn navigate(&self, route: Route) {
        let _ = self.routes.try_send(route);
    }
    pub async fn patch(&self, patch: Value, revision: u64) -> Result<Value, String> {
        self.services
            .accept_settings(patch, revision)?
            .await
            .map_err(|_| "Settings owner unavailable")?
    }
    pub async fn source(&self, key: &str, expected: Value, value: Value) -> Result<(), String> {
        {
            self.services
                .accept_source(key.into(), expected, value)?
                .await
                .map_err(|_| "Source owner unavailable")??;
            self.events
                .publish("main", "native-sources-changed", "null");
            Ok(())
        }
    }
    pub async fn panel(
        &self,
        request: Value,
        live: super::os::Live,
    ) -> Result<Option<String>, String> {
        Ok(self.os.panel(request, live).await?.and_then(|v| {
            v.as_str()
                .or_else(|| v.as_array().and_then(|a| a.first()).and_then(Value::as_str))
                .map(String::from)
        }))
    }
}
impl Backend {
    pub async fn effect(
        &self,
        command: &str,
        args: Value,
        live: super::os::Live,
    ) -> Result<Value, String> {
        let result = self
            .services
            .request_guarded(self.consumer, command, args, live)
            .await?;
        record_system_audio_receipt(&self.system_audio_relaunch, command, &result);
        Ok(result)
    }
}
/// `request_system_audio_permission` is the only command whose receipt carries
/// `needsRelaunch`; each new request replaces the previous answer.
pub fn record_system_audio_receipt(relaunch: &AtomicBool, command: &str, receipt: &Value) {
    if command == "request_system_audio_permission" {
        relaunch.store(receipt["needsRelaunch"] == true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{Authority, Event, Inbox, Projection, Session};
    #[test]
    fn only_system_audio_access_receipts_set_and_clear_the_relaunch_fact() {
        let flag = AtomicBool::new(false);
        let refused = json!({"permission":"denied","captureReady":false,"audioFlowing":false,"needsRelaunch":true});
        record_system_audio_receipt(&flag, "get_system_audio_permission_status", &refused);
        assert!(!flag.load(Ordering::Acquire));
        record_system_audio_receipt(&flag, "request_system_audio_permission", &refused);
        assert!(flag.load(Ordering::Acquire));
        record_system_audio_receipt(
            &flag,
            "get_system_audio_permission_status",
            &json!("granted"),
        );
        assert!(flag.load(Ordering::Acquire));
        let granted = json!({"permission":"granted","captureReady":true,"audioFlowing":false,"needsRelaunch":false});
        record_system_audio_receipt(&flag, "request_system_audio_permission", &granted);
        assert!(!flag.load(Ordering::Acquire));
    }
    #[derive(Clone)]
    struct State(u64);
    impl Projection for State {
        fn revision(&self) -> u64 {
            self.0
        }
        fn valid(&self) -> bool {
            true
        }
        fn bytes(&self) -> usize {
            8
        }
    }
    #[test]
    fn commit_notification_follows_its_original_action_reply() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let activity = Arc::new(Activity::default());
            let active = activity.enter();
            let mut session = Session::default();
            session.attach();
            let ticket = session.begin().unwrap();
            let authority = Authority::default();
            authority.sync(&session);
            let lease = authority.lease(ticket);
            let inbox = Inbox::default();
            let changed = inbox.clone();
            let owner = activity.clone();
            let watcher = tokio::spawn(async move {
                owner.wait().await;
                assert!(
                    changed
                        .publish(Event::Changed {
                            attachment: ticket.attachment,
                            snapshot: State(2)
                        })
                        .is_ok()
                );
            });
            tokio::task::yield_now().await;
            assert!(inbox.drain().is_empty());
            assert!(lease.is_current());
            assert!(
                inbox
                    .publish(Event::Reply {
                        ticket,
                        result: Ok(State(2))
                    })
                    .is_ok()
            );
            drop(active);
            watcher.await.unwrap();
            for event in inbox.drain() {
                session.event(event);
            }
            assert!(!session.pending);
            assert!(session.problem.is_none());
            assert_eq!(session.revision, 2);
        });
    }
}
