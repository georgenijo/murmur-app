# AEC feasibility spike

This is a private Stage 0 experiment for Meeting Capture speaker-echo
cancellation. It is not a product feature, does not change the meeting
protocol, and does not run in a normal Murmur build.

The experiment records a consented, paired system-audio reference and raw
microphone capture locally, then runs WebRTC AEC3 offline. The result tells us
whether an external CATap reference can remove speaker echo without harming
the speaker at the microphone. Until those measurements pass, Murmur continues
to ship its existing raw microphone path.

## Privacy boundary

- The private `aec-spike` Cargo feature is off by default. Production and
  release helper builds do not recognize its commands.
- A session is written only after the exact consent value
  `I_UNDERSTAND_THIS_WRITES_LOCAL_AUDIO` is supplied.
- Artifacts never use the meeting protocol, SQLite/history, telemetry, log
  shipper, Murmur Bench, CI artifacts, or Fleet artifacts.
- Use a directory outside the repository and outside Murmur's normal app-data
  directory, for example `~/Library/Application Support/Murmur AEC Spike/v1`.
  Delete a session directory to delete its raw audio, timing data, and reports.
- `manifest.json` records only a device-class enum, hashes, rates/counts,
  explicit consent, and local-only/network-false assertions. It never records
  a device name, UID, transcript, or audio in JSON.

## Build and capture

Install the local bundled-build prerequisites once: `meson`, `ninja`, and
`pkg-config` (or `pkgconf`). Then build the isolated helper:

```bash
python3 scripts/build_aec_spike.py
```

Capture a short, consented fixture while playing representative far-end audio
through speakers and also speaking over it:

```bash
app/src-tauri/target/aec-spike/debug/murmur-capture-helper \
  --aec-spike-capture \
  --output-root "$HOME/Library/Application Support/Murmur AEC Spike/v1" \
  --duration-seconds 30 \
  --consent I_UNDERSTAND_THIS_WRITES_LOCAL_AUDIO
```

The tool currently requires both the system tap and microphone client format
to be 48 kHz. It fails closed for another tap rate; it does not resample,
discard timing, or publish a misleading fixture. The resulting session has:

```text
session-<epoch>-<pid>/
  manifest.json
  render.wav
  microphone.wav
  timing.bin
```

`timing.bin` is a compact numeric fixed-record sidecar. Each record anchors a
worker drain position to the latest callback's Core Audio host-time/sample-time
pair. It is deliberately described as an anchor, not as a false claim of
per-sample timing precision.

## Offline analysis

Run AEC3 on the paired WAVs and keep results in the same local session:

```bash
app/src-tauri/target/aec-spike/debug/murmur-capture-helper \
  --aec-spike-analyze \
  --render "/absolute/path/session-.../render.wav" \
  --microphone "/absolute/path/session-.../microphone.wav" \
  --timing "/absolute/path/session-.../timing.bin" \
  --output "/absolute/path/session-.../cleaned.wav" \
  --report "/absolute/path/session-.../report.json" \
  --consent I_UNDERSTAND_THIS_WRITES_LOCAL_AUDIO
```

AEC3 works in 10 ms mono 48 kHz frames. The analyzer saves an atomic cleaned
WAV plus a local-only report with file hashes, median/p10 and global
energy-based ERLE, AEC delay percentiles, timing-sidecar drift and
discontinuity counts, and p50/p95/max frame-processing time. `Them` is only
the AEC reference in this tool; its source WAV is never modified.

When `--timing` is present, the analyzer derives a bounded coarse initial
render offset from callback anchors before handing fine delay tracking to
AEC3. Without it, it preserves the two WAVs' existing sample-zero alignment;
do not use that fallback for a captured-room decision.

## Evidence gate

Run scenarios separately: far-end-only at low/medium/loud volume, near-end
only, genuine double-talk, and built-in speakers/mic before trying external or
Bluetooth devices. Assess ERLE per far-end-only active window, delay/drift from
the timing sidecar, clipping/gaps, frame CPU/RSS, and listening/ASR comparison
against a headphone or close-talk baseline.

Do not treat aggregate WER or a global ERLE number as a pass: near-end loss in
double-talk is the hard failure. SI-SDR requires a known clean near-end source;
an ordinary room recording cannot supply that ground truth. No realtime
protocol, UI, or default setting should be added unless the Stage 0 fixtures
demonstrate repeatable echo reduction without unacceptable near-end damage.
