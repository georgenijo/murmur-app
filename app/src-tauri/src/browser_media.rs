//! Optional "Pause browser media while dictating" (#835), off by default.
//!
//! Adapted from Hex's `browser_media.rs` (MIT, Copyright (c) 2026 Christopher
//! Langton). macOS no longer tells other apps what is playing, so Murmur
//! listens to the browsers' own output: the level says whether the system
//! pause reached a browser, and only that proof allows the play at the end.
//! Murmur never sends play for media it did not prove it paused, and never
//! the play/pause toggle, which can start media that was not playing.
//!
//! The app process opens no audio device. A short-lived signed capture worker
//! (`--browser-level`) owns the private process tap and answers bounded level
//! questions; this module owns the decision and sends the MediaRemote command.
//! The tap needs macOS System Audio Recording consent. Murmur asks for it only
//! when the user turns the setting on, and never taps without it, so no system
//! dialog can open while dictating.

use crate::state::DictationStatus;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// Sound: a peak above this in every listen window.
const SOUND_DBFS: f32 = -60.0;
/// Silence: a peak below this in consecutive windows soon after the pause.
const SILENCE_DBFS: f32 = -90.0;
const WINDOW: Duration = Duration::from_millis(100);
const LISTEN_WINDOWS: usize = 3;
/// The silence must start in the first windows after the pause: a sound
/// that ends later on its own is not the pause reaching the browser.
const CONFIRM_WINDOWS: usize = 5;
const SILENT_WINDOWS: usize = 2;
/// The last listen before play.
const RESUME_LISTEN: Duration = Duration::from_millis(300);
/// After play, the browser must sound within this many windows (2 s). If it
/// does not, the play reached something else, so Murmur pauses it again.
const PLAY_CHECK_WINDOWS: usize = 20;
/// A browser that closed its output opens it again within this many windows
/// of play. If none does, the play reached another app: pause it sooner.
const OPEN_CHECK_WINDOWS: usize = 10;

/// While dictating, Murmur keeps listening to every browser.
const WATCH_WINDOW: Duration = Duration::from_millis(500);

/// What one watch window means for the pause proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Watch {
    /// Silent, or every browser output closed (a long pause): keep the proof.
    Keep,
    /// A browser sounds again: the user played it or another one, so the
    /// Now Playing target may have changed. A later pause by the user must
    /// never get play.
    Restarted,
    /// Murmur cannot tell silence from a tap that stopped delivering:
    /// ownership is uncertain, so give it up.
    Lost,
}

/// `closed`: the worker confirmed every browser closed its output, the only
/// reason a working tap delivers nothing.
fn watch(peak: Option<f32>, listening: bool, closed: bool) -> Watch {
    match peak {
        Some(peak) if peak > SOUND_DBFS => Watch::Restarted,
        _ if !listening => Watch::Lost,
        Some(_) => Watch::Keep,
        None if closed => Watch::Keep,
        None => Watch::Lost,
    }
}

/// The explicit MediaRemote commands. Never the toggle: it can start music.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub(crate) enum MediaCommand {
    Play = 0,
    Pause = 1,
}

pub(crate) trait BrowserAudio {
    /// A browser is using the microphone: probably a call.
    fn in_call(&mut self) -> bool;
    /// Peak of the browsers' output over `window`, in dBFS; None when it cannot listen.
    fn peak_dbfs(&mut self, window: Duration) -> Option<f32>;
    fn send(&mut self, command: MediaCommand) -> bool;
    /// A browser process runs and every one has closed its audio output.
    fn output_closed(&mut self) -> bool;
}

/// Proof that Murmur paused a sounding browser; only this allows play.
#[derive(Debug)]
pub(crate) struct PausedBrowser(());

#[cfg(test)]
impl PausedBrowser {
    fn for_test() -> Self {
        Self(())
    }
}

/// Why a browser was left alone (or not proven paused). Content-free codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PauseSkip {
    /// System Audio Recording is not granted: tapping would prompt.
    NoPermission,
    InCall,
    Silent,
    Unheard,
    Refused,
    Unconfirmed,
}

impl PauseSkip {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoPermission => "no_permission",
            Self::InCall => "in_call",
            Self::Silent => "silent",
            Self::Unheard => "unheard",
            Self::Refused => "refused",
            Self::Unconfirmed => "unconfirmed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResumeOutcome {
    Resumed,
    /// The user started it again before dictation ended.
    AlreadyPlaying,
    Unheard,
    Refused,
    /// Play was sent; the tap stopped delivering afterwards.
    LostAfterPlay,
    /// Play was sent; the browser never sounded, so pause went out again.
    Repaused,
}

impl ResumeOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Resumed => "resumed",
            Self::AlreadyPlaying => "already_playing",
            Self::Unheard => "unheard",
            Self::Refused => "refused",
            Self::LostAfterPlay => "lost_after_play",
            Self::Repaused => "repaused",
        }
    }
}

pub(crate) fn pause(audio: &mut impl BrowserAudio) -> Result<PausedBrowser, PauseSkip> {
    if audio.in_call() {
        return Err(PauseSkip::InCall);
    }
    for _ in 0..LISTEN_WINDOWS {
        match audio.peak_dbfs(WINDOW) {
            Some(peak) if peak > SOUND_DBFS => {}
            Some(_) => return Err(PauseSkip::Silent),
            None => return Err(PauseSkip::Unheard),
        }
    }
    if !audio.send(MediaCommand::Pause) {
        return Err(PauseSkip::Refused);
    }
    let mut silent = 0;
    for _ in 0..CONFIRM_WINDOWS {
        if audio
            .peak_dbfs(WINDOW)
            .is_some_and(|peak| peak < SILENCE_DBFS)
        {
            silent += 1;
            if silent == SILENT_WINDOWS {
                return Ok(PausedBrowser(()));
            }
        } else {
            silent = 0;
        }
    }
    // Another tab, a sound that ended later, or a pause that reached another
    // app: this browser never gets play.
    Err(PauseSkip::Unconfirmed)
}

pub(crate) fn resume(audio: &mut impl BrowserAudio, _paused: PausedBrowser) -> ResumeOutcome {
    let mut heard = true;
    match audio.peak_dbfs(RESUME_LISTEN) {
        Some(peak) if peak <= SOUND_DBFS => {}
        Some(_) => return ResumeOutcome::AlreadyPlaying,
        // A pause longer than about 10 s closes the browser's output, and a
        // tap on closed outputs delivers nothing: that browser is still paused.
        None if audio.output_closed() => heard = false,
        None => return ResumeOutcome::Unheard,
    }
    if !audio.send(MediaCommand::Play) {
        return ResumeOutcome::Refused;
    }
    // The tap delivers again once play opens the output, about 300 ms later.
    for window in 1..=PLAY_CHECK_WINDOWS {
        match audio.peak_dbfs(WINDOW) {
            Some(peak) if peak > SOUND_DBFS => return ResumeOutcome::Resumed,
            Some(_) => heard = true,
            None if !heard => {
                if window == OPEN_CHECK_WINDOWS && audio.output_closed() {
                    break;
                }
            }
            None => return ResumeOutcome::LostAfterPlay,
        }
    }
    if !heard && !audio.output_closed() {
        return ResumeOutcome::LostAfterPlay;
    }
    // A closed or reloaded tab stays silent, and the play went to whatever
    // macOS now treats as playing: pause that again.
    audio.send(MediaCommand::Pause);
    ResumeOutcome::Repaused
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Job {
    Pause(u64),
    Resume(u64),
}

/// Map one dictation status transition to the work it starts. Recovery keeps
/// the same recording, so only Starting → Recording pauses; leaving the live
/// states for Processing or Idle resumes.
fn job_for_transition(
    recording_id: u64,
    from: DictationStatus,
    to: DictationStatus,
) -> Option<Job> {
    use DictationStatus::{Idle, Processing, Recording, Recovering, Starting};
    if recording_id == 0 {
        return None;
    }
    match (from, to) {
        (Starting, Recording) => Some(Job::Pause(recording_id)),
        (Starting | Recording | Recovering, Idle | Processing) => Some(Job::Resume(recording_id)),
        _ => None,
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
/// The recording a pause was requested for; 0 once it ended.
static ACTIVE_RECORDING: AtomicU64 = AtomicU64::new(0);

pub(crate) fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

/// Called from the single dictation-status seam. Never blocks: the work runs
/// in order on one background thread, so a resume always follows its pause.
pub(crate) fn dictation_status_changed(
    recording_id: u64,
    from: DictationStatus,
    to: DictationStatus,
) {
    let job = match job_for_transition(recording_id, from, to) {
        // The setting is read once at recording start; turning it off later
        // still lets Murmur resume what it already paused.
        Some(Job::Pause(id)) if ENABLED.load(Ordering::SeqCst) => {
            ACTIVE_RECORDING.store(id, Ordering::SeqCst);
            Job::Pause(id)
        }
        // Only a recording that requested a pause has anything to resume.
        Some(Job::Resume(id))
            if ACTIVE_RECORDING
                .compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok() =>
        {
            Job::Resume(id)
        }
        _ => return,
    };
    native::enqueue(job);
}

/// Content-free System Audio Recording consent state for the settings UI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMediaPermission {
    Granted,
    Denied,
    NotDetermined,
    Unavailable,
}

#[tauri::command]
pub fn get_browser_media_permission() -> BrowserMediaPermission {
    native::authorization()
}

/// Shows the macOS System Audio Recording prompt when consent is undecided.
/// The settings UI calls this only when the user turns the setting on.
#[tauri::command]
pub async fn request_browser_media_permission() -> BrowserMediaPermission {
    tauri::async_runtime::spawn_blocking(native::request_authorization)
        .await
        .unwrap_or(BrowserMediaPermission::Unavailable)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod native {
    use super::{
        pause, resume, BrowserAudio, BrowserMediaPermission, Job, MediaCommand, PausedBrowser,
        ACTIVE_RECORDING,
    };
    use super::{watch, Watch, WATCH_WINDOW};
    use crate::managed_child::{bundled_sibling, ManagedChild};
    use std::ffi::c_void;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const CAPTURE_WORKER_IDENTIFIER: &str = "com.localdictation.capture-worker";
    /// Spawn, enumerate, and tap warm-up (300 ms in the worker).
    const READY_DEADLINE: Duration = Duration::from_secs(2);
    const REPLY_SLACK: Duration = Duration::from_millis(750);
    const STOP_DEADLINE: Duration = Duration::from_millis(500);
    const MAX_REPLY_BYTES: u64 = 32;
    const PERMISSION_DEADLINE: Duration = Duration::from_secs(120);

    pub(super) fn enqueue(job: Job) {
        static JOBS: OnceLock<Option<Mutex<Sender<Job>>>> = OnceLock::new();
        let sender = JOBS.get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("murmur-browser-media".into())
                .spawn(move || run_jobs(receiver))
                .ok()
                .map(|_| Mutex::new(sender))
        });
        if let Some(sender) = sender {
            let _ = sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .send(job);
        }
    }

    /// The pause Murmur owns, and the tap that keeps watching it.
    struct Held {
        recording_id: u64,
        proof: PausedBrowser,
        watch: WorkerBrowserAudio,
    }

    fn run_jobs(receiver: Receiver<Job>) {
        let mut paused: Option<Held> = None;
        loop {
            let job = match paused.as_mut() {
                None => match receiver.recv() {
                    Ok(job) => job,
                    Err(_) => return,
                },
                Some(held) => match receiver.try_recv() {
                    Ok(job) => job,
                    Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => {
                        let peak = held.watch.peak_dbfs(WATCH_WINDOW);
                        let closed = peak.is_none() && held.watch.output_closed();
                        let outcome = watch(peak, held.watch.listening(), closed);
                        if outcome != Watch::Keep {
                            tracing::info!(
                                target: "audio",
                                event_code = "audio.browser_media_released",
                                recording_id = held.recording_id,
                                outcome = match outcome {
                                    Watch::Restarted => "restarted",
                                    _ => "lost",
                                },
                                "browser media pause released"
                            );
                            paused = None;
                        }
                        continue;
                    }
                },
            };
            match job {
                Job::Pause(recording_id) => {
                    // A recording that already ended (a quick tap) needs no pause.
                    if paused.is_some() || ACTIVE_RECORDING.load(Ordering::SeqCst) != recording_id {
                        continue;
                    }
                    let started = Instant::now();
                    let granted = authorization() == BrowserMediaPermission::Granted;
                    let mut audio = WorkerBrowserAudio::new(Listen::Playing);
                    let result = if granted {
                        pause(&mut audio)
                    } else {
                        Err(super::PauseSkip::NoPermission)
                    };
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.browser_media_pause",
                        recording_id,
                        outcome = match &result {
                            Ok(_) => "paused",
                            Err(skip) => skip.as_str(),
                        },
                        permission_granted = granted,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "browser media pause finished"
                    );
                    if let Ok(proof) = result {
                        // The pause tap covers every browser and keeps
                        // listening, with no unobserved handoff: any browser
                        // sounding may move Now Playing, after which play
                        // could start media that was not playing.
                        paused = Some(Held {
                            recording_id,
                            proof,
                            watch: audio,
                        });
                    }
                }
                Job::Resume(recording_id) => {
                    let Some(held) = paused.take_if(|held| held.recording_id == recording_id)
                    else {
                        continue;
                    };
                    // A newer recording already started: it inherits the pause
                    // instead of the browser playing into its first second.
                    let newer = ACTIVE_RECORDING.load(Ordering::SeqCst);
                    if newer != 0 {
                        paused = Some(Held {
                            recording_id: newer,
                            ..held
                        });
                        continue;
                    }
                    let Held { proof, watch, .. } = held;
                    drop(watch);
                    let started = Instant::now();
                    let outcome = resume(&mut WorkerBrowserAudio::new(Listen::All), proof);
                    tracing::info!(
                        target: "audio",
                        event_code = "audio.browser_media_resume",
                        recording_id,
                        outcome = outcome.as_str(),
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "browser media resume finished"
                    );
                }
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Listen {
        /// Every browser, only when one has open output: a closed output is silent.
        Playing,
        /// Every browser: a paused one closes its output a few seconds later.
        All,
    }

    struct Worker {
        child: ManagedChild,
        input: Option<std::process::ChildStdin>,
        replies: Receiver<String>,
        in_call: bool,
        /// The worker created its tap. Without one, "no level" proves nothing.
        tapped: bool,
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            // Closing stdin ends the worker's loop, which tears the tap down.
            drop(self.input.take());
            if self
                .child
                .wait_for_exit(Instant::now() + STOP_DEADLINE)
                .is_none()
            {
                let _ = self
                    .child
                    .hard_kill_confirmed(Instant::now() + STOP_DEADLINE);
            }
        }
    }

    /// One worker per pause or resume: the tap covers the browser processes
    /// running when it first listens.
    struct WorkerBrowserAudio {
        listen: Listen,
        worker: Option<Worker>,
        started: bool,
    }

    impl WorkerBrowserAudio {
        fn new(listen: Listen) -> Self {
            Self {
                listen,
                worker: None,
                started: false,
            }
        }

        /// The worker still runs with a tap.
        fn listening(&self) -> bool {
            self.worker.as_ref().is_some_and(|worker| worker.tapped)
        }

        fn worker(&mut self) -> Option<&mut Worker> {
            if !self.started {
                self.started = true;
                self.worker = start_worker(self.listen);
            }
            self.worker.as_mut()
        }

        fn ask(&mut self, request: &str, wait: Duration) -> Option<String> {
            let worker = self.worker()?;
            let asked = match worker.input.as_mut() {
                Some(input) => input
                    .write_all(request.as_bytes())
                    .and_then(|()| input.flush()),
                None => return None,
            };
            if asked.is_err() {
                self.worker = None;
                return None;
            }
            match worker.replies.recv_timeout(wait + REPLY_SLACK) {
                Ok(reply) => Some(reply),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                    self.worker = None;
                    None
                }
            }
        }
    }

    impl BrowserAudio for WorkerBrowserAudio {
        fn in_call(&mut self) -> bool {
            // Fail closed: no worker means Murmur cannot prove there is no call.
            self.worker().is_none_or(|worker| worker.in_call)
        }

        fn peak_dbfs(&mut self, window: Duration) -> Option<f32> {
            let request = format!("peak {}\n", window.as_millis().clamp(1, 1_000));
            parse_peak(&self.ask(&request, window)?)?
        }

        fn send(&mut self, command: MediaCommand) -> bool {
            send_media_command(command)
        }

        fn output_closed(&mut self) -> bool {
            // A failed tap also delivers nothing; only a working tap's silence
            // means the browser closed its output while paused.
            if !self.worker().is_some_and(|worker| worker.tapped) {
                return false;
            }
            self.ask("closed\n", Duration::ZERO)
                .is_some_and(|reply| reply == "closed 1")
        }
    }

    /// `peak <dBFS>` → Some(Some(level)); `peak none` → Some(None).
    pub(super) fn parse_peak(reply: &str) -> Option<Option<f32>> {
        match reply.strip_prefix("peak ")? {
            "none" => Some(None),
            level => level
                .parse::<f32>()
                .ok()
                .filter(|level| level.is_finite() && (-200.0..=40.0).contains(level))
                .map(Some),
        }
    }

    /// `ready <in_call> <tapped>`.
    pub(super) fn parse_ready(reply: &str) -> Option<(bool, bool)> {
        let flag = |value: &str| match value {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        let mut parts = reply.strip_prefix("ready ")?.split(' ');
        let ready = (flag(parts.next()?)?, flag(parts.next()?)?);
        parts.next().is_none().then_some(ready)
    }

    fn start_worker(listen: Listen) -> Option<Worker> {
        let path = bundled_sibling("murmur-capture-worker")
            .or_else(|_| bundled_sibling("murmur-capture-worker-aarch64-apple-darwin"))
            .ok()?;
        if !cfg!(debug_assertions)
            && crate::code_signing::validate_bundled_helper(&path, CAPTURE_WORKER_IDENTIFIER)
                .is_err()
        {
            tracing::warn!(target: "audio", "browser media worker signature is invalid");
            return None;
        }
        let safari_pids: Vec<String> = safari_media_pids()
            .into_iter()
            .map(|pid| pid.to_string())
            .collect();
        let mut arguments = vec![
            "--browser-level",
            match listen {
                Listen::Playing => "playing",
                Listen::All => "all",
            },
        ];
        arguments.extend(safari_pids.iter().map(String::as_str));
        let (child, input, output) =
            ManagedChild::spawn_with_arguments(&path, &arguments, &[]).ok()?;
        let (sender, replies) = mpsc::channel();
        std::thread::Builder::new()
            .name("murmur-browser-level-reader".into())
            .spawn(move || read_replies(output, sender))
            .ok()?;
        let mut worker = Worker {
            child,
            input: Some(input),
            replies,
            in_call: true,
            tapped: false,
        };
        let (in_call, tapped) = worker
            .replies
            .recv_timeout(READY_DEADLINE)
            .ok()
            .and_then(|reply| parse_ready(&reply))?;
        worker.in_call = in_call;
        worker.tapped = tapped;
        Some(worker)
    }

    fn read_replies(output: std::process::ChildStdout, sender: Sender<String>) {
        let mut output = BufReader::new(output);
        loop {
            let mut line = String::new();
            match (&mut output).take(MAX_REPLY_BYTES).read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) if !line.ends_with('\n') => return,
                Ok(_) => {}
            }
            line.pop();
            if sender.send(line).is_err() {
                return;
            }
        }
    }

    /// Safari plays through a `com.apple.WebKit.GPU` process told apart from
    /// other WebKit clients (Mail, help viewers) only by its display name.
    fn safari_media_pids() -> Vec<i32> {
        use objc2_app_kit::NSRunningApplication;
        let mut pids = vec![0_i32; 4_096];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
        let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if count <= 0 {
            return Vec::new();
        }
        pids.truncate((count as usize).min(pids.len()));
        objc2::rc::autoreleasepool(|_| {
            pids.into_iter()
                .filter(|&pid| {
                    pid > 1 && process_name(pid).as_deref() == Some("com.apple.WebKit.GPU")
                })
                .filter(|&pid| {
                    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
                        .and_then(|application| application.localizedName())
                        .is_some_and(|name| name.to_string().starts_with("Safari"))
                })
                .take(32)
                .collect()
        })
    }

    fn process_name(pid: i32) -> Option<String> {
        let mut name = [0_u8; 64];
        let length = unsafe { libc::proc_name(pid, name.as_mut_ptr().cast(), name.len() as u32) };
        (length > 0).then(|| String::from_utf8_lossy(&name[..length as usize]).into_owned())
    }

    type SendCommand = unsafe extern "C" fn(u32, *const c_void) -> u8;

    /// `MRMediaRemoteSendCommand` from the private MediaRemote framework: the
    /// same pause and play as the media keys, sent to the app macOS has as Now Playing.
    fn send_media_command(command: MediaCommand) -> bool {
        static SEND: OnceLock<Option<SendCommand>> = OnceLock::new();
        let send = SEND.get_or_init(|| unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote".as_ptr(),
                libc::RTLD_LAZY,
            );
            if handle.is_null() {
                return None;
            }
            let symbol = libc::dlsym(handle, c"MRMediaRemoteSendCommand".as_ptr());
            (!symbol.is_null()).then(|| std::mem::transmute::<*mut c_void, SendCommand>(symbol))
        });
        let Some(send) = send else {
            return false;
        };
        unsafe { send(command as u32, std::ptr::null()) != 0 }
    }

    type TccPreflight = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
    type TccRequest = unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void);

    struct Tcc {
        preflight: TccPreflight,
        request: TccRequest,
    }

    fn tcc() -> Option<&'static Tcc> {
        static TCC: OnceLock<Option<Tcc>> = OnceLock::new();
        TCC.get_or_init(|| unsafe {
            let handle = libc::dlopen(
                c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(),
                libc::RTLD_NOW,
            );
            if handle.is_null() {
                return None;
            }
            let preflight = libc::dlsym(handle, c"TCCAccessPreflight".as_ptr());
            let request = libc::dlsym(handle, c"TCCAccessRequest".as_ptr());
            if preflight.is_null() || request.is_null() {
                return None;
            }
            Some(Tcc {
                preflight: std::mem::transmute::<*mut c_void, TccPreflight>(preflight),
                request: std::mem::transmute::<*mut c_void, TccRequest>(request),
            })
        })
        .as_ref()
    }

    /// macOS "System Audio Recording" consent (TCC `kTCCServiceAudioCapture`),
    /// which process taps need. Never prompts.
    pub(super) fn authorization() -> BrowserMediaPermission {
        let Some(tcc) = tcc() else {
            return BrowserMediaPermission::Unavailable;
        };
        let service = objc2_core_foundation::CFString::from_static_str("kTCCServiceAudioCapture");
        let result = unsafe {
            (tcc.preflight)(
                (&*service as *const objc2_core_foundation::CFString).cast(),
                std::ptr::null(),
            )
        };
        match result {
            0 => BrowserMediaPermission::Granted,
            1 => BrowserMediaPermission::Denied,
            _ => BrowserMediaPermission::NotDetermined,
        }
    }

    pub(super) fn request_authorization() -> BrowserMediaPermission {
        let current = authorization();
        if current != BrowserMediaPermission::NotDetermined {
            return current;
        }
        let Some(tcc) = tcc() else {
            return BrowserMediaPermission::Unavailable;
        };
        let (sender, answered) = mpsc::sync_channel::<()>(1);
        let completion = block2::RcBlock::new(move |_granted: objc2::runtime::Bool| {
            let _ = sender.try_send(());
        });
        let service = objc2_core_foundation::CFString::from_static_str("kTCCServiceAudioCapture");
        // TCC copies the completion block before returning.
        unsafe {
            (tcc.request)(
                (&*service as *const objc2_core_foundation::CFString).cast(),
                std::ptr::null(),
                block2::RcBlock::as_ptr(&completion).cast(),
            );
        }
        let _ = answered.recv_timeout(PERMISSION_DEADLINE);
        authorization()
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod native {
    use super::{BrowserMediaPermission, Job};

    pub(super) fn enqueue(_job: Job) {}

    pub(super) fn authorization() -> BrowserMediaPermission {
        BrowserMediaPermission::Unavailable
    }

    pub(super) fn request_authorization() -> BrowserMediaPermission {
        BrowserMediaPermission::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use super::*;

    struct Fake {
        in_call: bool,
        peaks: VecDeque<Option<f32>>,
        sent: Vec<MediaCommand>,
        send_ok: bool,
        listened: usize,
        output_closed: bool,
    }

    impl Fake {
        fn new(peaks: impl IntoIterator<Item = Option<f32>>) -> Self {
            Self {
                in_call: false,
                peaks: peaks.into_iter().collect(),
                sent: Vec::new(),
                send_ok: true,
                listened: 0,
                output_closed: false,
            }
        }
    }

    impl BrowserAudio for Fake {
        fn in_call(&mut self) -> bool {
            self.in_call
        }

        fn peak_dbfs(&mut self, _window: Duration) -> Option<f32> {
            self.listened += 1;
            self.peaks.pop_front().flatten()
        }

        fn send(&mut self, command: MediaCommand) -> bool {
            self.sent.push(command);
            self.send_ok
        }

        fn output_closed(&mut self) -> bool {
            self.output_closed
        }
    }

    /// The three listen windows of a browser that plays.
    fn playing() -> [Option<f32>; 3] {
        [Some(-20.0); 3]
    }

    #[test]
    fn a_silent_browser_is_left_alone() {
        let mut audio = Fake::new([Some(-120.0)]);
        assert_eq!(pause(&mut audio).unwrap_err(), PauseSkip::Silent);
        assert_eq!(audio.sent, []);
    }

    #[test]
    fn a_short_sound_is_not_a_playing_browser() {
        for listen in [
            [Some(-20.0), Some(-120.0), Some(-120.0)],
            [Some(-120.0), Some(-20.0), Some(-20.0)],
            [Some(-20.0), Some(-20.0), None],
        ] {
            let mut audio = Fake::new(listen.into_iter().chain([Some(-200.0); 5]));
            assert!(pause(&mut audio).is_err(), "{listen:?}");
            assert_eq!(audio.sent, [], "{listen:?}");
        }
    }

    #[test]
    fn a_call_is_never_paused() {
        let mut audio = Fake::new(playing());
        audio.in_call = true;
        assert_eq!(pause(&mut audio).unwrap_err(), PauseSkip::InCall);
        assert_eq!(audio.sent, []);
        assert_eq!(audio.listened, 0);
    }

    #[test]
    fn a_confirmed_pause_is_resumed() {
        let mut audio = Fake::new(playing().into_iter().chain([
            Some(-30.0),
            Some(-200.0),
            Some(-200.0),
            Some(-200.0),
            Some(-200.0),
            Some(-25.0),
        ]));
        let paused = pause(&mut audio).expect("the pause is confirmed");
        assert_eq!(resume(&mut audio, paused), ResumeOutcome::Resumed);
        assert_eq!(audio.sent, [MediaCommand::Pause, MediaCommand::Play]);
    }

    #[test]
    fn a_play_the_browser_never_takes_is_paused_again() {
        let mut audio = Fake::new(std::iter::repeat_n(Some(-200.0), 40));
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::Repaused
        );
        assert_eq!(audio.sent, [MediaCommand::Play, MediaCommand::Pause]);
    }

    #[test]
    fn a_browser_that_plays_late_in_the_check_keeps_playing() {
        let mut audio = Fake::new(std::iter::repeat_n(Some(-200.0), 19).chain([Some(-25.0)]));
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::Resumed
        );
        assert_eq!(audio.sent, [MediaCommand::Play]);
    }

    #[test]
    fn a_browser_it_stops_hearing_after_play_gets_nothing_more() {
        let mut audio = Fake::new([Some(-200.0), Some(-200.0)]);
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::LostAfterPlay
        );
        assert_eq!(audio.sent, [MediaCommand::Play]);
    }

    #[test]
    fn two_sounding_tabs_never_get_play() {
        let mut audio = Fake::new(
            playing()
                .into_iter()
                .chain(std::iter::repeat_n(Some(-25.0), 10)),
        );
        assert_eq!(pause(&mut audio).unwrap_err(), PauseSkip::Unconfirmed);
        assert_eq!(audio.sent, [MediaCommand::Pause]);
    }

    #[test]
    fn a_late_silence_is_not_a_confirmed_pause() {
        let mut audio = Fake::new(
            playing()
                .into_iter()
                .chain([Some(-25.0); 4])
                .chain([Some(-200.0); 6]),
        );
        assert!(pause(&mut audio).is_err());
        assert_eq!(audio.sent, [MediaCommand::Pause]);
    }

    #[test]
    fn one_quiet_window_is_not_a_confirmed_pause() {
        let mut audio = Fake::new(playing().into_iter().chain([
            Some(-200.0),
            Some(-25.0),
            Some(-200.0),
            Some(-25.0),
            Some(-25.0),
            Some(-200.0),
            Some(-200.0),
        ]));
        assert!(pause(&mut audio).is_err());
        assert_eq!(audio.sent, [MediaCommand::Pause]);
    }

    #[test]
    fn no_level_sends_nothing() {
        let mut audio = Fake::new([None]);
        assert_eq!(pause(&mut audio).unwrap_err(), PauseSkip::Unheard);
        assert_eq!(audio.sent, []);
    }

    #[test]
    fn a_browser_the_user_already_restarted_gets_no_play() {
        let mut audio = Fake::new(playing().into_iter().chain([Some(-200.0), Some(-200.0)]));
        let paused = pause(&mut audio).expect("the pause is confirmed");
        audio.peaks = VecDeque::from([Some(-20.0)]);
        assert_eq!(resume(&mut audio, paused), ResumeOutcome::AlreadyPlaying);
        assert_eq!(audio.sent, [MediaCommand::Pause]);
    }

    #[test]
    fn a_browser_it_cannot_hear_gets_no_play() {
        let mut audio = Fake::new([None]);
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::Unheard
        );
        assert_eq!(audio.sent, []);
    }

    #[test]
    fn a_browser_heard_before_play_that_goes_quiet_is_left_alone() {
        let mut audio = Fake::new([Some(-200.0), None]);
        audio.output_closed = true;
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::LostAfterPlay
        );
        assert_eq!(audio.sent, [MediaCommand::Play]);
        assert_eq!(audio.listened, 2);
    }

    #[test]
    fn a_browser_that_closed_its_output_gets_play() {
        // A pause longer than about 10 s: the browser closed its output, so
        // the tap delivers nothing until play opens it again.
        let mut audio = Fake::new([None, None, None, None, Some(-25.0)]);
        audio.output_closed = true;
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::Resumed
        );
        assert_eq!(audio.sent, [MediaCommand::Play]);
    }

    #[test]
    fn a_closed_browser_that_never_opens_after_play_is_paused_again() {
        let mut audio = Fake::new(std::iter::repeat_n(None, 40));
        audio.output_closed = true;
        assert_eq!(
            resume(&mut audio, PausedBrowser::for_test()),
            ResumeOutcome::Repaused
        );
        assert_eq!(audio.sent, [MediaCommand::Play, MediaCommand::Pause]);
        // A closed tab sends the play to another app: stop it after 1 s, not 2.
        assert_eq!(audio.listened, 1 + 10);
    }

    #[test]
    fn a_refused_command_is_not_confirmed() {
        let mut audio = Fake::new(playing().into_iter().chain([Some(-200.0); 5]));
        audio.send_ok = false;
        assert_eq!(pause(&mut audio).unwrap_err(), PauseSkip::Refused);
        assert_eq!(audio.sent, [MediaCommand::Pause]);
    }

    #[test]
    fn a_pause_the_user_takes_back_during_dictation_is_released() {
        // Silence, or confirmed closed outputs after a long pause, keep the proof.
        assert_eq!(watch(Some(-200.0), true, false), Watch::Keep);
        assert_eq!(watch(None, true, true), Watch::Keep);
        // A browser played again: a later pause is the user's, never Murmur's.
        assert_eq!(watch(Some(-20.0), true, false), Watch::Restarted);
        // No level without confirmed closed outputs: the tap may have stopped.
        assert_eq!(watch(None, true, false), Watch::Lost);
        // Murmur stopped hearing it: ownership is uncertain.
        assert_eq!(watch(None, false, true), Watch::Lost);
        assert_eq!(watch(Some(-200.0), false, false), Watch::Lost);
    }

    #[test]
    fn only_a_started_recording_pauses_and_only_its_end_resumes() {
        use DictationStatus::{Idle, Processing, Recording, Recovering, Starting};
        assert_eq!(
            job_for_transition(7, Starting, Recording),
            Some(Job::Pause(7))
        );
        // Capture recovery is the same recording, not a new pause.
        assert_eq!(job_for_transition(7, Recovering, Recording), None);
        assert_eq!(job_for_transition(7, Recording, Recovering), None);
        for from in [Starting, Recording, Recovering] {
            for to in [Idle, Processing] {
                assert_eq!(job_for_transition(7, from, to), Some(Job::Resume(7)));
            }
        }
        assert_eq!(job_for_transition(7, Idle, Starting), None);
        assert_eq!(job_for_transition(7, Processing, Idle), None);
        assert_eq!(job_for_transition(0, Starting, Recording), None);
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn worker_replies_are_strictly_parsed() {
        use native::{parse_peak, parse_ready};
        assert_eq!(parse_ready("ready 0 1"), Some((false, true)));
        assert_eq!(parse_ready("ready 1 0"), Some((true, false)));
        for bad in ["ready 2 0", "ready 0", "ready 0 1 1", "ready", "peak 0"] {
            assert_eq!(parse_ready(bad), None, "{bad}");
        }
        assert_eq!(parse_peak("peak -25.4"), Some(Some(-25.4)));
        assert_eq!(parse_peak("peak none"), Some(None));
        for bad in [
            "peak NaN",
            "peak inf",
            "peak 99",
            "peak",
            "closed 1",
            "peak -999",
        ] {
            assert_eq!(parse_peak(bad), None, "{bad}");
        }
    }
}
