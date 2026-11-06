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
#[cfg(target_os = "macos")]
mod macos;
enum Message {
    Request(Request),
    Finished(Event),
}
pub(crate) struct Shutdown {
    core: Arc<Mutex<Coordinator>>,
    send: mpsc::Sender<Message>,
}
pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    let core = Arc::new(Mutex::new(Coordinator::default()));
    let (send, receive) = mpsc::channel();
    app.manage(Shutdown {
        core: core.clone(),
        send: send.clone(),
    });
    #[cfg(target_os = "macos")]
    macos::install(app.clone())?;
    let handle = app.clone();
    let context = app.state::<AppContext>().inner().clone();
    std::thread::Builder::new()
        .name("shutdown-deadlines".into())
        .spawn(move || {
            let clock = Instant::now();
            loop {
                let now = || clock.elapsed().as_millis() as u64;
                let message = receive.recv_timeout(Duration::from_millis(10));
                let effects = {
                    let mut core = core.lock_or_recover();
                    match message {
                        Ok(Message::Request(request)) => core.request(request, now()),
                        Ok(Message::Finished(event)) => core.advance(event, now()),
                        Err(mpsc::RecvTimeoutError::Timeout) => core.tick(now()),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                };
                for effect in effects {
                    if effect.step == Step::Terminate {
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
                        let outcome = run_owner(&app, effect, deadline).await;
                        let _ = send.send(Message::Finished(Event { effect, outcome }));
                    }));
                }
            }
        })
        .map_err(|_| "Shutdown deadline driver unavailable")?;
    signals(app.clone())?;
    Ok(())
}
pub(crate) fn request(app: &tauri::AppHandle, request: Request) {
    close_admission(); // synchronous even if the reducer/owner thread is delayed
    if let Some(shutdown) = app.try_state::<Shutdown>() {
        let _ = shutdown.send.send(Message::Request(request));
    }
}
pub(crate) fn permitted(app: &tauri::AppHandle) -> bool {
    app.try_state::<Shutdown>()
        .is_some_and(|s| s.core.lock_or_recover().termination_permitted())
}
pub(crate) fn receipt(app: &tauri::AppHandle) {
    if let Some(shutdown) = app.try_state::<Shutdown>() {
        let mut core = shutdown.core.lock_or_recover();
        tracing::info!(target:"shutdown", permitted=core.termination_permitted(), receipt=?core.receipt, "terminal shutdown receipt");
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
            let app = app.clone();
            let committed = tauri::async_runtime::spawn_blocking(move || {
                while WRITERS.load(Ordering::Acquire) != 0 {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let sources = crate::commands::settings_store::flush(&app);
                let windows = crate::window_state::flush(&app);
                let memo = crate::audio::flush_capture_memo();
                sources && windows && memo && crate::completion::shutdown_flush(&app)
            })
            .await
            .unwrap_or(false);
            Outcome::Writers(if committed {
                WriteOutcome::Committed
            } else {
                WriteOutcome::UnpreservedFailure
            })
        }
        Step::Terminate => unreachable!(),
    }
}
fn terminal(app: &tauri::AppHandle, action: Action) {
    // This is the sole application restart API call; the reducer committed its
    // drained permit before queuing this callback.
    if !permitted(app) {
        return;
    }
    #[cfg(target_os = "macos")]
    if macos::reply_if_pending() {
        return;
    }
    match action {
        Action::Restart => app.request_restart(),
        Action::Quit => app.exit(0),
    }
}

/// The raw handler does nothing except a nonblocking async-signal-safe write.
/// Locks, allocation and host calls live on the reader thread.
#[cfg(unix)]
fn signals(app: tauri::AppHandle) -> Result<(), String> {
    use std::sync::atomic::AtomicI32;
    static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);
    extern "C" fn notify(signal: libc::c_int) {
        let fd = SIGNAL_FD.load(Ordering::Relaxed);
        let byte = signal as u8;
        if fd >= 0 {
            unsafe {
                libc::write(fd, (&byte as *const u8).cast(), 1);
            }
        }
    }
    let mut fds = [-1; 2];
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err("Shutdown self-pipe unavailable".into());
        }
        libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK);
        for fd in fds {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
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
