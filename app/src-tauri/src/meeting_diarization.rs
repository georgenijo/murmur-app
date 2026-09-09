#[cfg(all(debug_assertions, target_os = "macos", target_arch = "aarch64"))]
#[path = "meeting_diarization_probe.rs"]
mod probe;

use crate::diarization_assignment::{self, SpeakerTurn};
use crate::diarization_audio::RemoteAudio;
use crate::managed_child::ManagedChild;
use crate::meeting_store::MeetingRepository;
use crate::{MutexExt, State};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{mpsc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const WORKER_ARGUMENT: &str = "--meeting-diarization-worker-v1";
const PREFIX: &str = "MRMR_DIARIZATION_V1 ";
const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;
const PREEMPT_DEADLINE: Duration = Duration::from_millis(150);
const PASS_DEADLINE: Duration = Duration::from_secs(120);
const JOB_DEADLINE: Duration = Duration::from_secs(10 * 60);
static CONTROL: LazyLock<Mutex<Control>> = LazyLock::new(|| Mutex::new(Control::default()));

#[derive(Default)]
struct Control {
    generation: u64,
    session_id: Option<String>,
    foreground_users: usize,
    interrupted: bool,
    child: Option<ManagedChild>,
}

fn release_confirmed_child(control: &mut Control) {
    control.child.take();
    crate::smart_auto_probe::wake();
}

fn stop_owned(control: &mut Control) -> Result<(), String> {
    if let Some(child) = control.child.as_mut() {
        if child
            .hard_kill_confirmed(Instant::now() + PREEMPT_DEADLINE)
            .is_none()
        {
            return Err(
                "Speaker refinement is still stopping. Try dictation again shortly.".into(),
            );
        }
        release_confirmed_child(control);
        control.interrupted = true;
    }
    Ok(())
}

struct JobOwner(u64);
impl Drop for JobOwner {
    fn drop(&mut self) {
        let mut control = CONTROL.lock_or_recover();
        if control.generation == self.0 {
            let _ = stop_owned(&mut control);
        }
    }
}

/// Call under recording_transition before a foreground capture begins.
pub fn preempt() -> Result<(), String> {
    stop_owned(&mut CONTROL.lock_or_recover())
}

pub(crate) fn is_active() -> bool {
    CONTROL.lock_or_recover().child.is_some()
}

pub fn cancel_session(session_id: &str) -> Result<(), String> {
    let mut control = CONTROL.lock_or_recover();
    if control.session_id.as_deref() == Some(session_id) {
        control.generation = control.generation.wrapping_add(1);
        stop_owned(&mut control)?;
    }
    Ok(())
}

pub fn cancel_all() -> Result<(), String> {
    let mut control = CONTROL.lock_or_recover();
    control.generation = control.generation.wrapping_add(1);
    stop_owned(&mut control)
}

/// Every ASR operation reserves its entire model lifetime against child startup.
pub struct ForegroundReservation;
impl ForegroundReservation {
    pub fn acquire() -> Result<Self, String> {
        let mut control = CONTROL.lock_or_recover();
        stop_owned(&mut control)?;
        control.foreground_users += 1;
        Ok(Self)
    }
}
impl Drop for ForegroundReservation {
    fn drop(&mut self) {
        CONTROL.lock_or_recover().foreground_users -= 1;
    }
}

fn app_idle(state: &State) -> bool {
    !state.app_state.meeting_blocks_asr()
        && !state.app_state.microphone_preview.is_active()
        && !crate::audio_lifecycle::is_audio_active()
        && !state.app_state.file_transcribing.load(Ordering::SeqCst)
        && !state.benchmark.is_running()
        && !state.app_state.transform_status().blocks_recording()
        && !state.transform_runtime.is_transform_busy()
        && state.app_state.dictation.lock_or_recover().status == crate::state::DictationStatus::Idle
}

pub fn schedule(
    app: tauri::AppHandle,
    repository: MeetingRepository,
    session_id: String,
    capture_generation: u64,
    audio: RemoteAudio,
) {
    let generation = {
        let mut control = CONTROL.lock_or_recover();
        if stop_owned(&mut control).is_err() {
            return;
        }
        control.generation = control.generation.wrapping_add(1);
        control.session_id = Some(session_id.clone());
        control.generation
    };
    let _ = std::thread::Builder::new()
        .name("murmur-meeting-speakers".into())
        .spawn(move || {
            let _owner = JobOwner(generation);
            let deadline = Instant::now() + JOB_DEADLINE;
            let mut attempts = 0;
            while Instant::now() < deadline && attempts < 8 {
                if CONTROL.lock_or_recover().generation != generation
                    || repository.get_session(&session_id).is_err()
                {
                    break;
                }
                let state = app.state::<State>();
                if state.app_state.meeting_generation.load(Ordering::SeqCst) != capture_generation {
                    break;
                }
                if !app_idle(&state) {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let Ok(model_use) = crate::diarization_model::MODEL_USE.try_lock() else {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                };
                let Ok(_cache) = crate::diarization_model::CacheLease::acquire(false) else {
                    break;
                };
                if !crate::diarization_model::installed() {
                    break;
                }
                let transition = state.app_state.recording_transition.blocking_lock();
                if !app_idle(&state) {
                    drop(transition);
                    drop(_cache);
                    drop(model_use);
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let mut control = CONTROL.lock_or_recover();
                if control.generation != generation {
                    break;
                }
                if control.foreground_users != 0 || control.child.is_some() {
                    drop(control);
                    drop(_cache);
                    drop(model_use);
                    drop(transition);
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let Ok(executable) = std::env::current_exe() else {
                    break;
                };
                let Ok((child, mut input, output)) =
                    ManagedChild::spawn_with_arguments(&executable, &[WORKER_ARGUMENT], &[])
                else {
                    break;
                };
                control.child = Some(child);
                control.interrupted = false;
                drop(control);
                drop(transition);
                attempts += 1;
                let request = WorkerRequest {
                    audio_path: audio.path().to_path_buf(),
                    duration_ms: audio.duration_ms(),
                };
                let wrote = serde_json::to_writer(&mut input, &request).is_ok()
                    && input.write_all(b"\n").is_ok()
                    && input.flush().is_ok();
                let (sender, receiver) = mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let mut bytes = Vec::new();
                    let result = output.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut bytes);
                    let _ = sender.send(
                        if result.is_ok() && bytes.len() as u64 <= MAX_OUTPUT_BYTES {
                            Some(bytes)
                        } else {
                            None
                        },
                    );
                });
                let pass_deadline = Instant::now() + PASS_DEADLINE;
                let mut bytes = None;
                while wrote && Instant::now() < pass_deadline && Instant::now() < deadline {
                    match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(result) => {
                            bytes = result;
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let control = CONTROL.lock_or_recover();
                    if control.generation != generation || control.interrupted {
                        break;
                    }
                }
                drop(input);
                let mut control = CONTROL.lock_or_recover();
                let interrupted = control.interrupted;
                if control.generation != generation {
                    break;
                }
                let exited = control.child.as_mut().and_then(|child| {
                    child.wait_for_exit(Instant::now() + Duration::from_millis(10))
                });
                let success = exited.is_some_and(|receipt| receipt.exit_code == Some(0));
                if exited.is_some() {
                    release_confirmed_child(&mut control);
                }
                if stop_owned(&mut control).is_err() {
                    break;
                }
                drop(control);
                drop(_cache);
                drop(model_use);
                if interrupted {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                if !success {
                    break;
                }
                let Some(turns) = bytes.and_then(|bytes| parse_output(&bytes, audio.duration_ms()))
                else {
                    break;
                };
                let Ok(detail) = repository.detail(&session_id) else {
                    break;
                };
                let assignments =
                    diarization_assignment::assignments(&detail.segments, &turns, audio.origin_ms);
                // Accept this complete pass before the transaction. The repository checks
                // exact session ownership; deletion either precedes or follows its commit.
                let current = CONTROL.lock_or_recover().generation == generation;
                if current
                    && repository
                        .apply_remote_speakers(&session_id, &assignments)
                        .is_ok()
                {
                    let _ = app.emit(
                        "meeting-speakers-updated",
                        serde_json::json!({"sessionId":session_id}),
                    );
                }
                break;
            }
            // The owned file and all inference turns die here, including every failure path.
        });
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WorkerRequest {
    audio_path: std::path::PathBuf,
    duration_ms: u64,
}

fn parse_output(bytes: &[u8], duration_ms: u64) -> Option<Vec<SpeakerTurn>> {
    let output = std::str::from_utf8(bytes).ok()?;
    let mut lines = output.lines().filter_map(|line| line.strip_prefix(PREFIX));
    let turns: Vec<SpeakerTurn> = serde_json::from_str(lines.next()?).ok()?;
    if lines.next().is_some() || !diarization_assignment::valid_turns(&turns, duration_ms) {
        return None;
    }
    Some(turns)
}

#[cfg(any(target_os = "macos", test))]
fn worker_profile(root: &std::path::Path) -> Option<std::ffi::CString> {
    let path = root.to_str()?;
    if path.chars().any(char::is_control) {
        return None;
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    std::ffi::CString::new(format!(
        "(version 1) (allow default) (deny network*) (deny file-write* (subpath \"{escaped}\"))"
    ))
    .ok()
}

#[cfg(target_os = "macos")]
fn restrict_worker_io() -> bool {
    unsafe extern "C" {
        fn sandbox_init(
            profile: *const std::ffi::c_char,
            flags: u64,
            error: *mut *mut std::ffi::c_char,
        ) -> i32;
        fn sandbox_free_error(error: *mut std::ffi::c_char);
    }
    let Some(profile) = crate::diarization_model::directory()
        .and_then(|root| root.canonicalize().ok())
        .and_then(|root| worker_profile(&root))
    else {
        return false;
    };
    let mut error = std::ptr::null_mut();
    let result = unsafe { sandbox_init(profile.as_ptr(), 0, &mut error) };
    if !error.is_null() {
        unsafe {
            sandbox_free_error(error);
        }
    }
    result == 0
}
#[cfg(not(target_os = "macos"))]
fn restrict_worker_io() -> bool {
    false
}

pub fn run_cli_if_requested() -> Option<i32> {
    #[cfg(all(debug_assertions, target_os = "macos", target_arch = "aarch64"))]
    if let Some(exit) = probe::run_cli_if_requested() {
        return Some(exit);
    }
    let mut arguments = std::env::args_os().skip(1);
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_ARGUMENT)) {
        return None;
    }
    if arguments.next().is_some() {
        return Some(64);
    }
    #[cfg(unix)]
    if unsafe { libc::getpgrp() != libc::getpid() } {
        return Some(64);
    }
    let mut reader = std::io::BufReader::new(std::io::stdin());
    let mut encoded = String::new();
    if std::io::Read::by_ref(&mut reader)
        .take(4096)
        .read_line(&mut encoded)
        .is_err()
        || !encoded.ends_with('\n')
    {
        return Some(64);
    }
    let Ok(request) = serde_json::from_str::<WorkerRequest>(&encoded) else {
        return Some(64);
    };
    if request.duration_ms == 0
        || request.duration_ms > crate::diarization_audio::MAX_AUDIO_SECONDS * 1000
        || !request.audio_path.is_absolute()
    {
        return Some(64);
    }
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        loop {
            if !matches!(reader.read(&mut byte), Ok(1)) {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-libc::getpgrp(), libc::SIGKILL);
                }
                std::process::exit(70);
            }
        }
    });
    let Ok(_cache) = crate::diarization_model::CacheLease::acquire(false) else {
        return Some(65);
    };
    if !restrict_worker_io() || !crate::diarization_model::installed() {
        return Some(65);
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let result = (|| {
            let engine = fluidaudio_rs::FluidAudio::new().ok()?;
            engine.init_diarization(0.7).ok()?;
            println!("MRMR_DIARIZATION_RUNNING_V1");
            std::io::stdout().flush().ok()?;
            let result = engine.diarize_file(&request.audio_path).ok()?;
            if result.len() > diarization_assignment::MAX_TURNS {
                return None;
            }
            let mut identities = std::collections::HashMap::new();
            let mut turns = Vec::with_capacity(result.len());
            for turn in result {
                if !turn.start_time.is_finite()
                    || !turn.end_time.is_finite()
                    || turn.start_time < 0.0
                    || turn.end_time <= turn.start_time
                {
                    return None;
                }
                let next = identities.len() as u32;
                let speaker = *identities.entry(turn.speaker_id).or_insert(next);
                turns.push(SpeakerTurn {
                    speaker,
                    start_ms: (turn.start_time * 1000.0).round() as u64,
                    end_ms: (turn.end_time * 1000.0).round() as u64,
                    quality: f64::from(turn.quality_score),
                });
            }
            if !diarization_assignment::valid_turns(&turns, request.duration_ms) {
                return None;
            }
            let encoded = serde_json::to_string(&turns).ok()?;
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{PREFIX}{encoded}").ok()?;
            stdout.flush().ok()?;
            Some(())
        })();
        Some(if result.is_some() { 0 } else { 66 })
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Some(65)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn natural_success_and_failure_both_wake_parked_input_checks() {
        for executable in ["/usr/bin/true", "/usr/bin/false"] {
            let (child, input, output) =
                ManagedChild::spawn_with_arguments(std::path::Path::new(executable), &[], &[])
                    .unwrap();
            let mut control = Control {
                child: Some(child),
                ..Control::default()
            };
            assert!(control
                .child
                .as_mut()
                .unwrap()
                .wait_for_exit(Instant::now() + Duration::from_secs(2))
                .is_some());
            let _ =
                tokio::time::timeout(Duration::ZERO, crate::smart_auto_probe::notified_for_test())
                    .await;
            release_confirmed_child(&mut control);
            assert!(control.child.is_none());
            assert!(tokio::time::timeout(
                Duration::from_millis(100),
                crate::smart_auto_probe::notified_for_test()
            )
            .await
            .is_ok());
            drop((input, output));
        }
    }
    #[test]
    fn protocol_rejects_duplicates_unknown_fields_and_out_of_range_turns() {
        let line =
            format!("{PREFIX}[{{\"speaker\":0,\"startMs\":0,\"endMs\":1000,\"quality\":0.8}}]\n");
        assert!(parse_output(line.as_bytes(), 1000).is_some());
        assert!(parse_output(format!("{line}{line}").as_bytes(), 1000).is_none());
        assert!(parse_output(line.as_bytes(), 100).is_none());
        assert!(parse_output(
            line.replace("\"quality\":", "\"embedding\":[],\"quality\":")
                .as_bytes(),
            1000
        )
        .is_none());
    }
    #[cfg(unix)]
    #[test]
    fn foreground_preemption_confirms_exact_child_exit_before_releasing_owner() {
        let (child, input, output) =
            ManagedChild::spawn_with_arguments(std::path::Path::new("/bin/sleep"), &["30"], &[])
                .unwrap();
        let pid = child.pid();
        let mut control = Control {
            child: Some(child),
            ..Control::default()
        };
        let started = Instant::now();
        stop_owned(&mut control).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(control.child.is_none());
        assert!(control.interrupted);
        assert_eq!(unsafe { libc::kill(-(pid as i32), 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        drop((input, output));
    }
    #[test]
    fn foreground_reservation_prevents_new_worker_ownership() {
        let before = CONTROL.lock_or_recover().foreground_users;
        {
            let _guard = ForegroundReservation::acquire().unwrap();
            assert_eq!(CONTROL.lock_or_recover().foreground_users, before + 1);
        }
        assert_eq!(CONTROL.lock_or_recover().foreground_users, before);
    }
}
