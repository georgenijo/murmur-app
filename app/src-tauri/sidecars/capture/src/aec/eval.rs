//! Offline feasibility evaluation of the Meeting Capture echo canceller (#594).
//!
//! Test-only: this module compiles only under `cfg(test)` and is not reachable
//! from any runtime path. It never opens an audio device, plays sound, or reads
//! private audio. Every fixture is synthesized from a fixed seed, so a run is
//! reproducible bit for bit on the same machine.
//!
//! The fixtures model one acoustic loop:
//!
//! ```text
//! render (System Audio / "Them") -> [speaker nonlinearity] -> [clock drift]
//!     -> bulk delay -> room impulse response -> echo at the microphone
//! mic = echo + near-end talker ("Me") + noise floor
//! ```
//!
//! Candidates, all scored in the 16 kHz band that meeting ASR consumes:
//!
//! - the exact AEC3 configuration the worker ships (`WebRtcEchoProcessor::new`)
//!   at 48 kHz, driven in 10 ms frames as production drives it;
//! - the same AEC3 configuration run on 16 kHz streams;
//! - a linear NLMS filter given the true delay, as a reference point for how
//!   much of the reduction is linear cancellation.
//!
//! Fast checks run in the ordinary suite. The full matrix is an ignored test:
//!
//! ```text
//! cd app/src-tauri
//! cargo test --release -p murmur-capture-helper --bin murmur-capture-helper \
//!   aec::eval::feasibility_report -- --ignored --nocapture
//! ```
//!
//! Set `MURMUR_AEC_EVAL_WAV_DIR=/some/local/dir` to also write each fixture
//! and its processed output as 16-bit WAV files for offline listening or
//! inspection. Nothing is written unless that variable is set.

use super::{MeetingMicrophonePath, WebRtcEchoProcessor, FRAME_SAMPLES, SAMPLE_RATE};
use murmur_capture_helper_protocol::{EchoCancellationMode, EchoCancellationStatus};
use std::time::{Duration, Instant};

const FS: f32 = SAMPLE_RATE as f32;

// ---------------------------------------------------------------- fixtures --

/// xorshift64* — deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// Approximately standard normal (Irwin-Hall, 4 terms).
    fn gauss(&mut self) -> f32 {
        let sum: f32 = (0..4).map(|_| self.unit()).sum();
        (sum - 2.0) * 3.0f32.sqrt()
    }
}

/// Two-pole resonator, the classic formant filter.
struct Resonator {
    a1: f32,
    a2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    fn new(freq: f32, bandwidth: f32) -> Self {
        let r = (-std::f32::consts::PI * bandwidth / FS).exp();
        let w = 2.0 * std::f32::consts::PI * freq / FS;
        Self {
            a1: 2.0 * r * w.cos(),
            a2: -r * r,
            gain: 1.0 - r,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn retune(&mut self, freq: f32, bandwidth: f32) {
        let next = Self::new(freq, bandwidth);
        self.a1 = next.a1;
        self.a2 = next.a2;
        self.gain = next.gain;
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.gain * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A synthetic talker: pitched glottal pulses through three moving formants,
/// fricative noise onsets, syllable envelopes, and phrase pauses. It is not
/// speech, but it has speech's spectral tilt, harmonic structure, and on/off
/// statistics, which is what drives an echo canceller's adaptation and
/// double-talk logic.
#[derive(Clone, Copy)]
struct Talker {
    seed: u64,
    f0: f32,
    formant_scale: f32,
}

const VOWELS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0],
    [270.0, 2290.0, 3010.0],
    [530.0, 1840.0, 2480.0],
    [300.0, 870.0, 2240.0],
    [570.0, 840.0, 2410.0],
];

fn synth_talker(talker: Talker, samples: usize, active_rms_dbfs: f32) -> Vec<f32> {
    let mut rng = Rng::new(talker.seed);
    let mut out = vec![0.0f32; samples];
    let mut formants = [
        Resonator::new(VOWELS[0][0], 90.0),
        Resonator::new(VOWELS[0][1], 110.0),
        Resonator::new(VOWELS[0][2], 170.0),
    ];
    let mut phase = 0.0f32;
    let mut glottal = 0.0f32;
    let mut envelope = 0.0f32;
    let mut position = 0usize;
    while position < samples {
        // One phrase of syllables, then a pause.
        let phrase_end = (position + (rng.range(1.4, 2.6) * FS) as usize).min(samples);
        while position < phrase_end {
            let syllable = (rng.range(0.12, 0.26) * FS) as usize;
            let gap = (rng.range(0.02, 0.07) * FS) as usize;
            let vowel = VOWELS[(rng.next_u64() % VOWELS.len() as u64) as usize];
            for (resonator, (freq, bw)) in formants
                .iter_mut()
                .zip(vowel.iter().zip([90.0, 110.0, 170.0]))
            {
                resonator.retune(freq * talker.formant_scale, bw);
            }
            let fricative = (rng.unit() < 0.3).then_some((0.04 * FS) as usize);
            let pitch_bend = rng.range(0.9, 1.15);
            let end = (position + syllable + gap).min(samples);
            for (offset, sample) in out[position..end].iter_mut().enumerate() {
                let t = (position + offset) as f32 / FS;
                let voiced = offset < syllable;
                let target = if voiced { 1.0 } else { 0.08 };
                envelope += (target - envelope) * 0.004;
                let f0 = talker.f0
                    * pitch_bend
                    * (1.0 + 0.04 * (2.0 * std::f32::consts::PI * 3.1 * t).sin())
                    * (1.0 - 0.15 * offset as f32 / syllable as f32);
                phase += f0 / FS;
                let pulse = if phase >= 1.0 {
                    phase -= 1.0;
                    1.0
                } else {
                    0.0
                };
                glottal = 0.97 * glottal + pulse;
                let source = glottal + 0.02 * rng.gauss();
                let voiced_sample: f32 = formants.iter_mut().map(|f| f.tick(source)).sum();
                let noise = match fricative {
                    Some(length) if offset < length => {
                        0.25 * rng.gauss() * (1.0 - offset as f32 / length as f32)
                    }
                    _ => 0.0,
                };
                *sample = envelope * voiced_sample + noise;
            }
            position = end;
        }
        let pause = (rng.range(0.35, 0.8) * FS) as usize;
        let end = (position + pause).min(samples);
        for sample in &mut out[position..end] {
            envelope *= 0.996;
            *sample = 0.0;
        }
        position = end;
    }
    // First-difference pre-emphasis gives speech-like spectral tilt.
    let mut previous = 0.0;
    for sample in &mut out {
        let current = *sample;
        *sample = current - 0.9 * previous;
        previous = current;
    }
    normalize_active(&mut out, active_rms_dbfs);
    out
}

/// Scale so the RMS of the active (non-silent) frames equals `dbfs`.
fn normalize_active(signal: &mut [f32], dbfs: f32) {
    let energies = frame_energy(signal);
    let peak = energies.iter().copied().fold(0.0f64, f64::max);
    let active: Vec<f64> = energies.into_iter().filter(|&e| e > peak * 1e-3).collect();
    if active.is_empty() {
        return;
    }
    let rms = (active.iter().sum::<f64>() / active.len() as f64).sqrt();
    let gain = (db_to_amp(dbfs) as f64 / rms) as f32;
    for sample in signal {
        *sample *= gain;
    }
}

/// Sparse room impulse response: a direct path, discrete early reflections,
/// then an exponentially decaying diffuse tail. Normalized to unit energy.
fn room_ir(seed: u64, rt60_s: f32) -> Vec<(usize, f32)> {
    let mut rng = Rng::new(seed ^ 0xA5A5);
    let mut taps = vec![(0usize, 1.0f32)];
    for _ in 0..10 {
        let at = rng.range(0.002, 0.025);
        let sign = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        taps.push(((at * FS) as usize, sign * rng.range(0.25, 0.6)));
    }
    // Tail to -30 dB (half of RT60), one tap every 4-16 samples.
    let tail_end = (0.5 * rt60_s * FS) as usize;
    let mut at = (0.025 * FS) as usize;
    while at < tail_end {
        let t = at as f32 / FS;
        let decay = 10f32.powf(-3.0 * t / rt60_s);
        taps.push((at, 0.35 * decay * rng.gauss()));
        at += 4 + (rng.next_u64() % 13) as usize;
    }
    let energy: f32 = taps.iter().map(|(_, g)| g * g).sum();
    let scale = energy.sqrt().recip();
    for (_, gain) in &mut taps {
        *gain *= scale;
    }
    taps
}

fn convolve_sparse(input: &[f32], taps: &[(usize, f32)], delay: usize, gain: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; input.len()];
    for &(offset, tap) in taps {
        let shift = offset + delay;
        if shift >= input.len() {
            continue;
        }
        let g = tap * gain;
        for (o, &x) in out[shift..].iter_mut().zip(input) {
            *o += g * x;
        }
    }
    out
}

/// Resample by `1 + ppm * 1e-6` with linear interpolation: the speaker's
/// clock runs at a slightly different rate than the render tap's.
fn drift(input: &[f32], ppm: f32) -> Vec<f32> {
    if ppm == 0.0 {
        return input.to_vec();
    }
    let step = 1.0 + f64::from(ppm) * 1e-6;
    (0..input.len())
        .map(|n| {
            let position = n as f64 * step;
            let index = position as usize;
            let fraction = (position - index as f64) as f32;
            let a = input.get(index).copied().unwrap_or(0.0);
            let b = input.get(index + 1).copied().unwrap_or(0.0);
            a + (b - a) * fraction
        })
        .collect()
}

/// In double-talk fixtures the near-end talker joins after this much
/// far-end-only audio.
const DOUBLE_TALK_ONSET_S: f32 = 3.0;

#[derive(Clone, Copy, PartialEq)]
enum Talk {
    FarOnly,
    NearOnly,
    Double,
}

#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    talk: Talk,
    seconds: f32,
    /// Render reference level (active RMS).
    render_dbfs: f32,
    /// Echo path attenuation from render to microphone (ERL), dB.
    erl_db: f32,
    near_dbfs: f32,
    /// Positive: the echo reaches the microphone after the reference frame.
    /// Negative: the reference arrives late (non-causal for the canceller).
    delay_ms: f32,
    rt60_s: f32,
    drift_ppm: f32,
    /// Speaker overdrive; 0 means linear.
    clip_drive: f32,
    noise_dbfs: f32,
}

const BASE: Scenario = Scenario {
    name: "baseline",
    talk: Talk::FarOnly,
    seconds: 10.0,
    render_dbfs: -20.0,
    erl_db: 10.0,
    near_dbfs: -26.0,
    delay_ms: 40.0,
    rt60_s: 0.25,
    drift_ppm: 0.0,
    clip_drive: 0.0,
    noise_dbfs: -70.0,
};

const FAR_TALKER: Talker = Talker {
    seed: 0x5EED_0594,
    f0: 118.0,
    formant_scale: 1.0,
};
const NEAR_TALKER: Talker = Talker {
    seed: 0x0594_C0DE,
    f0: 205.0,
    formant_scale: 1.12,
};

struct Fixture {
    render: Vec<f32>,
    mic: Vec<f32>,
    /// Echo component of `mic` only.
    echo: Vec<f32>,
    /// Near-end component of `mic` only: the reference for preservation.
    near: Vec<f32>,
}

fn build_fixture(s: &Scenario) -> Fixture {
    let samples = (s.seconds * FS) as usize;
    let render = if s.talk == Talk::NearOnly {
        vec![0.0; samples]
    } else {
        synth_talker(FAR_TALKER, samples, s.render_dbfs)
    };
    let near = match s.talk {
        Talk::FarOnly => vec![0.0; samples],
        Talk::NearOnly => synth_talker(NEAR_TALKER, samples, s.near_dbfs),
        // Far-end alone first, so the canceller has converged before the
        // local talker joins.
        Talk::Double => {
            let onset = (DOUBLE_TALK_ONSET_S * FS) as usize;
            let mut near = vec![0.0; samples];
            near[onset..].copy_from_slice(&synth_talker(NEAR_TALKER, samples - onset, s.near_dbfs));
            near
        }
    };
    let speaker: Vec<f32> = if s.clip_drive > 0.0 {
        render
            .iter()
            .map(|&x| (x * s.clip_drive).tanh() / s.clip_drive)
            .collect()
    } else {
        render.clone()
    };
    let speaker = drift(&speaker, s.drift_ppm);
    let taps = room_ir(0x0594, s.rt60_s);
    let delay = (s.delay_ms.abs() / 1000.0 * FS) as usize;
    let gain = db_to_amp(-s.erl_db);
    let mut echo = convolve_sparse(&speaker, &taps, 0, gain);
    // Apply the bulk delay to whichever stream is late.
    let render = if s.delay_ms >= 0.0 {
        echo = shift_right(&echo, delay);
        render
    } else {
        shift_right(&render, delay)
    };
    let mut rng = Rng::new(0x0594_0000 ^ samples as u64);
    let noise = db_to_amp(s.noise_dbfs);
    let mic = echo
        .iter()
        .zip(&near)
        .map(|(&e, &n)| e + n + noise * rng.gauss())
        .collect();
    Fixture {
        render,
        mic,
        echo,
        near,
    }
}

fn shift_right(signal: &[f32], by: usize) -> Vec<f32> {
    let mut out = vec![0.0; signal.len()];
    if by < signal.len() {
        out[by..].copy_from_slice(&signal[..signal.len() - by]);
    }
    out
}

// -------------------------------------------------------------- candidates --

/// Meeting ASR consumes 16 kHz mono, so every candidate is scored in that
/// band: 0-7 kHz, decimated by three, with the candidate's fixed processing
/// latency removed.
const EVAL_RATE: u32 = 16_000;
const EVAL_FRAME: usize = EVAL_RATE as usize / 100;
const EVAL_FS: f32 = EVAL_RATE as f32;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Candidate {
    /// The worker's AEC3 configuration at 48 kHz, exactly as shipped.
    Shipped48k,
    /// The same AEC3 configuration on 16 kHz streams (band-limited and
    /// decimated before processing), matching the ASR rate.
    Aec3At16k,
    /// Time-domain NLMS on 16 kHz streams with the true bulk delay given to
    /// it. A linear-only reference point, not a product candidate.
    NlmsOracle16k,
}

impl Candidate {
    fn label(self) -> &'static str {
        match self {
            Self::Shipped48k => "AEC3 @ 48 kHz (shipped config)",
            Self::Aec3At16k => "AEC3 @ 16 kHz (same config)",
            Self::NlmsOracle16k => "NLMS @ 16 kHz, oracle delay, 2048 taps",
        }
    }
}

struct Run {
    /// Output in the evaluation domain, latency-compensated.
    out: Vec<f32>,
    processing: Duration,
}

/// Windowed-sinc low-pass at 7 kHz (255 taps, Blackman), zero-phase by
/// removing its group delay.
fn lowpass_7k(signal: &[f32]) -> Vec<f32> {
    const TAPS: usize = 255;
    let fc = 7000.0f64 / f64::from(SAMPLE_RATE);
    let mid = (TAPS / 2) as f64;
    let taps: Vec<(usize, f32)> = (0..TAPS)
        .map(|i| {
            let t = i as f64 - mid;
            let sinc = if t == 0.0 {
                2.0 * fc
            } else {
                (2.0 * std::f64::consts::PI * fc * t).sin() / (std::f64::consts::PI * t)
            };
            let phase = 2.0 * std::f64::consts::PI * i as f64 / (TAPS - 1) as f64;
            let window = 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
            (i, (sinc * window) as f32)
        })
        .collect();
    let mut padded = signal.to_vec();
    padded.extend(std::iter::repeat_n(0.0, TAPS / 2));
    convolve_sparse(&padded, &taps, 0, 1.0)[TAPS / 2..].to_vec()
}

/// 48 kHz -> 16 kHz: the ASR band.
fn to_eval(signal: &[f32]) -> Vec<f32> {
    lowpass_7k(signal).into_iter().step_by(3).collect()
}

fn advance(signal: &[f32], by: usize) -> Vec<f32> {
    let mut out = signal[by.min(signal.len())..].to_vec();
    out.resize(signal.len(), 0.0);
    out
}

fn aec3_processor(rate: u32) -> webrtc_audio_processing::Processor {
    if rate == SAMPLE_RATE {
        // The worker's own constructor, so the shipped config is what runs.
        return WebRtcEchoProcessor::new()
            .expect("AEC3 initializes")
            .processor;
    }
    // Same settings as `WebRtcEchoProcessor::new`, at another rate.
    use webrtc_audio_processing::{config::EchoCanceller, Config, Processor};
    let processor = Processor::new(rate).expect("AEC3 initializes");
    processor.set_config(Config {
        echo_canceller: Some(EchoCanceller::Full {
            stream_delay_ms: None,
        }),
        ..Config::default()
    });
    processor
}

/// Render first, then the capture frame for the same 10 ms, as the worker
/// drives it. Returns the raw output at the processing rate.
fn drive_aec3(rate: u32, render: &[f32], mic: &[f32]) -> (Vec<f32>, Duration) {
    let processor = aec3_processor(rate);
    let frame = rate as usize / 100;
    let mut out = Vec::with_capacity(mic.len());
    let mut processing = Duration::ZERO;
    let mut capture = vec![0.0f32; frame];
    for (r, m) in render.chunks_exact(frame).zip(mic.chunks_exact(frame)) {
        capture.copy_from_slice(m);
        let started = Instant::now();
        processor.analyze_render_frame([r]).unwrap();
        processor.process_capture_frame([&mut capture[..]]).unwrap();
        processing += started.elapsed();
        out.extend_from_slice(&capture);
    }
    out.resize(mic.len(), 0.0);
    (out, processing)
}

/// AEC3's fixed processing latency at `rate`: the position of an impulse
/// pushed through with a silent reference.
fn aec3_latency(rate: u32) -> usize {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Vec<(u32, usize)>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(&(_, latency)) = cache.lock().unwrap().iter().find(|(r, _)| *r == rate) {
        return latency;
    }
    let n = rate as usize;
    let mut mic = vec![0.0f32; n];
    mic[n / 2] = 0.5;
    let (out, _) = drive_aec3(rate, &vec![0.0; n], &mic);
    let peak = out
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, _)| i)
        .unwrap();
    let latency = peak - n / 2;
    cache.lock().unwrap().push((rate, latency));
    latency
}

fn nlms(render: &[f32], mic: &[f32], delay: usize, taps: usize) -> Vec<f32> {
    let mu = 0.4f32;
    let mut weights = vec![0.0f32; taps];
    // Newest sample at index 0 after each rotate.
    let mut history = vec![0.0f32; taps];
    let mut power = 0.0f32;
    let mut out = Vec::with_capacity(mic.len());
    for (n, &m) in mic.iter().enumerate() {
        let x = if n >= delay { render[n - delay] } else { 0.0 };
        power += x * x - history[taps - 1] * history[taps - 1];
        power = power.max(0.0);
        history.rotate_right(1);
        history[0] = x;
        let estimate: f32 = weights.iter().zip(&history).map(|(w, h)| w * h).sum();
        let error = m - estimate;
        let step = mu * error / (power + 1e-4);
        for (w, &h) in weights.iter_mut().zip(&history) {
            *w += step * h;
        }
        out.push(error);
    }
    out
}

struct EvalFixture {
    render: Vec<f32>,
    mic: Vec<f32>,
    echo: Vec<f32>,
    near: Vec<f32>,
}

fn eval_fixture(f: &Fixture) -> EvalFixture {
    EvalFixture {
        render: to_eval(&f.render),
        mic: to_eval(&f.mic),
        echo: to_eval(&f.echo),
        near: to_eval(&f.near),
    }
}

fn run(candidate: Candidate, s: &Scenario, f: &Fixture, e: &EvalFixture) -> Run {
    match candidate {
        Candidate::Shipped48k => {
            let (out, processing) = drive_aec3(SAMPLE_RATE, &f.render, &f.mic);
            Run {
                out: to_eval(&advance(&out, aec3_latency(SAMPLE_RATE))),
                processing,
            }
        }
        Candidate::Aec3At16k => {
            let (out, processing) = drive_aec3(EVAL_RATE, &e.render, &e.mic);
            Run {
                out: advance(&out, aec3_latency(EVAL_RATE)),
                processing,
            }
        }
        Candidate::NlmsOracle16k => {
            let delay = (s.delay_ms.max(0.0) / 1000.0 * EVAL_FS) as usize;
            let started = Instant::now();
            let out = nlms(&e.render, &e.mic, delay, 2048);
            Run {
                out,
                processing: started.elapsed(),
            }
        }
    }
}

/// Run audio through the production `MeetingMicrophonePath`.
fn run_production_path(render: &[f32], mic: &[f32]) -> Vec<f32> {
    let mut path =
        MeetingMicrophonePath::new(EchoCancellationMode::Enabled, SAMPLE_RATE, SAMPLE_RATE);
    assert_eq!(path.status(), EchoCancellationStatus::Active);
    let mut out = Vec::with_capacity(mic.len());
    for (r, m) in render
        .chunks_exact(FRAME_SAMPLES)
        .zip(mic.chunks_exact(FRAME_SAMPLES))
    {
        assert_eq!(path.push_render(r), None);
        let status = path
            .push_capture(m, |samples| {
                out.extend_from_slice(samples);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(status, None);
    }
    path.finish(|samples| {
        out.extend_from_slice(samples);
        Ok::<_, ()>(())
    })
    .unwrap();
    out
}

// ----------------------------------------------------------------- metrics --

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-20).log10()
}

fn frame_energy_with(signal: &[f32], frame: usize) -> Vec<f64> {
    signal
        .chunks(frame)
        .map(|c| c.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / c.len() as f64)
        .collect()
}

fn frame_energy(signal: &[f32]) -> Vec<f64> {
    frame_energy_with(signal, FRAME_SAMPLES)
}

/// Evaluation-domain frames that carry `component` above -50 dBFS RMS.
fn active_frames(component: &[f32], from_frame: usize) -> Vec<bool> {
    let threshold = f64::from(db_to_amp(-50.0)).powi(2);
    frame_energy_with(component, EVAL_FRAME)
        .into_iter()
        .enumerate()
        .map(|(i, e)| i >= from_frame && e > threshold)
        .collect()
}

fn masked<'a>(signal: &'a [f32], mask: &'a [bool]) -> impl Iterator<Item = f64> + 'a {
    signal
        .chunks(EVAL_FRAME)
        .zip(mask)
        .filter(|(_, &on)| on)
        .flat_map(|(c, _)| c.iter().map(|&x| f64::from(x)))
}

fn erle_db(mic: &[f32], out: &[f32], echo_active: &[bool]) -> f64 {
    let input: f64 = masked(mic, echo_active).map(|x| x * x).sum();
    let output: f64 = masked(out, echo_active).map(|x| x * x).sum();
    db(input / output)
}

/// Scale-invariant SDR over masked frames, and the optimal scale in dB.
fn si_sdr(estimate: &[f32], reference: &[f32], mask: &[bool]) -> (f64, f64) {
    let pairs = || masked(estimate, mask).zip(masked(reference, mask));
    let dot: f64 = pairs().map(|(a, b)| a * b).sum();
    let ref_energy: f64 = pairs().map(|(_, b)| b * b).sum();
    let alpha = dot / ref_energy.max(1e-20);
    let error: f64 = pairs().map(|(a, b)| (a - alpha * b).powi(2)).sum();
    (
        db(alpha * alpha * ref_energy / error.max(1e-20)),
        20.0 * alpha.abs().max(1e-10).log10(),
    )
}

/// Share of near-end-active frames whose output sits more than 10 dB below
/// the near-end component: the "muted genuine speech" check.
fn muted_fraction(out: &[f32], near: &[f32], near_active: &[bool]) -> f64 {
    let out_e = frame_energy_with(out, EVAL_FRAME);
    let near_e = frame_energy_with(near, EVAL_FRAME);
    let considered = near_active.iter().filter(|&&on| on).count();
    if considered == 0 {
        return 0.0;
    }
    let muted = near_active
        .iter()
        .zip(out_e.iter().zip(&near_e))
        .filter(|(&on, (&o, &n))| on && o < n * 0.1)
        .count();
    muted as f64 / considered as f64
}

/// ERLE per one-second block.
fn erle_by_second(mic: &[f32], out: &[f32], echo_active: &[bool]) -> Vec<f64> {
    let frames = EVAL_RATE as usize / EVAL_FRAME;
    mic.chunks(EVAL_RATE as usize)
        .zip(out.chunks(EVAL_RATE as usize))
        .zip(echo_active.chunks(frames))
        .filter(|(_, mask)| mask.iter().any(|&on| on))
        .map(|((m, o), mask)| erle_db(m, o, mask))
        .collect()
}

/// Steady state excludes the first two seconds of adaptation.
const CONVERGENCE_FRAMES: usize = 200;

#[derive(Debug, Default)]
struct Measurement {
    erle_db: Option<f64>,
    erle_by_second: Vec<f64>,
    worst_second_erle_db: Option<f64>,
    near_si_sdr_raw_db: Option<f64>,
    near_si_sdr_out_db: Option<f64>,
    near_gain_db: Option<f64>,
    muted_fraction: Option<f64>,
    /// Muted share split by whether echo is present in the same frame.
    muted_overlap: Option<f64>,
    muted_alone: Option<f64>,
    cpu_ms_per_audio_s: f64,
}

fn measure(s: &Scenario, e: &EvalFixture, run: &Run) -> Measurement {
    let echo_steady = active_frames(&e.echo, CONVERGENCE_FRAMES);
    let echo_all = active_frames(&e.echo, 0);
    let near_from = match s.talk {
        Talk::NearOnly => 0,
        _ => (DOUBLE_TALK_ONSET_S * 100.0) as usize,
    };
    let near_active = active_frames(&e.near, near_from);
    let far_only = s.talk == Talk::FarOnly;
    let has_near = s.talk != Talk::FarOnly && near_active.iter().any(|&on| on);
    let by_second = if far_only {
        erle_by_second(&e.mic, &run.out, &echo_all)
    } else {
        Vec::new()
    };
    let overlap: Vec<bool> = near_active
        .iter()
        .zip(&echo_all)
        .map(|(&n, &x)| n && x)
        .collect();
    let alone: Vec<bool> = near_active
        .iter()
        .zip(&echo_all)
        .map(|(&n, &x)| n && !x)
        .collect();
    let (raw, _) = si_sdr(&e.mic, &e.near, &near_active);
    let (out, gain) = si_sdr(&run.out, &e.near, &near_active);
    Measurement {
        erle_db: far_only.then(|| erle_db(&e.mic, &run.out, &echo_steady)),
        worst_second_erle_db: by_second.iter().skip(2).copied().reduce(f64::min),
        erle_by_second: by_second,
        near_si_sdr_raw_db: has_near.then_some(raw),
        near_si_sdr_out_db: has_near.then_some(out),
        near_gain_db: has_near.then_some(gain),
        muted_fraction: has_near.then(|| muted_fraction(&run.out, &e.near, &near_active)),
        muted_overlap: overlap
            .iter()
            .any(|&on| on)
            .then(|| muted_fraction(&run.out, &e.near, &overlap)),
        muted_alone: alone
            .iter()
            .any(|&on| on)
            .then(|| muted_fraction(&run.out, &e.near, &alone)),
        cpu_ms_per_audio_s: run.processing.as_secs_f64() * 1000.0 / f64::from(s.seconds),
    }
}

fn evaluate(candidate: Candidate, s: &Scenario) -> Measurement {
    let fixture = build_fixture(s);
    let e = eval_fixture(&fixture);
    let r = run(candidate, s, &fixture, &e);
    maybe_write_wavs(s, candidate, &e, &r.out);
    measure(s, &e, &r)
}

// ------------------------------------------------------------------ output --

fn write_wav(path: &std::path::Path, rate: u32, samples: &[f32]) {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &x in samples {
        let v = (x.clamp(-1.0, 1.0) * 32767.0) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("write fixture wav");
}

/// Write evaluation-domain WAVs only when `MURMUR_AEC_EVAL_WAV_DIR` is set.
fn maybe_write_wavs(s: &Scenario, candidate: Candidate, e: &EvalFixture, out: &[f32]) {
    let Some(dir) = std::env::var_os("MURMUR_AEC_EVAL_WAV_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create wav dir");
    let name: String = s
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    for (suffix, signal) in [
        ("render", e.render.as_slice()),
        ("mic", e.mic.as_slice()),
        ("near", e.near.as_slice()),
    ] {
        write_wav(&dir.join(format!("{name}-{suffix}.wav")), EVAL_RATE, signal);
    }
    let tag = format!("{candidate:?}").to_lowercase();
    write_wav(&dir.join(format!("{name}-{tag}.wav")), EVAL_RATE, out);
}

fn fmt(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{v:.1}"))
}

fn report_matrix() -> Vec<Scenario> {
    let s = |name, edit: fn(&mut Scenario)| {
        let mut scenario = BASE;
        scenario.name = name;
        edit(&mut scenario);
        scenario
    };
    vec![
        s("far, moderate (ERL 10 dB)", |_| {}),
        s("far, low (ERL 20 dB)", |x| x.erl_db = 20.0),
        s("far, loud (ERL 0 dB)", |x| x.erl_db = 0.0),
        s("far, loud + speaker clipping", |x| {
            x.erl_db = 0.0;
            x.render_dbfs = -12.0;
            x.clip_drive = 6.0;
        }),
        s("delay 0 ms", |x| x.delay_ms = 0.0),
        s("delay 10 ms", |x| x.delay_ms = 10.0),
        s("delay 100 ms", |x| x.delay_ms = 100.0),
        s("delay 200 ms", |x| x.delay_ms = 200.0),
        s("delay 400 ms", |x| x.delay_ms = 400.0),
        s("delay 600 ms", |x| x.delay_ms = 600.0),
        s("delay 800 ms", |x| x.delay_ms = 800.0),
        s("reference 20 ms late", |x| x.delay_ms = -20.0),
        s("room RT60 0.1 s", |x| x.rt60_s = 0.1),
        s("room RT60 0.5 s", |x| x.rt60_s = 0.5),
        s("room RT60 0.8 s", |x| x.rt60_s = 0.8),
        s("drift 20 ppm, 30 s", |x| {
            x.drift_ppm = 20.0;
            x.seconds = 30.0;
        }),
        s("drift 100 ppm, 30 s", |x| {
            x.drift_ppm = 100.0;
            x.seconds = 30.0;
        }),
        s("drift 1000 ppm, 30 s", |x| {
            x.drift_ppm = 1000.0;
            x.seconds = 30.0;
        }),
        s("noise floor -50 dBFS", |x| x.noise_dbfs = -50.0),
        s("near only, render silent", |x| x.talk = Talk::NearOnly),
        s("near only, noise -50 dBFS", |x| {
            x.talk = Talk::NearOnly;
            x.noise_dbfs = -50.0;
        }),
        s("double-talk, moderate", |x| x.talk = Talk::Double),
        s("double-talk, loud echo (ERL 0)", |x| {
            x.talk = Talk::Double;
            x.erl_db = 0.0;
        }),
        s("double-talk, quiet near (-36 dBFS)", |x| {
            x.talk = Talk::Double;
            x.near_dbfs = -36.0;
        }),
    ]
}

fn print_table(candidate: Candidate, scenarios: &[Scenario]) {
    println!("\n#### {}\n", candidate.label());
    println!(
        "| scenario | ERLE dB | ERLE s1/s2/s3/s4 | worst 1 s ERLE | near SI-SDR raw → out dB | near gain dB | muted: all / with echo / alone | CPU ms / audio s |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for scenario in scenarios {
        let m = evaluate(candidate, scenario);
        let early = m
            .erle_by_second
            .iter()
            .take(4)
            .map(|v| format!("{v:.0}"))
            .collect::<Vec<_>>()
            .join("/");
        let near = match (m.near_si_sdr_raw_db, m.near_si_sdr_out_db) {
            (Some(raw), Some(out)) => format!("{raw:.1} → {out:.1}"),
            _ => "–".to_string(),
        };
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.2} |",
            scenario.name,
            fmt(m.erle_db),
            if early.is_empty() {
                "–".into()
            } else {
                early
            },
            fmt(m.worst_second_erle_db),
            near,
            fmt(m.near_gain_db),
            [m.muted_fraction, m.muted_overlap, m.muted_alone]
                .map(|v| v.map_or_else(|| "–".to_string(), |v| format!("{:.0}%", v * 100.0)))
                .join(" / "),
            m.cpu_ms_per_audio_s,
        );
    }
}

// ------------------------------------------------------------------- tests --

/// A short moderate fixture for the always-on checks.
fn quick(talk: Talk) -> Scenario {
    Scenario {
        name: "quick",
        talk,
        seconds: 5.0,
        ..BASE
    }
}

#[test]
fn fixtures_are_deterministic_and_speech_shaped() {
    let a = build_fixture(&quick(Talk::Double));
    let b = build_fixture(&quick(Talk::Double));
    assert_eq!(a.mic, b.mic);
    assert_eq!(a.render, b.render);
    assert!(a.mic.iter().all(|x| x.is_finite() && x.abs() < 1.0));
    // The talker pauses: some frames are silent, most are not.
    let near = build_fixture(&quick(Talk::NearOnly)).near;
    let active = active_frames(&to_eval(&near), 0);
    let share = active.iter().filter(|&&on| on).count() as f64 / active.len() as f64;
    assert!((0.4..0.95).contains(&share), "near-end activity {share}");
}

#[test]
fn production_path_matches_the_measured_runner_bit_for_bit() {
    let fixture = build_fixture(&quick(Talk::Double));
    let (direct, _) = drive_aec3(SAMPLE_RATE, &fixture.render, &fixture.mic);
    let production = run_production_path(&fixture.render, &fixture.mic);
    assert_eq!(production.len(), fixture.mic.len());
    assert_eq!(production, direct);
}

#[test]
fn aec3_latency_is_fixed_and_small() {
    // Measured: 430 samples (9.0 ms) at 48 kHz, 128 samples (8.0 ms) at 16 kHz.
    assert!(aec3_latency(SAMPLE_RATE) < SAMPLE_RATE as usize / 50);
    assert!(aec3_latency(EVAL_RATE) < EVAL_RATE as usize / 50);
}

#[test]
fn both_aec3_rates_remove_moderate_synthetic_echo() {
    let s = quick(Talk::FarOnly);
    for candidate in [Candidate::Shipped48k, Candidate::Aec3At16k] {
        let erle = evaluate(candidate, &s).erle_db.unwrap();
        // A regression floor for configuration or framing mistakes, not a
        // physical acceptance gate.
        assert!(erle >= 20.0, "{candidate:?} steady-state ERLE {erle:.1} dB");
    }
}

#[test]
fn neither_aec3_rate_mutes_near_end_without_render() {
    let s = quick(Talk::NearOnly);
    for candidate in [Candidate::Shipped48k, Candidate::Aec3At16k] {
        let m = evaluate(candidate, &s);
        assert_eq!(m.muted_fraction, Some(0.0), "{candidate:?}");
        let gain = m.near_gain_db.unwrap();
        assert!(gain.abs() < 1.0, "{candidate:?} near gain {gain:.2} dB");
    }
}

#[test]
fn aec3_at_16k_preserves_near_end_waveform_better_than_shipped_48k() {
    // The spike's main finding, pinned: with no echo at all, the shipped 48 kHz
    // path changes the near-end waveform far more than AEC3 run at the ASR rate.
    let s = quick(Talk::NearOnly);
    let shipped = evaluate(Candidate::Shipped48k, &s)
        .near_si_sdr_out_db
        .unwrap();
    let at_16k = evaluate(Candidate::Aec3At16k, &s)
        .near_si_sdr_out_db
        .unwrap();
    assert!(
        at_16k >= shipped + 10.0,
        "16 kHz {at_16k:.1} dB vs shipped {shipped:.1} dB"
    );
}

#[test]
fn shipped_aec3_ducks_near_end_during_double_talk() {
    // The no-go finding, pinned so a configuration change that fixes it is
    // noticed: once converged, the shipped suppressor pushes a large share of
    // overlapped near-end frames more than 10 dB down. If this starts failing,
    // re-run the report and update docs/features/aec-offline-evaluation.md.
    let s = Scenario {
        seconds: 8.0,
        ..quick(Talk::Double)
    };
    let m = evaluate(Candidate::Shipped48k, &s);
    let overlap = m.muted_overlap.unwrap();
    let alone = m.muted_alone.unwrap();
    assert!(overlap > 0.2, "muted overlapped frames {overlap:.2}");
    assert!(
        alone < overlap / 2.0,
        "alone {alone:.2} vs overlap {overlap:.2}"
    );
}

#[test]
fn aec3_cannot_cancel_a_reference_that_arrives_late() {
    // The worker's callback-clock timeline must keep the render reference
    // ahead of the echo it explains; a late reference is barely cancelled.
    let aligned = evaluate(Candidate::Shipped48k, &quick(Talk::FarOnly))
        .erle_db
        .unwrap();
    let late = evaluate(
        Candidate::Shipped48k,
        &Scenario {
            delay_ms: -20.0,
            ..quick(Talk::FarOnly)
        },
    )
    .erle_db
    .unwrap();
    assert!(
        late + 6.0 < aligned,
        "late {late:.1} dB vs aligned {aligned:.1} dB"
    );
}

#[test]
#[ignore = "full offline matrix; run in release with --ignored --nocapture"]
fn feasibility_report() {
    let profile = if cfg!(debug_assertions) {
        "debug (CPU numbers not representative)"
    } else {
        "release"
    };
    println!("\n### AEC offline feasibility matrix (#594)\n");
    println!(
        "Build: {profile}. Synthetic seed-fixed fixtures generated at 48 kHz; every \
         candidate is scored in the 16 kHz ASR band (0-7 kHz) with its fixed latency \
         removed (AEC3: {} samples @ 48 kHz, {} samples @ 16 kHz). Steady state \
         excludes the first {} s.",
        aec3_latency(SAMPLE_RATE),
        aec3_latency(EVAL_RATE),
        CONVERGENCE_FRAMES / 100
    );
    let matrix = report_matrix();
    print_table(Candidate::Shipped48k, &matrix);
    print_table(Candidate::Aec3At16k, &matrix);
    let linear: Vec<Scenario> = matrix
        .iter()
        .copied()
        .filter(|s| {
            s.name.starts_with("far, moderate")
                || s.name.starts_with("far, loud + speaker")
                || s.name.starts_with("room RT60 0.5")
                || s.name.starts_with("drift 100")
                || s.name.starts_with("double-talk, moderate")
        })
        .collect();
    print_table(Candidate::NlmsOracle16k, &linear);
}
