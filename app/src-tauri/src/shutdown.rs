//! Process-wide admission latch; the host adapter owns shutdown effects.
use std::sync::atomic::{AtomicBool, Ordering};
static CLOSING: AtomicBool = AtomicBool::new(false);
pub(crate) fn closing() -> bool {
    CLOSING.load(Ordering::Acquire)
}
pub(crate) fn close_admission() {
    CLOSING.store(true, Ordering::Release);
}
pub(crate) fn require_running() -> Result<(), String> {
    if closing() {
        Err("Murmur is shutting down".into())
    } else {
        Ok(())
    }
}

use crate::{MutexExt, State};
use murmur_core::{
    context::AppContext,
    shutdown::{Action, Coordinator, Effect, Event, Outcome, Request, Step, WriteOutcome},
};
use std::{
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::Manager;
pub(crate) mod adapter;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(feature = "shutdown-test-support")]
pub(crate) mod qa_evidence;
use adapter::Message;
pub(crate) struct Shutdown {
    clock: Instant,
    core: Arc<Mutex<Coordinator>>,
    send: mpsc::Sender<Message>,
}
pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    #[cfg(feature = "shutdown-test-support")]
    test_support::initialize_evidence(app);
    let clock = Instant::now();
    let core = Arc::new(Mutex::new(Coordinator::default()));
    let (send, receive) = mpsc::channel();
    app.manage(Shutdown {
        clock,
        core: core.clone(),
        send: send.clone(),
    });
    #[cfg(target_os = "macos")]
    macos::install(app.clone())?;
    let handle = app.clone();
    let context = app.state::<AppContext>().inner().clone();
    std::thread::Builder::new()
        .name("shutdown-deadlines".into())
        .spawn(move || loop {
            let now = || clock.elapsed().as_millis() as u64;
            let message = receive.recv_timeout(Duration::from_millis(10));
            let Some(effects) = reduce(&core, message, now()) else {
                break;
            };
            for effect in effects {
                if effect.step == Step::Terminate {
                    #[cfg(feature = "shutdown-test-support")]
                    test_support::hold(&handle, &context, &receive, &core);
                    let action = core.lock_or_recover().issue();
                    if let Some(action) = action {
                        let app = handle.clone();
                        let _ = context
                            .main_thread
                            .dispatch(Box::new(move || terminal(&app, action)));
                    }
                    return;
                }
                let app = handle.clone();
                let send = send.clone();
                let deadline = clock + Duration::from_millis(effect.deadline_ms);
                let _ = context.background.spawn(Box::pin(async move {
                    #[cfg(feature = "shutdown-test-support")]
                    test_support::delay(&app, effect.step).await;
                    let outcome = run_owner(&app, effect, deadline).await;
                    let _ = send.send(Message::Finished(Event { effect, outcome }));
                }));
            }
        })
        .map_err(|_| "Shutdown deadline driver unavailable")?;
    signals(app.clone())?;
    Ok(())
}
fn reduce(
    core: &Mutex<Coordinator>,
    message: Result<Message, mpsc::RecvTimeoutError>,
    now: u64,
) -> Option<Vec<Effect>> {
    #[cfg(feature = "shutdown-test-support")]
    if test_support::evidence_enabled() {
        return adapter::reduce_observed(core, message, now, |before, core, event| {
            test_support::transition(before, core, event, None, now);
        });
    }
    adapter::reduce(core, message, now)
}
pub(crate) fn request(app: &tauri::AppHandle, request: Request) {
    close_admission(); // synchronous even if the reducer/owner thread is delayed
    if let Some(shutdown) = app.try_state::<Shutdown>() {
        // Acceptance and issue share the reducer mutex. A queued notification
        // cannot hide an accepted Quit/System request from terminal commitment.
        let now = shutdown.clock.elapsed().as_millis() as u64;
        #[cfg(feature = "shutdown-test-support")]
        if test_support::evidence_enabled() {
            adapter::accept_observed(
                &shutdown.core,
                &shutdown.send,
                request,
                now,
                |before, core| {
                    test_support::transition(before, core, None, Some(request), now);
                },
            );
            return;
        }
        adapter::accept(&shutdown.core, &shutdown.send, request, now);
    }
}
pub(crate) fn forbidden_restart_exit(code: Option<i32>, permitted: bool) -> bool {
    !permitted && code == Some(tauri::RESTART_EXIT_CODE)
}
pub(crate) fn permitted(app: &tauri::AppHandle) -> bool {
    app.try_state::<Shutdown>()
        .is_some_and(|s| s.core.lock_or_recover().termination_permitted())
}
pub(crate) fn receipt(app: &tauri::AppHandle) {
    let logs = crate::telemetry::shutdown_receipt();
    if let Some(shutdown) = app.try_state::<Shutdown>() {
        let mut core = shutdown.core.lock_or_recover();
        #[cfg(feature = "shutdown-test-support")]
        if core.termination_permitted() {
            // A fresh native termination request can observe the committed
            // permit before the queued terminal callback runs. Cover that
            // runtime terminal path too; the sink publishes at most once.
            test_support::write_terminal_evidence(app, core.action());
        }
        tracing::info!(target:"shutdown", permitted=core.termination_permitted(), receipt=?core.receipt, telemetry=?logs, "terminal shutdown receipt");
        core.exited();
    }
}
async fn run_owner(app: &tauri::AppHandle, effect: Effect, deadline: Instant) -> Outcome {
    match effect.step {
        Step::Admission => {
            let state = app.state::<State>();
            let _fence = state.app_state.recording_transition.lock().await;
            let worker_app = app.clone();
            let fenced = tauri::async_runtime::spawn_blocking(move || {
                crate::managed_child::shutdown_fence();
                crate::audio::prepare::invalidate(
                    crate::audio::prepare::InvalidateReason::Superseded,
                );
                crate::smart_auto_probe::preempt();
                crate::smart_destination::set_enabled(false);
                worker_app.state::<State>().meeting_suggestions.shutdown();
                true
            })
            .await
            .unwrap_or(false);
            Outcome::Admission(fenced)
        }
        Step::Capture => {
            let state = app.state::<State>();
            let generation = {
                let _fence = state.app_state.recording_transition.lock().await;
                state.app_state.recording_id.load(Ordering::SeqCst)
            };
            let stop_app = app.clone();
            let dictation = async move {
                let state = stop_app.state::<State>();
                let _ = crate::commands::recording::stop_native_recording_for(
                    stop_app.clone(),
                    state,
                    Some(generation),
                    crate::audio_lifecycle::DictationStopTrigger::Other,
                )
                .await;
                // An existing Processing operation owns its completion; join its
                // generation instead of submitting another stop/pipeline.
                loop {
                    let state = stop_app.state::<State>();
                    if state.app_state.recording_id.load(Ordering::SeqCst) != generation {
                        return false;
                    }
                    if state.app_state.dictation.lock_or_recover().status
                        == crate::state::DictationStatus::Idle
                    {
                        return true;
                    }
                    if Instant::now() >= deadline {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            };
            let meeting_app = app.clone();
            let meetings = async move {
                tauri::async_runtime::spawn_blocking(move || {
                    meeting_app
                        .state::<State>()
                        .meetings
                        .shutdown(&meeting_app, deadline)
                })
                .await
                .unwrap_or(false)
            };
            let (dictation, meetings) = tokio::join!(dictation, meetings);
            Outcome::Capture(dictation && meetings)
        }
        Step::Helpers => {
            let app = app.clone();
            let confirmed = tauri::async_runtime::spawn_blocking(move || {
                crate::audio_lifecycle::retire_standby();
                crate::audio_lifecycle::cancel_preview_for_environment_change(
                    crate::audio_lifecycle::AudioCancelReason::User,
                );
                let _ = crate::meeting_diarization::cancel_all();
                let _ = crate::coreml_installer::clear_quarantined_installer();
                crate::audio_inventory::shutdown();
                crate::managed_child::shutdown_fence();
                while Instant::now() < deadline && !crate::managed_child::helpers_confirmed_empty()
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = app; // retain all managed owners through the wait
                crate::managed_child::helpers_confirmed_empty()
            })
            .await
            .unwrap_or(false);
            Outcome::Helpers(confirmed)
        }
        Step::Writers => {
            // Keep accepting final owner events until writers finish or their
            // absolute deadline. A stuck owner cannot postpone the log drain.
            let (writers_done, drained) = mpsc::channel();
            let telemetry = tauri::async_runtime::spawn_blocking(move || {
                crate::telemetry::shutdown_flush(drained, deadline)
            });
            let app = app.clone();
            let committed = tauri::async_runtime::spawn_blocking(move || {
                wait_accepted_producers();
                let sources = crate::commands::settings_store::flush(&app);
                let windows = crate::window_state::flush(&app);
                let memo = crate::audio::flush_capture_memo();
                let completion = crate::completion::shutdown_flush(&app);
                if sources && windows && memo {
                    completion
                } else {
                    WriteOutcome::UnpreservedFailure
                }
            })
            .await
            .unwrap_or(WriteOutcome::UnpreservedFailure);
            let _ = writers_done.send(());
            let telemetry_flushed = telemetry.await.unwrap_or(false);
            Outcome::Writers(writer_outcome(committed, telemetry_flushed))
        }
        Step::Terminate => unreachable!(),
    }
}
pub(crate) fn writer_outcome(content: WriteOutcome, telemetry_flushed: bool) -> WriteOutcome {
    if telemetry_flushed {
        content
    } else {
        WriteOutcome::UnpreservedFailure
    }
}

#[allow(clippy::disallowed_methods)]
fn terminal(app: &tauri::AppHandle, action: Action) {
    // This is the sole application restart API call; the reducer committed its
    // drained permit before queuing this callback.
    if !permitted(app) {
        return;
    }
    #[cfg(feature = "shutdown-test-support")]
    test_support::write_terminal_evidence(app, action);
    #[cfg(target_os = "macos")]
    if macos::reply_if_pending() {
        return;
    }
    match action {
        Action::Restart => app.request_restart(),
        Action::Quit => app.exit(0),
    }
}

#[cfg(unix)]
mod signal;

/// The raw handler does nothing except a nonblocking async-signal-safe write.
/// Locks, allocation and host calls live on the reader thread.
#[cfg(unix)]
fn signals(app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::AtomicI32;
    static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
    extern "C" fn notify(signal: libc::c_int) {
        self::signal::notify(SIGNAL_FD.load(Ordering::Relaxed), signal);
    }
    let mut fds = [-1; 2];
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err("Shutdown self-pipe unavailable".into());
        }
        if libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK) == -1
            || libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC) == -1
            || libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC) == -1
        {
            for fd in fds {
                libc::close(fd);
            }
            return Err("Shutdown self-pipe configuration failed".into());
        }
        SIGNAL_FD.store(fds[1], Ordering::Relaxed);
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = notify as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = libc::SA_RESTART;
        for signal in [libc::SIGTERM, libc::SIGINT] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err("Shutdown signal registration failed".into());
            }
        }
    }
    std::thread::Builder::new()
        .name("shutdown-signals".into())
        .spawn(move || {
            let mut byte = 0u8;
            loop {
                let count = unsafe { libc::read(fds[0], (&mut byte as *mut u8).cast(), 1) };
                if count > 0 {
                    request(&app, Request::System);
                } else if count == 0 {
                    break;
                }
            }
        })
        .map_err(|_| "Shutdown signal reader unavailable")?;
    Ok(())
}
#[cfg(not(unix))]
fn signals(_: tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

static WRITERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Registered before native source work is queued. Its lifetime crosses all
/// asynchronous publication continuations, including accepted jobs at shutdown.
pub(crate) struct WriterPermit;
impl WriterPermit {
    /// Only native owners of previously admitted work may use this seam.
    /// Closing external admission must not discard their final publications.
    pub(crate) fn accepted_producer() -> Self {
        WRITERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
    pub(crate) fn acquire() -> Result<Self, String> {
        WRITERS.fetch_add(1, Ordering::AcqRel);
        if closing() {
            WRITERS.fetch_sub(1, Ordering::AcqRel);
            return Err("Murmur is shutting down".into());
        }
        Ok(Self)
    }
}
impl Drop for WriterPermit {
    fn drop(&mut self) {
        WRITERS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn wait_accepted_producers() {
    while WRITERS.load(Ordering::Acquire) != 0 {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Accepted source intents enter one native FIFO before any disk wait. React's
/// retry chain is never the sole owner of an edit awaiting an older publication.
pub(crate) async fn source_call<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ()> {
    type Job = Box<dyn FnOnce() + Send>;
    static SOURCE: std::sync::OnceLock<mpsc::Sender<Job>> = std::sync::OnceLock::new();
    let source = SOURCE.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("native-source-intents".into())
            .spawn(move || {
                while let Ok(job) = receive.recv() {
                    job();
                }
            })
            .expect("native source owner");
        send
    });
    let (send, receive) = tokio::sync::oneshot::channel();
    source
        .send(Box::new(move || {
            let _ = send.send(work());
        }))
        .map_err(|_| ())?;
    receive.await.map_err(|_| ())
}

fn source_order() -> &'static Mutex<murmur_core::source_order::SourceOrder> {
    static ORDER: std::sync::OnceLock<Mutex<murmur_core::source_order::SourceOrder>> =
        std::sync::OnceLock::new();
    ORDER.get_or_init(Default::default)
}

fn valid_source_session(session: &str) -> bool {
    !session.is_empty() && session.len() <= 128 && session.is_ascii()
}

#[tauri::command]
pub(crate) async fn register_source_session(
    window: tauri::WebviewWindow,
    source_session: String,
) -> Result<(), String> {
    let admission = WriterPermit::acquire()?;
    register_source_owner(window.label().into(), source_session, admission).await
}

async fn register_source_owner(
    owner: String,
    session: String,
    admission: WriterPermit,
) -> Result<(), String> {
    if !matches!(owner.as_str(), "main" | "overlay") || !valid_source_session(&session) {
        return Err("Invalid source session".into());
    }
    source_call(move || {
        let _admission = admission;
        if source_order().lock_or_recover().register(&owner, &session) {
            Ok(())
        } else {
            Err("Source session retired".into())
        }
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

/// Session retirement and publication share the FIFO and mutex. No request can
/// re-activate an old renderer. Concurrent labels do not retire one another.
pub(crate) async fn ordered_source_call<T: Send + 'static>(
    owner: String,
    session: String,
    work: impl FnOnce(&mut murmur_core::source_order::SourceOrder, &str, &str) -> Result<T, String>
        + Send
        + 'static,
) -> Result<murmur_core::source_order::SourceWrite<T>, String> {
    if !matches!(owner.as_str(), "main" | "overlay") || !valid_source_session(&session) {
        return Err("Invalid source write owner or session".into());
    }
    source_call(move || {
        let mut order = source_order().lock_or_recover();
        if !order.is_current(&owner, &session) {
            return Ok(murmur_core::source_order::SourceWrite::Superseded);
        }
        work(&mut order, &owner, &session)
            .map(|value| murmur_core::source_order::SourceWrite::Applied { value })
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

pub(crate) async fn versioned_source_call<T: Send + 'static>(
    owner: String,
    session: String,
    key: String,
    generation: u64,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<murmur_core::source_order::SourceWrite<T>, String> {
    if owner != "main" || generation == 0 || generation > 9_007_199_254_740_991 {
        return Err("Invalid source write owner or generation".into());
    }
    // Flatten the session receipt without acknowledging any newer snapshot.
    match ordered_source_call(owner, session, move |order, owner, session| {
        order.apply(owner, session, &key, generation, work)
    })
    .await?
    {
        murmur_core::source_order::SourceWrite::Applied { value } => Ok(value),
        murmur_core::source_order::SourceWrite::Superseded => {
            Ok(murmur_core::source_order::SourceWrite::Superseded)
        }
    }
}

#[cfg(test)]
pub(crate) fn register_test_source(owner: &str, session: &str) {
    tauri::async_runtime::block_on(register_source_owner(
        owner.into(),
        session.into(),
        WriterPermit::accepted_producer(),
    ))
    .unwrap();
}

#[cfg(feature = "shutdown-test-support")]
pub(crate) mod test_support {
    use super::*;
    use std::sync::OnceLock;
    static EVIDENCE: OnceLock<Mutex<Option<qa_evidence::Evidence>>> = OnceLock::new();
    static COUNTS: OnceLock<Arc<qa_evidence::DispatcherCounts>> = OnceLock::new();

    pub(super) fn evidence_enabled() -> bool {
        EVIDENCE
            .get()
            .is_some_and(|evidence| evidence.lock_or_recover().is_some())
    }

    pub(super) fn initialize_evidence(app: &tauri::AppHandle) {
        let evidence = qa_evidence::Evidence::new(
            &app.config().identifier,
            crate::identity::initialized_identifier(),
            || app.path().app_data_dir().ok(),
        );
        let _ = EVIDENCE.set(Mutex::new(evidence));
    }
    pub(super) fn transition(
        before: murmur_core::shutdown::State,
        core: &Coordinator,
        event: Option<Event>,
        request: Option<Request>,
        now: u64,
    ) {
        if let Some(evidence) = EVIDENCE.get() {
            if let Some(evidence) = evidence.lock_or_recover().as_mut() {
                evidence.observe(before, core, event, request, now);
            }
        }
    }
    pub(super) fn write_terminal_evidence(app: &tauri::AppHandle, action: Action) {
        if !isolated(app) {
            return;
        }
        if let Some(evidence) = EVIDENCE.get() {
            if let Some(evidence) = evidence.lock_or_recover().as_mut() {
                let counts = COUNTS
                    .get()
                    .map(|counts| counts.snapshot())
                    .unwrap_or_default();
                if evidence.write_terminal(action, counts).is_err() {
                    // Do not route evidence failures through the sealed logger or
                    // include paths / I/O error messages in the diagnostic.
                    eprintln!("shutdown QA evidence: receipt_io_failure");
                }
            }
        }
    }
    fn isolated(app: &tauri::AppHandle) -> bool {
        app.config().identifier == "com.localdictation.shutdown-test"
    }
    pub(super) async fn delay(app: &tauri::AppHandle, step: Step) {
        if !isolated(app) {
            return;
        }
        let name = match step {
            Step::Admission => "ADMISSION",
            Step::Capture => "CAPTURE",
            Step::Helpers => "HELPERS",
            Step::Writers => "WRITERS",
            Step::Terminate => return,
        };
        if let Some(ms) = std::env::var(format!("MURMUR_SHUTDOWN_DELAY_{name}_MS"))
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            tokio::time::sleep(Duration::from_millis(ms.min(20_000))).await;
        }
    }
    /// After real owner work, hold final release for >=6 s. Each dispatcher
    /// must execute repeatedly while an actual AppKit termination wait is active.
    pub(super) fn hold(
        app: &tauri::AppHandle,
        context: &AppContext,
        receive: &mpsc::Receiver<Message>,
        core: &Mutex<Coordinator>,
    ) {
        if !isolated(app) {
            return;
        }
        let counts = Arc::new(qa_evidence::DispatcherCounts::default());
        let _ = COUNTS.set(counts.clone());
        for _ in 0..65 {
            for message in receive.try_iter() {
                let _ = super::reduce(core, Ok(message), 0);
            }
            let count = counts.clone();
            let _ = context.main_thread.dispatch(Box::new(move || {
                assert!(objc2::MainThreadMarker::new().is_some());
                count.app_context.fetch_add(1, Ordering::AcqRel);
            }));
            let count = counts.clone();
            let _ = app.run_on_main_thread(move || {
                assert!(objc2::MainThreadMarker::new().is_some());
                count.tauri_dispatch.fetch_add(1, Ordering::AcqRel);
            });
            let count = counts.clone();
            let block = block2::RcBlock::new(move || {
                assert!(objc2::MainThreadMarker::new().is_some());
                count.cf_heartbeat.fetch_add(1, Ordering::AcqRel);
            });
            let run_loop = objc2_core_foundation::CFRunLoop::main().unwrap();
            unsafe {
                run_loop.perform_block(
                    Some(
                        objc2_core_foundation::kCFRunLoopCommonModes
                            .unwrap()
                            .as_ref(),
                    ),
                    Some(&block),
                );
            }
            run_loop.wake_up();
            std::thread::sleep(Duration::from_millis(100));
        }
        tracing::info!(target:"shutdown", callbacks=counts.snapshot().total(), expected=195, "synthetic shutdown dispatcher receipt");
    }
    pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
        tauri::plugin::Builder::new("shutdown-test")
            .invoke_handler(tauri::generate_handler![shutdown_test_action])
            .build()
    }
    #[tauri::command]
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn shutdown_test_action(
        app: tauri::AppHandle,
        window: tauri::WebviewWindow,
        action: String,
    ) -> Result<(), String> {
        if !isolated(&app) || window.label() != "main" {
            return Err("Isolated shutdown fixture required".into());
        }
        match action.as_str() {
            "exit" => app.exit(0),
            "destroy-last" => {
                for (_, window) in app.webview_windows() {
                    window
                        .destroy()
                        .map_err(|_| "Fixture window destruction failed")?;
                }
            }
            "quit" => request(&app, Request::Quit),
            "relaunch" => request(&app, Request::Restart),
            _ => return Err("Unknown fixture action".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_shutdown_acl_is_feature_gated_and_main_only() {
        let mut context = crate::generated_context();
        let authority = context.runtime_authority_mut();
        for window in ["main", "diagnostics", "overlay", "dictation-preview"] {
            assert_eq!(
                authority
                    .resolve_access(
                        "plugin:shutdown-test|shutdown_test_action",
                        window,
                        window,
                        &tauri::ipc::Origin::Local,
                    )
                    .is_some(),
                cfg!(feature = "shutdown-test-support") && window == "main",
                "{window}",
            );
            // Evidence is native file output, never a webview command.
            assert!(authority
                .resolve_access(
                    "plugin:shutdown-test|read_terminal_evidence",
                    window,
                    window,
                    &tauri::ipc::Origin::Local,
                )
                .is_none());
        }
    }
    #[test]
    fn telemetry_success_preserves_content_writer_outcome() {
        for content in [
            WriteOutcome::Committed,
            WriteOutcome::PreservedForReplay,
            WriteOutcome::UnpreservedFailure,
        ] {
            assert_eq!(writer_outcome(content, true), content);
            assert_eq!(
                writer_outcome(content, false),
                WriteOutcome::UnpreservedFailure
            );
        }
    }

    #[test]
    fn source_task_polled_out_of_order_never_publishes_older_content() {
        use murmur_core::source_order::SourceWrite;
        let root = tempfile::tempdir().unwrap();
        for key in [
            "dictation-settings",
            "murmur-theme-library",
            "usage-dashboard-collapsed",
        ] {
            let session = uuid::Uuid::new_v4().to_string();
            register_test_source("main", &session);
            let path = root.path().join(key);
            let older_path = path.clone();
            // Construct A first but poll B first, exactly the IPC scheduling race.
            let older =
                versioned_source_call("main".into(), session.clone(), key.into(), 1, move || {
                    std::fs::write(older_path, b"older")
                        .map_err(|_| "synthetic write failure".into())
                });
            let newer_path = path.clone();
            let newer = versioned_source_call("main".into(), session, key.into(), 2, move || {
                std::fs::write(newer_path, b"newer").map_err(|_| "synthetic write failure".into())
            });
            assert_eq!(
                tauri::async_runtime::block_on(newer).unwrap(),
                SourceWrite::Applied { value: () }
            );
            assert_eq!(
                tauri::async_runtime::block_on(older).unwrap(),
                SourceWrite::Superseded
            );
            assert_eq!(std::fs::read(path).unwrap(), b"newer");
        }
    }

    #[test]
    fn retired_renderer_generation_100_cannot_overwrite_reloaded_generation_1() {
        use murmur_core::source_order::SourceWrite;
        let root = tempfile::tempdir().unwrap();
        for key in [
            "dictation-settings",
            "murmur-theme-library",
            "usage-dashboard-collapsed",
        ] {
            let old_session = uuid::Uuid::new_v4().to_string();
            register_test_source("main", &old_session);
            let path = root.path().join(key);
            let old_path = path.clone();
            let old = versioned_source_call(
                "main".into(),
                old_session.clone(),
                key.into(),
                100,
                move || std::fs::write(old_path, b"old").map_err(|e| e.to_string()),
            );
            let new_session = uuid::Uuid::new_v4().to_string();
            register_test_source("main", &new_session);
            let new_path = path.clone();
            assert_eq!(
                tauri::async_runtime::block_on(versioned_source_call(
                    "main".into(),
                    new_session,
                    key.into(),
                    1,
                    move || std::fs::write(new_path, b"new").map_err(|e| e.to_string())
                ))
                .unwrap(),
                SourceWrite::Applied { value: () }
            );
            assert_eq!(
                tauri::async_runtime::block_on(old).unwrap(),
                SourceWrite::Superseded
            );
            assert_eq!(std::fs::read(path).unwrap(), b"new");
            // A delayed registration retry cannot resurrect the retired session.
            assert!(tauri::async_runtime::block_on(register_source_owner(
                "main".into(),
                old_session,
                WriterPermit::accepted_producer()
            ))
            .is_err());
        }
    }

    #[test]
    fn idle_presentation_cannot_overtake_final_native_completion() {
        use murmur_core::completion::{Command, CompletionService};
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        service.execute(Command::Snapshot).unwrap();
        let producer = WriterPermit::accepted_producer();
        let completion = service.clone();
        let (idle, observed_idle) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let owner = std::thread::spawn(move || {
            let _producer = producer;
            idle.send(()).unwrap(); // The real owners publish Idle before this seam.
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            completion.execute(Command::Complete { source: "synthetic-terminal".into(), entry: Some(serde_json::json!({"schemaVersion":2,"id":"synthetic","text":"synthetic","timestamp":1,"duration":1,"source":"recording"})), event: None }).unwrap();
        });
        observed_idle.recv().unwrap();
        let (started, barrier_started) = mpsc::channel();
        let (done, barrier_done) = mpsc::channel();
        let barrier = std::thread::spawn(move || {
            started.send(()).unwrap();
            wait_accepted_producers();
            done.send(()).unwrap();
        });
        barrier_started.recv().unwrap();
        assert!(matches!(
            barrier_done.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        owner.join().unwrap();
        barrier_done.recv_timeout(Duration::from_secs(5)).unwrap();
        barrier.join().unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(service.shutdown_receipt()),
            WriteOutcome::Committed
        );
        drop(service);
        let next = CompletionService::start(root.path().into(), false).unwrap();
        assert_eq!(next.execute(Command::Snapshot).unwrap().history.len(), 1);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quit_reaches_terminal_while_startup_settings_io_is_blocked() {
        let (entered, blocked) = tokio::sync::oneshot::channel();
        let (release, resume) = mpsc::channel();
        let permit = WriterPermit::acquire().unwrap();
        let startup = tokio::spawn(async move {
            let _permit = permit;
            source_call(move || {
                assert!(objc2::MainThreadMarker::new().is_none());
                entered.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
            })
            .await
            .unwrap();
        });
        blocked.await.unwrap();
        let core = Mutex::new(Coordinator::default());
        let (send, receive) = mpsc::channel();
        adapter::accept(&core, &send, Request::Quit, 0);
        adapter::reduce(&core, Ok(receive.recv().unwrap()), 0);
        adapter::reduce(&core, Err(mpsc::RecvTimeoutError::Timeout), 10_000);
        assert_eq!(core.lock().unwrap().issue(), Some(Action::Quit));
        assert!(!startup.is_finished());
        release.send(()).unwrap();
        startup.await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_source_fifo_keeps_queued_intent_while_first_writer_is_blocked() {
        let (started, entered) = tokio::sync::oneshot::channel();
        let (release, resume) = mpsc::channel();
        let order = Arc::new(Mutex::new(vec![]));
        let first_order = order.clone();
        let first = tokio::spawn(source_call(move || {
            assert!(objc2::MainThreadMarker::new().is_none());
            let _ = started.send(());
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
            first_order.lock().unwrap().push(1);
        }));
        entered.await.unwrap();
        let second_order = order.clone();
        let second = tokio::spawn(source_call(move || {
            second_order.lock().unwrap().push(2);
        }));
        assert!(order.lock().unwrap().is_empty());
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(*order.lock().unwrap(), [1, 2]);
    }
}
