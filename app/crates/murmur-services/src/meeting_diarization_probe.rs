//! Debug-only hardware proof. Uses the same worker, ASR runtime and priority owner as the app.
use super::*;

const ARGUMENT: &str = "--meeting-diarization-preemption-check-v1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    audio_path: std::path::PathBuf,
    duration_ms: u64,
    asr_audio_path: std::path::PathBuf,
}

fn run(request: Request) -> Option<serde_json::Value> {
    use crate::model_runtime::{ModelRuntimeManager, PreparationReason};
    use crate::transcriber::COREML_MODEL_NAME;
    if !crate::diarization_model::installed()
        || !crate::model_runtime::model_installed(COREML_MODEL_NAME)
        || request.duration_ms == 0
        || request.duration_ms > crate::diarization_audio::MAX_AUDIO_SECONDS * 1000
        || !request.audio_path.is_absolute()
        || !request.asr_audio_path.is_absolute()
    {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&request.asr_audio_path)
        .ok()?
        .take(1_000_001)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 1_000_000 {
        return None;
    }
    let samples = crate::transcriber::parse_wav_to_samples(&bytes).ok()?;
    let runtime = ModelRuntimeManager::default();
    let decode = || {
        let started = Instant::now();
        let (text, _) = runtime
            .with_ready_backend(
                None,
                COREML_MODEL_NAME,
                PreparationReason::Recording,
                |backend| backend.transcribe(&samples, "en", None, true),
            )
            .ok()?;
        Some((text, started.elapsed().as_secs_f64() * 1000.0))
    };
    let (reference, _) = decode()?;
    if reference.is_empty() {
        return None;
    }
    let generation = {
        let mut control = CONTROL.lock_or_recover();
        stop_owned(&mut control).ok()?;
        control.generation = control.generation.wrapping_add(1);
        control.generation
    };
    let _owner = JobOwner(generation);
    let mut trials = Vec::new();
    for _ in 0..3 {
        let (baseline_text, baseline_ms) = decode()?;
        if baseline_text != reference {
            return None;
        }
        let baseline_rss = crate::resource_monitor::get_process_rss_bytes();
        let executable = std::env::current_exe().ok()?;
        let (child, mut input, output) =
            ManagedChild::spawn_with_arguments(&executable, &[WORKER_ARGUMENT], &[]).ok()?;
        let pid = child.pid();
        CONTROL.lock_or_recover().child = Some(child);
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(output).take(MAX_OUTPUT_BYTES);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|count| count > 0) {
                if line.trim() == "MRMR_DIARIZATION_RUNNING_V1" {
                    let _ = sender.try_send(());
                }
                line.clear();
            }
        });
        serde_json::to_writer(
            &mut input,
            &WorkerRequest {
                audio_path: request.audio_path.clone(),
                duration_ms: request.duration_ms,
            },
        )
        .ok()?;
        input.write_all(b"\n").ok()?;
        input.flush().ok()?;
        receiver.recv_timeout(PASS_DEADLINE).ok()?;
        // Let the announced native pass enter inference before taking dictation priority.
        std::thread::sleep(Duration::from_millis(40));
        let mut system = sysinfo::System::new();
        let target = sysinfo::Pid::from_u32(pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
        let child_rss = system.process(target)?.memory();
        if CONTROL
            .lock_or_recover()
            .child
            .as_mut()?
            .try_wait()
            .ok()?
            .is_some()
        {
            return None;
        }
        let started = Instant::now();
        let reservation = ForegroundReservation::acquire().ok()?;
        let cancellation_ms = started.elapsed().as_secs_f64() * 1000.0;
        let (text, decode_ms) = decode()?;
        let foreground_ms = started.elapsed().as_secs_f64() * 1000.0;
        drop(reservation);
        drop(input);
        if text != reference || CONTROL.lock_or_recover().child.is_some() {
            return None;
        }
        if unsafe { libc::kill(-(pid as i32), 0) } == 0 {
            return None;
        }
        trials.push(serde_json::json!({
            "baselineDecodeMs":baseline_ms, "cancellationMs":cancellation_ms,
            "priorityDecodeMs":decode_ms,"totalForegroundMs":foreground_ms,
            "workerRssBeforeCancellationBytes":child_rss,
            "mainRssBeforeWorkerBytes":baseline_rss,
            "mainRssBytes":crate::resource_monitor::get_process_rss_bytes(),
            "textUnchanged":true,"terminationConfirmed":true
        }));
    }
    Some(
        serde_json::json!({"schema":"murmur.diarization-preemption-check.v1","model":COREML_MODEL_NAME,"trials":trials}),
    )
}

pub(super) fn run_cli_if_requested() -> Option<i32> {
    let mut arguments = std::env::args_os().skip(1);
    let argument = arguments.next()?;
    let seed_mode = argument == "--meeting-diarization-seed-fixture-v1";
    if argument != ARGUMENT && !seed_mode {
        return None;
    }
    if arguments.next().is_some() {
        return Some(64);
    }
    let mut input = std::io::BufReader::new(std::io::stdin()).take(4096);
    let mut encoded = String::new();
    if input.read_line(&mut encoded).is_err() || !encoded.ends_with('\n') {
        return Some(64);
    }
    if seed_mode {
        let Ok(request) = serde_json::from_str(&encoded) else {
            return Some(64);
        };
        let Some(result) = seed(request) else {
            return Some(66);
        };
        println!("MRMR_DIARIZATION_GUI_FIXTURE_V1 {result}");
        return Some(0);
    }
    let Ok(request) = serde_json::from_str(&encoded) else {
        return Some(64);
    };
    let Some(result) = run(request) else {
        return Some(66);
    };
    println!("MRMR_DIARIZATION_PRIORITY_V1 {result}");
    Some(0)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeedRequest {
    audio_path: std::path::PathBuf,
    store_root: std::path::PathBuf,
}

struct SeedDirectory(std::path::PathBuf);
impl Drop for SeedDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn seed(request: SeedRequest) -> Option<serde_json::Value> {
    use crate::meeting_store::{MeetingSessionStatus, MeetingSpeaker};
    use sha2::{Digest, Sha256};
    const FIXTURE_HASH: &str = "3e2560b19bee6952c7c7ce041b0f1ea8a7ea9468044c4eea79d2a2c67e24ab0f";
    const DURATION: u64 = 1_049_355;
    let expected = dirs::data_dir()?.join("com.localdictation.diarization540/meetings");
    if request.store_root != expected || !request.audio_path.is_absolute() {
        return None;
    }
    if expected.exists()
        && (!std::fs::symlink_metadata(&expected).ok()?.is_dir()
            || std::fs::read_dir(&expected).ok()?.next().is_some())
    {
        return None;
    }
    let parent = expected.parent()?;
    if std::fs::symlink_metadata(parent).is_ok_and(|metadata| !metadata.is_dir()) {
        return None;
    }
    let mut fixture = std::fs::File::open(&request.audio_path).ok()?;
    if fixture.metadata().ok()?.len() != 33_579_394 {
        return None;
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = fixture.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != FIXTURE_HASH || !crate::diarization_model::installed()
    {
        return None;
    }
    let generation = {
        let mut control = CONTROL.lock_or_recover();
        stop_owned(&mut control).ok()?;
        control.generation = control.generation.wrapping_add(1);
        control.generation
    };
    let _owner = JobOwner(generation);
    let executable = std::env::current_exe().ok()?;
    let (child, mut input, output) =
        ManagedChild::spawn_with_arguments(&executable, &[WORKER_ARGUMENT], &[]).ok()?;
    CONTROL.lock_or_recover().child = Some(child);
    serde_json::to_writer(
        &mut input,
        &WorkerRequest {
            audio_path: request.audio_path,
            duration_ms: DURATION,
        },
    )
    .ok()?;
    input.write_all(b"\n").ok()?;
    input.flush().ok()?;
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
    let bytes = receiver.recv_timeout(PASS_DEADLINE).ok()??;
    let receipt = CONTROL
        .lock_or_recover()
        .child
        .as_mut()?
        .wait_for_exit(Instant::now() + Duration::from_secs(1))?;
    if receipt.exit_code != Some(0) {
        return None;
    }
    CONTROL.lock_or_recover().child.take();
    drop(input);
    let turns = parse_output(&bytes, DURATION)?;
    std::fs::create_dir_all(parent).ok()?;
    let staging = SeedDirectory(parent.join(format!(".fixture-{}", uuid::Uuid::new_v4())));
    let (repository, _) = MeetingRepository::initialize(staging.0.clone()).ok()?;
    let mut sessions = Vec::new();
    for number in 1..=2 {
        let id = format!("fixture-ami-{}", uuid::Uuid::new_v4());
        repository
            .create_session(
                &id,
                crate::transcriber::COREML_MODEL_NAME,
                "en",
                true,
                false,
            )
            .ok()?;
        for sequence in 0..70 {
            let start = sequence * 15_000;
            let end = (start + 15_000).min(DURATION);
            for speaker in [MeetingSpeaker::Me, MeetingSpeaker::Them] {
                let label = if speaker == MeetingSpeaker::Me {
                    "microphone"
                } else {
                    "remote"
                };
                let segment = repository
                    .insert_pending_segment(
                        &id,
                        speaker,
                        sequence,
                        start,
                        end,
                        &format!("audio/{id}/{label}-{sequence}.wav"),
                    )
                    .ok()?;
                repository.finalize_segment(segment,&format!("Fixture meeting {number}, {label} passage {}. This is a label verification marker, not recognized speech.",sequence+1),false).ok()?;
            }
        }
        repository
            .finish_session(&id, MeetingSessionStatus::Complete, None)
            .ok()?;
        let detail = repository.detail(&id).ok()?;
        let assigned = diarization_assignment::assignments(&detail.segments, &turns, 0);
        repository.apply_remote_speakers(&id, &assigned).ok()?;
        let workspace = repository.workspace(&id).ok()?;
        if workspace.remote_speakers.len() != 4 || assigned.is_empty() {
            return None;
        }
        sessions.push(serde_json::json!({"sessionId":id,"assignedSegments":assigned.len(),"speakerCount":workspace.remote_speakers.len()}));
    }
    // This newly-created fixture database has no user rows. Set a representative timeline.
    let connection = rusqlite::Connection::open(staging.0.join("meetings.sqlite3")).ok()?;
    connection
        .execute(
            "UPDATE meeting_sessions SET ended_at_ms=started_at_ms+?",
            [DURATION as i64],
        )
        .ok()?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .ok()?;
    drop(connection);
    if expected.exists() {
        std::fs::remove_dir(&expected).ok()?;
    }
    std::fs::rename(&staging.0, &expected).ok()?;
    Some(
        serde_json::json!({"schema":"murmur.diarization-gui-fixture.v1","fixtureSha256":FIXTURE_HASH,"sessions":sessions,"syntheticText":true,"labelsFromNativeWorker":true}),
    )
}
