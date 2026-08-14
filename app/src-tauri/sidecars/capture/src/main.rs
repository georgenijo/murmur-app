#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod production;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod system_audio;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod supported {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, Stream, StreamConfig};
    use murmur_capture_helper_protocol::{
        read_frame, valid_host_message, write_frame, CapturePhase, FailureCode, HelperMessage,
        HostMessage, PROTOCOL_NAME, PROTOCOL_VERSION, SYNTHETIC_FIXTURE, SYNTHETIC_FIXTURE_CHUNKS,
        SYNTHETIC_FIXTURE_DIGEST,
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    const FIRST_CALLBACK_DEADLINE: Duration = Duration::from_secs(3);
    const CALLBACK_STALL_DEADLINE: Duration = Duration::from_secs(1);

    fn message(nonce: &str, phase: CapturePhase) -> HelperMessage {
        HelperMessage::Phase {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.to_string(),
            phase,
        }
    }

    fn failure(nonce: &str, code: FailureCode) -> HelperMessage {
        HelperMessage::Failure {
            protocol: PROTOCOL_NAME.to_string(),
            version: PROTOCOL_VERSION,
            session_nonce: nonce.to_string(),
            code,
        }
    }

    fn callback_bucket(count: u64) -> &'static str {
        match count {
            0 => "0",
            1..=10 => "le10",
            11..=100 => "le100",
            101..=1_000 => "le1k",
            _ => "gt1k",
        }
    }

    struct CallbackState {
        count: AtomicU64,
        stream_error: AtomicBool,
    }

    fn build_stream(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        state: &Arc<CallbackState>,
    ) -> Result<Stream, FailureCode> {
        let stream_config: StreamConfig = (*config).into();
        macro_rules! build {
            ($sample:ty) => {{
                let callback_state = Arc::clone(state);
                let error_state = Arc::clone(state);
                device.build_input_stream(
                    stream_config,
                    move |data: &[$sample], _| {
                        // Real-time boundary: atomics only. PCM is neither copied,
                        // retained, logged, serialized, nor written to disk.
                        let _ = data;
                        callback_state.count.fetch_add(1, Ordering::Relaxed);
                    },
                    move |_| {
                        error_state.stream_error.store(true, Ordering::Release);
                    },
                    None,
                )
            }};
        }

        let result = match config.sample_format() {
            SampleFormat::I8 => build!(i8),
            SampleFormat::I16 => build!(i16),
            SampleFormat::I32 => build!(i32),
            SampleFormat::I64 => build!(i64),
            SampleFormat::U8 => build!(u8),
            SampleFormat::U16 => build!(u16),
            SampleFormat::U32 => build!(u32),
            SampleFormat::U64 => build!(u64),
            SampleFormat::F32 => build!(f32),
            SampleFormat::F64 => build!(f64),
            _ => return Err(FailureCode::ConfigurationFailed),
        };
        result.map_err(|_| FailureCode::StreamOpenFailed)
    }

    fn disable_core_dumps() {
        unsafe {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &limit);
        }
    }

    fn establish_process_group() -> Result<(), ()> {
        let pid = unsafe { libc::getpid() };
        if unsafe { libc::setpgid(0, 0) } != 0 && unsafe { libc::getpgrp() } != pid {
            return Err(());
        }
        if unsafe { libc::getpgrp() } != pid {
            return Err(());
        }
        Ok(())
    }

    fn wait_for_cancel(
        host_rx: &mpsc::Receiver<HostMessage>,
        stdout: &mut impl std::io::Write,
        nonce: &str,
    ) -> Result<(), ()> {
        match host_rx.recv() {
            Ok(HostMessage::Cancel {
                protocol,
                version,
                session_nonce,
            }) if protocol == PROTOCOL_NAME
                && version == PROTOCOL_VERSION
                && session_nonce == nonce =>
            {
                write_frame(stdout, &message(nonce, CapturePhase::Stopping)).map_err(|_| ())?;
                write_frame(
                    stdout,
                    &HelperMessage::Stopped {
                        protocol: PROTOCOL_NAME.to_string(),
                        version: PROTOCOL_VERSION,
                        session_nonce: nonce.to_string(),
                    },
                )
                .map_err(|_| ())
            }
            _ => {
                write_frame(stdout, &failure(nonce, FailureCode::InvalidMessage))
                    .map_err(|_| ())?;
                Err(())
            }
        }
    }

    fn run_synthetic(
        host_rx: &mpsc::Receiver<HostMessage>,
        stdout: &mut impl std::io::Write,
        nonce: &str,
        ignore_cancel: bool,
    ) -> Result<(), ()> {
        for phase in [CapturePhase::Enumeration, CapturePhase::StreamOpen] {
            write_frame(stdout, &message(nonce, phase)).map_err(|_| ())?;
        }
        write_frame(
            stdout,
            &HelperMessage::Ready {
                protocol: PROTOCOL_NAME.to_string(),
                version: PROTOCOL_VERSION,
                session_nonce: nonce.to_string(),
            },
        )
        .map_err(|_| ())?;
        write_frame(stdout, &message(nonce, CapturePhase::AwaitingFirstCallback))
            .map_err(|_| ())?;
        write_frame(
            stdout,
            &HelperMessage::FirstCallback {
                protocol: PROTOCOL_NAME.to_string(),
                version: PROTOCOL_VERSION,
                session_nonce: nonce.to_string(),
                callback_latency_ms: 0,
            },
        )
        .map_err(|_| ())?;
        write_frame(stdout, &message(nonce, CapturePhase::Active)).map_err(|_| ())?;
        for sequence in 0..SYNTHETIC_FIXTURE_CHUNKS {
            write_frame(
                stdout,
                &HelperMessage::SyntheticChunk {
                    protocol: PROTOCOL_NAME.to_string(),
                    version: PROTOCOL_VERSION,
                    session_nonce: nonce.to_string(),
                    fixture: SYNTHETIC_FIXTURE.to_string(),
                    fixture_digest: SYNTHETIC_FIXTURE_DIGEST.to_string(),
                    sequence,
                },
            )
            .map_err(|_| ())?;
        }
        if ignore_cancel {
            loop {
                std::thread::park_timeout(Duration::from_secs(60));
            }
        }
        wait_for_cancel(host_rx, stdout, nonce)
    }

    pub fn run(synthetic: bool, ignore_cancel: bool) -> Result<(), ()> {
        disable_core_dumps();
        // The worker establishes its own group before reading the host hello or
        // touching CoreAudio. A parent-side setpgid after Process.run() races
        // with exec and is permitted to fail with EACCES on macOS.
        establish_process_group()?;
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout().lock();
        let hello = read_frame::<HostMessage>(&mut stdin).map_err(|_| ())?;
        if !valid_host_message(&hello) {
            return Err(());
        }
        let nonce = match hello {
            HostMessage::Hello { session_nonce, .. } => session_nonce,
            HostMessage::Cancel { .. } => return Err(()),
        };
        drop(stdin);

        let (host_tx, host_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin().lock();
            while let Ok(frame) = read_frame::<HostMessage>(&mut stdin) {
                if host_tx.send(frame).is_err() {
                    return;
                }
            }
            // The agent owns this worker through the stdin pipe. If the agent is
            // killed while CoreAudio is blocked, the reader thread is still able
            // to terminate the whole worker immediately on EOF.
            unsafe { libc::_exit(0) }
        });

        if synthetic {
            return run_synthetic(&host_rx, &mut stdout, &nonce, ignore_cancel);
        }

        write_frame(&mut stdout, &message(&nonce, CapturePhase::Enumeration)).map_err(|_| ())?;
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(device) => device,
            None => {
                write_frame(&mut stdout, &failure(&nonce, FailureCode::NoInputDevice))
                    .map_err(|_| ())?;
                return Ok(());
            }
        };

        write_frame(&mut stdout, &message(&nonce, CapturePhase::StreamOpen)).map_err(|_| ())?;
        let config = match device.default_input_config() {
            Ok(config) => config,
            Err(_) => {
                write_frame(
                    &mut stdout,
                    &failure(&nonce, FailureCode::ConfigurationFailed),
                )
                .map_err(|_| ())?;
                return Ok(());
            }
        };
        let callback_state = Arc::new(CallbackState {
            count: AtomicU64::new(0),
            stream_error: AtomicBool::new(false),
        });
        let started = Instant::now();
        let stream = match build_stream(&device, &config, &callback_state) {
            Ok(stream) => stream,
            Err(code) => {
                write_frame(&mut stdout, &failure(&nonce, code)).map_err(|_| ())?;
                return Ok(());
            }
        };
        if stream.play().is_err() {
            write_frame(
                &mut stdout,
                &failure(&nonce, FailureCode::StreamStartFailed),
            )
            .map_err(|_| ())?;
            return Ok(());
        }

        write_frame(
            &mut stdout,
            &HelperMessage::Ready {
                protocol: PROTOCOL_NAME.to_string(),
                version: PROTOCOL_VERSION,
                session_nonce: nonce.clone(),
            },
        )
        .map_err(|_| ())?;
        write_frame(
            &mut stdout,
            &message(&nonce, CapturePhase::AwaitingFirstCallback),
        )
        .map_err(|_| ())?;

        let mut first_seen = false;
        let mut last_count = 0_u64;
        let mut last_callback_progress = started;
        let mut last_health = Instant::now();
        loop {
            match host_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(HostMessage::Cancel {
                    protocol,
                    version,
                    session_nonce,
                }) if protocol == PROTOCOL_NAME
                    && version == PROTOCOL_VERSION
                    && session_nonce == nonce =>
                {
                    write_frame(&mut stdout, &message(&nonce, CapturePhase::Stopping))
                        .map_err(|_| ())?;
                    let _ = stream.pause();
                    write_frame(
                        &mut stdout,
                        &HelperMessage::Stopped {
                            protocol: PROTOCOL_NAME.to_string(),
                            version: PROTOCOL_VERSION,
                            session_nonce: nonce,
                        },
                    )
                    .map_err(|_| ())?;
                    return Ok(());
                }
                Ok(_) => {
                    write_frame(&mut stdout, &failure(&nonce, FailureCode::InvalidMessage))
                        .map_err(|_| ())?;
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            if callback_state.stream_error.load(Ordering::Acquire) {
                write_frame(&mut stdout, &failure(&nonce, FailureCode::StreamError))
                    .map_err(|_| ())?;
                return Ok(());
            }
            let count = callback_state.count.load(Ordering::Acquire);
            if count != last_count {
                last_count = count;
                last_callback_progress = Instant::now();
            }
            if !first_seen && count > 0 {
                first_seen = true;
                write_frame(
                    &mut stdout,
                    &HelperMessage::FirstCallback {
                        protocol: PROTOCOL_NAME.to_string(),
                        version: PROTOCOL_VERSION,
                        session_nonce: nonce.clone(),
                        callback_latency_ms: started.elapsed().as_millis() as u64,
                    },
                )
                .map_err(|_| ())?;
                write_frame(&mut stdout, &message(&nonce, CapturePhase::Active)).map_err(|_| ())?;
            } else if !first_seen && started.elapsed() >= FIRST_CALLBACK_DEADLINE {
                write_frame(&mut stdout, &failure(&nonce, FailureCode::CallbackStalled))
                    .map_err(|_| ())?;
                return Ok(());
            } else if first_seen && last_callback_progress.elapsed() >= CALLBACK_STALL_DEADLINE {
                write_frame(&mut stdout, &failure(&nonce, FailureCode::CallbackStalled))
                    .map_err(|_| ())?;
                return Ok(());
            }

            if first_seen && last_health.elapsed() >= Duration::from_secs(1) {
                write_frame(
                    &mut stdout,
                    &HelperMessage::CallbackHealth {
                        protocol: PROTOCOL_NAME.to_string(),
                        version: PROTOCOL_VERSION,
                        session_nonce: nonce.clone(),
                        callback_count_bucket: callback_bucket(count).to_string(),
                    },
                )
                .map_err(|_| ())?;
                last_health = Instant::now();
            }
        }
    }
}

fn main() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        if arguments.first().map(String::as_str) == Some("--production-v5") {
            if production::run(&arguments[1..]).is_ok() {
                return;
            }
            std::process::exit(70);
        }
        let (synthetic, ignore_cancel) = match arguments.as_slice() {
            [] => (false, false),
            [flag, fixture]
                if flag == "--synthetic-fixture"
                    && fixture == murmur_capture_helper_protocol::SYNTHETIC_FIXTURE =>
            {
                (true, false)
            }
            [flag, fixture, fault]
                if flag == "--synthetic-fixture"
                    && fixture == murmur_capture_helper_protocol::SYNTHETIC_FIXTURE
                    && fault == "--ignore-cancel" =>
            {
                (true, true)
            }
            _ => std::process::exit(64),
        };
        if supported::run(synthetic, ignore_cancel).is_ok() {
            return;
        }
    }
    std::process::exit(70);
}
