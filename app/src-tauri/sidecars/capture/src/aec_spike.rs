//! Consent-gated, local-only acoustic echo-cancellation feasibility tooling.
//!
//! This module is compiled only with the private `aec-spike` feature. It is
//! deliberately not reachable from normal helper arguments or release builds.

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use webrtc_audio_processing::{config::EchoCanceller, Config, Processor};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 480;
const CONSENT: &str = "I_UNDERSTAND_THIS_WRITES_LOCAL_AUDIO";
const MIN_RENDER_ENERGY: f64 = 1e-8;
const BUILD_SHA: &str = match option_env!("MURMUR_AEC_SPIKE_BUILD_SHA") {
    Some(value) => value,
    None => "local-unidentified",
};

#[derive(Debug)]
struct Arguments {
    render: PathBuf,
    microphone: PathBuf,
    timing: Option<PathBuf>,
    output: PathBuf,
    report: PathBuf,
}

#[derive(Debug)]
struct CaptureArguments {
    output_root: PathBuf,
    duration: Duration,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureManifest {
    schema_version: u32,
    contains_real_user_data: bool,
    local_only: bool,
    network_used: bool,
    consent_acknowledgment: &'static str,
    captured_at_unix_seconds: u64,
    tool_build_sha: &'static str,
    source: &'static str,
    device_class: &'static str,
    render: CapturedFile,
    microphone: CapturedFile,
    timing: CapturedFile,
    timing_kind: &'static str,
    deletion: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedFile {
    file_name: &'static str,
    sha256: String,
    sample_rate_hz: Option<u32>,
    channels: Option<u16>,
    samples: Option<u64>,
    bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    local_only: bool,
    network_used: bool,
    sample_rate_hz: u32,
    frame_samples: usize,
    render_sha256: String,
    microphone_sha256: String,
    timing_sha256: Option<String>,
    cleaned_sha256: String,
    render_samples: usize,
    microphone_samples: usize,
    cleaned_samples: usize,
    measured_frames: usize,
    erle_db_global: Option<f64>,
    erle_db_median: Option<f64>,
    erle_db_p10: Option<f64>,
    processor_delay_ms_p50: Option<u32>,
    processor_delay_ms_p95: Option<u32>,
    processor_erle_db_last: Option<f64>,
    residual_echo_likelihood_last: Option<f64>,
    timing_records: usize,
    timing_discontinuities: usize,
    clock_drift_ppm: Option<f64>,
    coarse_render_offset_samples: Option<i64>,
    frame_process_us_p50: u64,
    frame_process_us_p95: u64,
    frame_process_us_max: u64,
    deletion: &'static str,
}

#[derive(Clone, Copy, Debug)]
struct TimingRecord {
    channel: u8,
    sample_offset: u64,
    sample_count: u32,
    host_time: u64,
    sample_time: f64,
}

pub(super) fn run(arguments: &[String]) -> Result<(), String> {
    let arguments = parse_arguments(arguments)?;
    validate_paths(&arguments)?;

    let render = read_mono_f32(&arguments.render)?;
    let microphone = read_mono_f32(&arguments.microphone)?;
    let timing = arguments.timing.as_deref().map(read_timing).transpose()?;
    let coarse_render_offset_samples = timing.as_deref().and_then(coarse_render_offset_samples);
    if render.is_empty() || microphone.is_empty() {
        return Err("render and microphone WAVs must contain samples".to_string());
    }

    let processor = Processor::new(SAMPLE_RATE).map_err(processor_error)?;
    processor.set_config(Config {
        echo_canceller: Some(EchoCanceller::Full {
            stream_delay_ms: None,
        }),
        ..Config::default()
    });

    let mut cleaned = Vec::with_capacity(microphone.len());
    let mut processing_us = Vec::new();
    let mut raw_energy = 0_f64;
    let mut cleaned_energy = 0_f64;
    let mut measured_frames = 0_usize;
    let mut erle_windows = Vec::new();
    let mut delays = Vec::new();
    let frames = render.len().max(microphone.len()).div_ceil(FRAME_SAMPLES);

    for frame_index in 0..frames {
        let start = frame_index * FRAME_SAMPLES;
        let mut render_frame = [0_f32; FRAME_SAMPLES];
        let mut microphone_frame = [0_f32; FRAME_SAMPLES];
        copy_frame_with_offset(
            &render,
            start,
            coarse_render_offset_samples.unwrap_or(0),
            &mut render_frame,
        );
        copy_frame_with_offset(&microphone, start, 0, &mut microphone_frame);

        let render_energy = energy(&render_frame);
        let started = Instant::now();
        processor
            .analyze_render_frame([&render_frame[..]])
            .map_err(processor_error)?;
        processor
            .process_capture_frame([&mut microphone_frame[..]])
            .map_err(processor_error)?;
        processing_us.push(started.elapsed().as_micros().min(u64::MAX as u128) as u64);

        let available = microphone.len().saturating_sub(start).min(FRAME_SAMPLES);
        if render_energy >= MIN_RENDER_ENERGY && available > 0 {
            let raw_window_energy = energy(&microphone[start..start + available]);
            let cleaned_window_energy = energy(&microphone_frame[..available]);
            raw_energy += raw_window_energy;
            cleaned_energy += cleaned_window_energy;
            if let Some(window_erle) = erle_db(raw_window_energy, cleaned_window_energy) {
                erle_windows.push(window_erle);
            }
            measured_frames += 1;
        }
        cleaned.extend_from_slice(&microphone_frame[..available]);
        if let Some(delay) = processor.get_stats().delay_ms {
            delays.push(delay);
        }
    }

    write_mono_f32(&arguments.output, &cleaned)?;
    processing_us.sort_unstable();
    erle_windows.sort_by(f64::total_cmp);
    delays.sort_unstable();
    let stats = processor.get_stats();
    let timing_summary = timing.as_deref().map(summarize_timing).unwrap_or_default();
    let report = Report {
        schema_version: 1,
        local_only: true,
        network_used: false,
        sample_rate_hz: SAMPLE_RATE,
        frame_samples: FRAME_SAMPLES,
        render_sha256: sha256_file(&arguments.render)?,
        microphone_sha256: sha256_file(&arguments.microphone)?,
        timing_sha256: arguments.timing.as_deref().map(sha256_file).transpose()?,
        cleaned_sha256: sha256_file(&arguments.output)?,
        render_samples: render.len(),
        microphone_samples: microphone.len(),
        cleaned_samples: cleaned.len(),
        measured_frames,
        erle_db_global: erle_db(raw_energy, cleaned_energy),
        erle_db_median: percentile_f64(&erle_windows, 50),
        erle_db_p10: percentile_f64(&erle_windows, 10),
        processor_delay_ms_p50: percentile_u32(&delays, 50),
        processor_delay_ms_p95: percentile_u32(&delays, 95),
        processor_erle_db_last: stats.echo_return_loss_enhancement,
        residual_echo_likelihood_last: stats.residual_echo_likelihood,
        timing_records: timing_summary.records,
        timing_discontinuities: timing_summary.discontinuities,
        clock_drift_ppm: timing_summary.clock_drift_ppm,
        coarse_render_offset_samples,
        frame_process_us_p50: percentile(&processing_us, 50),
        frame_process_us_p95: percentile(&processing_us, 95),
        frame_process_us_max: processing_us.last().copied().unwrap_or(0),
        deletion:
            "Delete this report and its paired WAV files to remove the local AEC spike artifact.",
    };
    write_report(&arguments.report, &report)?;
    println!(
        "aec-spike: {} frames, ERLE {}, report {}",
        frames,
        report
            .erle_db_median
            .map(|value| format!("{value:.2} dB"))
            .unwrap_or_else(|| "unavailable".to_string()),
        arguments.report.display()
    );
    Ok(())
}

/// Captures a consented, local paired fixture. This is intentionally a
/// feature-only command rather than a production protocol message: it never
/// exposes raw audio to the host, database, telemetry, or normal bundle.
pub(super) fn run_capture(arguments: &[String]) -> Result<(), String> {
    let arguments = parse_capture_arguments(arguments)?;
    crate::supported::disable_core_dumps();
    crate::supported::establish_process_group()
        .map_err(|_| "could not establish isolated capture process group".to_string())?;
    if !arguments.output_root.is_absolute() {
        return Err("--output-root must be absolute".to_string());
    }
    fs::create_dir_all(&arguments.output_root)
        .map_err(|error| format!("create output root: {error}"))?;
    if !arguments.output_root.is_dir() {
        return Err("--output-root is not a directory".to_string());
    }

    let captured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch".to_string())?;
    let session = arguments.output_root.join(format!(
        "session-{}-{}",
        captured_at.as_nanos(),
        std::process::id()
    ));
    fs::create_dir(&session)
        .map_err(|error| format!("create session {}: {error}", session.display()))?;

    let result = capture_session(&session, arguments.duration, captured_at.as_secs());
    if let Err(error) = result {
        // The directory was created above and is a single exact session target.
        let _ = fs::remove_dir_all(&session);
        return Err(error);
    }
    println!(
        "aec-spike: local paired fixture saved at {}",
        session.display()
    );
    Ok(())
}

fn parse_capture_arguments(arguments: &[String]) -> Result<CaptureArguments, String> {
    let mut output_root = None;
    let mut duration_seconds = None;
    let mut consented = false;
    let mut index = 0;
    while index < arguments.len() {
        let flag = &arguments[index];
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--output-root" => output_root = Some(PathBuf::from(value)),
            "--duration-seconds" => {
                duration_seconds = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| "--duration-seconds must be an integer".to_string())?,
                )
            }
            "--consent" => consented = value == CONSENT,
            _ => return Err(format!("unknown argument {flag}")),
        }
        index += 2;
    }
    if !consented {
        return Err(format!("--consent {CONSENT} is required"));
    }
    let duration_seconds = duration_seconds.ok_or("--duration-seconds is required")?;
    if !(1..=300).contains(&duration_seconds) {
        return Err("--duration-seconds must be between 1 and 300".to_string());
    }
    Ok(CaptureArguments {
        output_root: required_path(output_root, "--output-root")?,
        duration: Duration::from_secs(duration_seconds),
    })
}

fn capture_session(
    session: &Path,
    duration: Duration,
    captured_at_unix_seconds: u64,
) -> Result<(), String> {
    let render_temporary = session.join("render.wav.tmp");
    let microphone_temporary = session.join("microphone.wav.tmp");
    let timing_temporary = session.join("timing.bin.tmp");
    let render_path = session.join("render.wav");
    let microphone_path = session.join("microphone.wav");
    let timing_path = session.join("timing.bin");

    if !crate::system_audio::supported() {
        return Err("system audio capture requires macOS 14.2 or later".to_string());
    }
    let render_ring = Arc::new(crate::production::SpscRing::new());
    let render_clock = Arc::new(crate::production::CallbackClock::new());
    let mut render_stream = crate::system_audio::SystemAudioStream::start_observed_with_clock(
        Arc::clone(&render_ring),
        Some(Arc::clone(&render_clock)),
        |_, _| {},
    )
    .map_err(|code| format!("start system audio: {code:?}"))?;
    if render_stream.sample_rate() != SAMPLE_RATE {
        render_stream.stop();
        return Err(format!(
            "this Stage 0 build currently requires a 48 kHz system tap (got {} Hz)",
            render_stream.sample_rate()
        ));
    }

    let microphone_ring = Arc::new(crate::production::SpscRing::new());
    let microphone_clock = Arc::new(crate::production::CallbackClock::new());
    let mut microphone_stream = match crate::production::start_auhal(
        None,
        Arc::clone(&microphone_ring),
        Some(Arc::clone(&microphone_clock)),
        &mut |_| Ok(()),
    ) {
        Ok((stream, rate)) if rate == SAMPLE_RATE => stream,
        Ok((mut stream, rate)) => {
            stream.stop();
            render_stream.stop();
            return Err(format!(
                "expected 48 kHz microphone capture (got {rate} Hz)"
            ));
        }
        Err(code) => {
            render_stream.stop();
            return Err(format!("start microphone: {code:?}"));
        }
    };

    // The system tap is intentionally started first. Discard setup-era samples
    // after the microphone is live so both fixture files begin in the same
    // short capture epoch rather than preserving arbitrary setup latency.
    let mut discard = [0_f32; 4096];
    while render_ring.drain(&mut discard) > 0 {}
    while microphone_ring.drain(&mut discard) > 0 {}

    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut render_writer = WavWriter::create(&render_temporary, spec)
        .map_err(|error| format!("create render WAV: {error}"))?;
    let mut microphone_writer = WavWriter::create(&microphone_temporary, spec)
        .map_err(|error| format!("create microphone WAV: {error}"))?;
    let timing_file = File::create(&timing_temporary)
        .map_err(|error| format!("create timing sidecar: {error}"))?;
    let mut timing_writer = BufWriter::new(timing_file);
    timing_writer
        .write_all(b"MURMAEC\0\x01\0\0\0")
        .map_err(|error| format!("write timing header: {error}"))?;

    let mut render_scratch = [0_f32; 4096];
    let mut microphone_scratch = [0_f32; 4096];
    let mut render_samples = 0_u64;
    let mut microphone_samples = 0_u64;
    let started = Instant::now();
    let mut render_seen = false;
    let mut microphone_seen = false;
    let capture_result = loop {
        if render_ring.overflowed() || microphone_ring.overflowed() {
            break Err("capture ring overflowed; fixture was discarded".to_string());
        }
        let render_count = render_ring.drain(&mut render_scratch);
        if render_count > 0 {
            render_seen = true;
            write_samples(
                &mut render_writer,
                &render_scratch[..render_count],
                "render",
            )?;
            write_timing_record(
                &mut timing_writer,
                1,
                render_samples,
                render_count as u32,
                render_clock.snapshot(),
            )?;
            render_samples += render_count as u64;
        }
        let microphone_count = microphone_ring.drain(&mut microphone_scratch);
        if microphone_count > 0 {
            microphone_seen = true;
            write_samples(
                &mut microphone_writer,
                &microphone_scratch[..microphone_count],
                "microphone",
            )?;
            write_timing_record(
                &mut timing_writer,
                2,
                microphone_samples,
                microphone_count as u32,
                microphone_clock.snapshot(),
            )?;
            microphone_samples += microphone_count as u64;
        }
        if started.elapsed() >= duration {
            break if render_seen && microphone_seen {
                Ok(())
            } else {
                Err(
                    "audio callbacks did not produce both streams; fixture was discarded"
                        .to_string(),
                )
            };
        }
        std::thread::sleep(Duration::from_millis(4));
    };

    microphone_stream.stop();
    render_stream.stop();
    capture_result?;
    render_writer
        .finalize()
        .map_err(|error| format!("finalize render WAV: {error}"))?;
    microphone_writer
        .finalize()
        .map_err(|error| format!("finalize microphone WAV: {error}"))?;
    timing_writer
        .flush()
        .map_err(|error| format!("flush timing sidecar: {error}"))?;
    timing_writer
        .into_inner()
        .map_err(|error| format!("finish timing sidecar: {error}"))?
        .sync_all()
        .map_err(|error| format!("sync timing sidecar: {error}"))?;
    sync_file(&render_temporary)?;
    sync_file(&microphone_temporary)?;
    fs::rename(&render_temporary, &render_path)
        .map_err(|error| format!("publish render: {error}"))?;
    fs::rename(&microphone_temporary, &microphone_path)
        .map_err(|error| format!("publish microphone: {error}"))?;
    fs::rename(&timing_temporary, &timing_path)
        .map_err(|error| format!("publish timing: {error}"))?;

    let manifest = CaptureManifest {
        schema_version: 1,
        contains_real_user_data: true,
        local_only: true,
        network_used: false,
        consent_acknowledgment: CONSENT,
        captured_at_unix_seconds,
        tool_build_sha: BUILD_SHA,
        source: "consented_local_stage_0_capture",
        device_class: "default_input_and_system_tap",
        render: captured_file("render.wav", &render_path, Some(render_samples))?,
        microphone: captured_file("microphone.wav", &microphone_path, Some(microphone_samples))?,
        timing: captured_file("timing.bin", &timing_path, None)?,
        timing_kind:
            "latest_callback_anchor_at_each_worker_drain; host_time is Core Audio host-time ticks",
        deletion:
            "Delete this session directory to remove all raw audio, timing, and report artifacts.",
    };
    write_manifest(&session.join("manifest.json"), &manifest)
}

fn write_samples(
    writer: &mut WavWriter<std::io::BufWriter<File>>,
    samples: &[f32],
    label: &str,
) -> Result<(), String> {
    for sample in samples {
        writer
            .write_sample(*sample)
            .map_err(|error| format!("write {label} WAV: {error}"))?;
    }
    Ok(())
}

fn write_timing_record(
    writer: &mut BufWriter<File>,
    channel: u8,
    sample_offset: u64,
    sample_count: u32,
    (host_time, sample_time): (u64, f64),
) -> Result<(), String> {
    let mut record = [0_u8; 32];
    record[0] = channel;
    record[4..12].copy_from_slice(&sample_offset.to_le_bytes());
    record[12..16].copy_from_slice(&sample_count.to_le_bytes());
    record[16..24].copy_from_slice(&host_time.to_le_bytes());
    record[24..32].copy_from_slice(&sample_time.to_bits().to_le_bytes());
    writer
        .write_all(&record)
        .map_err(|error| format!("write timing sidecar: {error}"))
}

fn captured_file(
    file_name: &'static str,
    path: &Path,
    samples: Option<u64>,
) -> Result<CapturedFile, String> {
    Ok(CapturedFile {
        file_name,
        sha256: sha256_file(path)?,
        sample_rate_hz: samples.map(|_| SAMPLE_RATE),
        channels: samples.map(|_| 1),
        samples,
        bytes: fs::metadata(path)
            .map_err(|error| format!("metadata {}: {error}", path.display()))?
            .len(),
    })
}

fn write_manifest(path: &Path, manifest: &CaptureManifest) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let payload = serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?;
    let mut file = File::create(&temporary).map_err(|error| format!("write manifest: {error}"))?;
    file.write_all(&payload)
        .map_err(|error| format!("write manifest: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync manifest: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("publish manifest: {error}"))
}

fn sync_file(path: &Path) -> Result<(), String> {
    File::open(path)
        .map_err(|error| format!("open {} for sync: {error}", path.display()))?
        .sync_all()
        .map_err(|error| format!("sync {}: {error}", path.display()))
}

fn parse_arguments(arguments: &[String]) -> Result<Arguments, String> {
    let mut render = None;
    let mut microphone = None;
    let mut timing = None;
    let mut output = None;
    let mut report = None;
    let mut consented = false;
    let mut index = 0;
    while index < arguments.len() {
        let flag = &arguments[index];
        if flag == "--consent" {
            consented = arguments
                .get(index + 1)
                .is_some_and(|value| value == CONSENT);
        } else {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--render" => render = Some(PathBuf::from(value)),
                "--microphone" => microphone = Some(PathBuf::from(value)),
                "--timing" => timing = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                "--report" => report = Some(PathBuf::from(value)),
                _ => return Err(format!("unknown argument {flag}")),
            }
        }
        index += 2;
    }
    if !consented {
        return Err(format!("--consent {CONSENT} is required"));
    }
    Ok(Arguments {
        render: required_path(render, "--render")?,
        microphone: required_path(microphone, "--microphone")?,
        timing,
        output: required_path(output, "--output")?,
        report: required_path(report, "--report")?,
    })
}

fn required_path(path: Option<PathBuf>, flag: &str) -> Result<PathBuf, String> {
    path.ok_or_else(|| format!("{flag} is required"))
}

fn validate_paths(arguments: &Arguments) -> Result<(), String> {
    for (label, path) in [
        ("render", &arguments.render),
        ("microphone", &arguments.microphone),
        ("output", &arguments.output),
        ("report", &arguments.report),
    ] {
        if !path.is_absolute() {
            return Err(format!("{label} path must be absolute"));
        }
    }
    if let Some(timing) = &arguments.timing {
        if !timing.is_absolute() {
            return Err("timing path must be absolute".to_string());
        }
        if !timing.is_file() {
            return Err(format!(
                "timing sidecar does not exist: {}",
                timing.display()
            ));
        }
    }
    for input in [&arguments.render, &arguments.microphone] {
        if !input.is_file() {
            return Err(format!("input WAV does not exist: {}", input.display()));
        }
    }
    if arguments.output == arguments.render
        || arguments.output == arguments.microphone
        || arguments.report == arguments.render
        || arguments.report == arguments.microphone
        || arguments.output == arguments.report
    {
        return Err(
            "output/report paths must be distinct and not overwrite an input WAV".to_string(),
        );
    }
    for output in [&arguments.output, &arguments.report] {
        let parent = output
            .parent()
            .ok_or_else(|| format!("output has no parent: {}", output.display()))?;
        if !parent.is_dir() {
            return Err(format!(
                "output parent does not exist: {}",
                parent.display()
            ));
        }
        if output.exists() {
            return Err(format!("output already exists: {}", output.display()));
        }
    }
    Ok(())
}

#[derive(Default)]
struct TimingSummary {
    records: usize,
    discontinuities: usize,
    clock_drift_ppm: Option<f64>,
}

fn read_timing(path: &Path) -> Result<Vec<TimingRecord>, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    const HEADER: &[u8; 12] = b"MURMAEC\0\x01\0\0\0";
    if bytes.len() < HEADER.len() || &bytes[..HEADER.len()] != HEADER {
        return Err(format!("{} has an invalid timing header", path.display()));
    }
    let payload = &bytes[HEADER.len()..];
    if payload.len() % 32 != 0 {
        return Err(format!("{} has a truncated timing record", path.display()));
    }
    payload
        .chunks_exact(32)
        .map(|record| {
            let channel = record[0];
            if !matches!(channel, 1 | 2) {
                return Err("timing sidecar has an unknown channel".to_string());
            }
            Ok(TimingRecord {
                channel,
                sample_offset: u64::from_le_bytes(record[4..12].try_into().unwrap()),
                sample_count: u32::from_le_bytes(record[12..16].try_into().unwrap()),
                host_time: u64::from_le_bytes(record[16..24].try_into().unwrap()),
                sample_time: f64::from_bits(u64::from_le_bytes(record[24..32].try_into().unwrap())),
            })
        })
        .collect()
}

fn summarize_timing(records: &[TimingRecord]) -> TimingSummary {
    let mut summary = TimingSummary {
        records: records.len(),
        ..TimingSummary::default()
    };
    let mut render = Vec::new();
    let mut microphone = Vec::new();
    let mut prior = [(0_u64, 0_u64, 0_u64, f64::NEG_INFINITY, false); 3];
    for record in records {
        let slot = &mut prior[record.channel as usize];
        if slot.4
            && (record.sample_offset != slot.0.saturating_add(slot.1)
                || record.host_time < slot.2
                || record.sample_time < slot.3)
        {
            summary.discontinuities += 1;
        }
        *slot = (
            record.sample_offset,
            u64::from(record.sample_count),
            record.host_time,
            record.sample_time,
            true,
        );
        if record.host_time > 0 && record.sample_time.is_finite() {
            match record.channel {
                1 => render.push((record.host_time as f64, record.sample_time)),
                2 => microphone.push((record.host_time as f64, record.sample_time)),
                _ => {}
            }
        }
    }
    summary.clock_drift_ppm = match (linear_slope(&render), linear_slope(&microphone)) {
        (Some(render_slope), Some(microphone_slope))
            if render_slope.is_finite() && microphone_slope.is_finite() && render_slope > 0.0 =>
        {
            Some((microphone_slope / render_slope - 1.0) * 1_000_000.0)
        }
        _ => None,
    };
    summary
}

fn linear_slope(points: &[(f64, f64)]) -> Option<f64> {
    linear_fit(points).map(|(slope, _)| slope)
}

fn linear_fit(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 3 {
        return None;
    }
    let count = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / count;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / count;
    let (covariance, variance) = points.iter().fold((0.0, 0.0), |(cov, var), (x, y)| {
        let dx = x - mean_x;
        (cov + dx * (y - mean_y), var + dx * dx)
    });
    if variance <= 0.0 {
        return None;
    }
    let slope = covariance / variance;
    Some((slope, mean_y - slope * mean_x))
}

/// Estimate which render-file sample is contemporaneous with microphone-file
/// sample zero. The fixture stores only latest-callback anchors, so this is a
/// coarse initial alignment; AEC3 still owns fine delay estimation.
fn coarse_render_offset_samples(records: &[TimingRecord]) -> Option<i64> {
    let mut render = Vec::new();
    let mut microphone = Vec::new();
    for record in records {
        if record.host_time == 0 {
            continue;
        }
        let point = (
            record.host_time as f64,
            record
                .sample_offset
                .saturating_add(u64::from(record.sample_count)) as f64,
        );
        match record.channel {
            1 => render.push(point),
            2 => microphone.push(point),
            _ => {}
        }
    }
    let (render_slope, render_intercept) = linear_fit(&render)?;
    let (microphone_slope, microphone_intercept) = linear_fit(&microphone)?;
    if !render_slope.is_finite()
        || !microphone_slope.is_finite()
        || render_slope <= 0.0
        || microphone_slope <= 0.0
    {
        return None;
    }
    let microphone_zero_host_time = -microphone_intercept / microphone_slope;
    let render_offset = render_slope * microphone_zero_host_time + render_intercept;
    let rounded = render_offset.round();
    // A setup or timestamp anomaly outside this narrow feasibility window is
    // not a trustworthy alignment input. Let AEC3 use its native estimate.
    (rounded.is_finite() && rounded.abs() <= 10.0 * SAMPLE_RATE as f64).then_some(rounded as i64)
}

fn read_mono_f32(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader =
        WavReader::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != SAMPLE_RATE
        || spec.sample_format != SampleFormat::Float
        || spec.bits_per_sample != 32
    {
        return Err(format!(
            "{} must be 48 kHz mono 32-bit float WAV",
            path.display()
        ));
    }
    reader
        .samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read {}: {error}", path.display()))
}

fn write_mono_f32(path: &Path, samples: &[f32]) -> Result<(), String> {
    let temporary = path.with_extension("tmp.wav");
    let spec = WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&temporary, spec)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    for sample in samples {
        writer
            .write_sample(*sample)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    }
    writer
        .finalize()
        .map_err(|error| format!("finalize {}: {error}", temporary.display()))?;
    sync_file(&temporary)?;
    fs::rename(&temporary, path).map_err(|error| format!("publish {}: {error}", path.display()))
}

fn write_report(path: &Path, report: &Report) -> Result<(), String> {
    let temporary = path.with_extension("tmp.json");
    let payload = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let mut file = File::create(&temporary)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| format!("publish {}: {error}", path.display()))
}

fn copy_frame_with_offset(
    input: &[f32],
    start: usize,
    offset: i64,
    output: &mut [f32; FRAME_SAMPLES],
) {
    for (index, sample) in output.iter_mut().enumerate() {
        let source_index = start as i64 + index as i64 + offset;
        if let Ok(source_index) = usize::try_from(source_index) {
            if let Some(value) = input.get(source_index) {
                *sample = *value;
            }
        }
    }
}

fn energy(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum()
}

fn erle_db(raw_energy: f64, cleaned_energy: f64) -> Option<f64> {
    (raw_energy > 0.0 && cleaned_energy > 0.0).then(|| 10.0 * (raw_energy / cleaned_energy).log10())
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = (values.len() - 1) * percentile / 100;
    values[index]
}

fn percentile_u32(values: &[u32], percentile: usize) -> Option<u32> {
    (!values.is_empty()).then(|| values[(values.len() - 1) * percentile / 100])
}

fn percentile_f64(values: &[f64], percentile: usize) -> Option<f64> {
    (!values.is_empty()).then(|| values[(values.len() - 1) * percentile / 100])
}

fn processor_error(error: webrtc_audio_processing::Error) -> String {
    format!("AEC3 processor error: {error}")
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erle_requires_nonzero_energy() {
        assert_eq!(erle_db(0.0, 1.0), None);
        assert_eq!(erle_db(1.0, 0.0), None);
        assert!(erle_db(100.0, 1.0).is_some_and(|value| value > 10.0));
    }

    #[test]
    fn percentile_uses_nearest_rank_lower_bound() {
        assert_eq!(percentile(&[], 95), 0);
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 50), 3);
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 95), 4);
    }

    #[test]
    fn capture_arguments_require_exact_consent_and_bounded_duration() {
        let root = "/private/tmp/murmur-aec-spike".to_string();
        let valid = vec![
            "--output-root".to_string(),
            root.clone(),
            "--duration-seconds".to_string(),
            "30".to_string(),
            "--consent".to_string(),
            CONSENT.to_string(),
        ];
        let parsed = parse_capture_arguments(&valid).unwrap();
        assert_eq!(parsed.output_root, PathBuf::from(root));
        assert_eq!(parsed.duration, Duration::from_secs(30));

        let missing_consent = valid[..4].to_vec();
        assert!(parse_capture_arguments(&missing_consent).is_err());
        let too_long = vec![
            "--output-root".to_string(),
            "/private/tmp/murmur-aec-spike".to_string(),
            "--duration-seconds".to_string(),
            "301".to_string(),
            "--consent".to_string(),
            CONSENT.to_string(),
        ];
        assert!(parse_capture_arguments(&too_long).is_err());
    }

    #[test]
    fn timing_record_is_fixed_width_and_content_free() {
        let path = std::env::temp_dir().join(format!(
            "murmur-aec-spike-timing-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let file = File::create(&path).unwrap();
            let mut writer = BufWriter::new(file);
            write_timing_record(&mut writer, 1, 48_000, 480, (123, 456.0)).unwrap();
            writer.flush().unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(bytes.len(), 32);
        assert_eq!(bytes[0], 1);
        assert_eq!(&bytes[4..12], &48_000_u64.to_le_bytes());
        assert_eq!(&bytes[12..16], &480_u32.to_le_bytes());
    }

    #[test]
    fn offline_analyzer_writes_cleaned_audio_and_local_report() {
        let session = std::env::temp_dir().join(format!(
            "murmur-aec-spike-analysis-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&session).unwrap();
        let render = session.join("render.wav");
        let microphone = session.join("microphone.wav");
        let output = session.join("cleaned.wav");
        let report = session.join("report.json");
        let frame = (0..FRAME_SAMPLES)
            .map(|index| (index as f32 * 0.01).sin() * 0.1)
            .collect::<Vec<_>>();
        write_mono_f32(&render, &frame).unwrap();
        write_mono_f32(&microphone, &frame).unwrap();

        run(&[
            "--render".to_string(),
            render.to_string_lossy().into_owned(),
            "--microphone".to_string(),
            microphone.to_string_lossy().into_owned(),
            "--output".to_string(),
            output.to_string_lossy().into_owned(),
            "--report".to_string(),
            report.to_string_lossy().into_owned(),
            "--consent".to_string(),
            CONSENT.to_string(),
        ])
        .unwrap();

        assert_eq!(read_mono_f32(&output).unwrap().len(), FRAME_SAMPLES);
        let report_text = fs::read_to_string(&report).unwrap();
        assert!(report_text.contains("\"localOnly\": true"));
        assert!(report_text.contains("\"networkUsed\": false"));
        fs::remove_dir_all(session).unwrap();
    }

    #[test]
    fn timing_summary_estimates_relative_clock_drift() {
        let mut records = Vec::new();
        for index in 0..4_u64 {
            let host = index * 48_000;
            records.push(TimingRecord {
                channel: 1,
                sample_offset: host,
                sample_count: 48_000,
                host_time: host,
                sample_time: host as f64,
            });
            records.push(TimingRecord {
                channel: 2,
                sample_offset: host,
                sample_count: 48_000,
                host_time: host,
                sample_time: host as f64 * 1.000_2,
            });
        }
        let summary = summarize_timing(&records);
        assert_eq!(summary.records, 8);
        assert_eq!(summary.discontinuities, 0);
        assert!(summary
            .clock_drift_ppm
            .is_some_and(|ppm| (ppm - 200.0).abs() < 0.1));
    }

    #[test]
    fn coarse_alignment_maps_microphone_zero_to_render_samples() {
        let mut records = Vec::new();
        for index in 0..4_u64 {
            let host = 1_000 + index * 480;
            records.push(TimingRecord {
                channel: 1,
                sample_offset: index * 480,
                sample_count: 480,
                host_time: host,
                sample_time: index as f64 * 480.0,
            });
            records.push(TimingRecord {
                channel: 2,
                sample_offset: index * 480,
                sample_count: 480,
                host_time: host + 100,
                sample_time: index as f64 * 480.0,
            });
        }
        assert_eq!(coarse_render_offset_samples(&records), Some(100));
    }
}
