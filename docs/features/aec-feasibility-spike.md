# AEC feasibility spike

This is the private Stage 0 evidence tool for Meeting Capture speaker-echo
cancellation. The tool does not run in a normal Murmur build and its capture
and analysis commands are not part of the production protocol.

The experiment records a consented, paired system-audio reference and raw
microphone capture locally, then runs WebRTC AEC3 offline. The result helps
measure whether an external CATap reference can remove speaker echo without
harming the speaker at the microphone. Murmur now has a separate experimental,
default-off realtime AEC3 path in the signed capture worker. That integration
does not prove the acoustic acceptance gates below. The realtime setting must
remain opt-in until representative physical fixtures pass them.

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

`--duration-seconds` accepts 1 through 1800 seconds. The 30-minute maximum
supports one continuous long-session drift run while keeping capture bounded.

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
an ordinary room recording cannot supply that ground truth. Do not enable AEC
by default or remove its experimental label unless the Stage 0 fixtures show
repeatable echo reduction without unacceptable near-end damage. The current
opt-in realtime path is implementation evidence, not acoustic acceptance
evidence.

## Repeatable acceptance run

Use one immutable commit for the helper, production app, and receipt. Start
with the generated-fixture tests, which exercise the analyzer without reading
private audio:

```bash
cd app/src-tauri
cargo test -p murmur-capture-helper --features aec-spike -- --test-threads=1
```

That test is only a tool preflight. Synthetic echo cannot establish speaker,
room, microphone, or double-talk quality.

Run the physical matrix below with short, consented local fixtures. Use the
same playback, microphone placement, volume, and spoken script for both the AEC
off and AEC on runs. Keep a headphone or close-talk recording as the clean ASR
baseline. SI-SDR requires a known, time-aligned clean near-end source.

| Route | Required scenarios |
| --- | --- |
| Built-in speakers and microphone | far-end only at low, moderate, and loud volume; near-end only; double-talk; clipping |
| USB microphone with built-in speakers | far-end only; near-end only; double-talk |
| Display output with the selected microphone | far-end only; near-end only; double-talk |
| Bluetooth input and output | far-end only; near-end only; double-talk; rate or route change |
| Headphones | near-end and double-talk baselines; confirm no near-end damage |

For at least one moderate-volume built-in run, continue for 30 minutes and
record the start, middle, and end windows separately. Use shorter fault runs to
switch the input and output, insert silence, restart playback, and create a
bounded CPU-load spike. Confirm that recovery reports a truthful state. The
generated-fixture tests must continue to prove that bypass emits every
microphone sample once.

For each fixture, retain this local receipt next to the WAVs and report:

- Record the exact commit and device-class label. Do not record a device UID or
  user content.
- Report far-end-only ERLE for active windows. Apply the 18 dB gate to each
  representative moderate-volume fixture.
- Report near-end-only SI-SDR loss against the aligned clean source. The maximum
  loss is 1 dB.
- Report local ASR WER for double-talk against the clean baseline. The maximum
  relative regression is 5%.
- Report AEC delay, timing discontinuities, and start-to-end clock drift.
- Report the offline analyzer's p50, p95, and maximum frame-processing time.
  Wrap the analyzer with `/usr/bin/time -l` to capture its maximum resident set
  size. These values measure the offline tool, which holds the input and output
  WAV samples in memory. They do not measure realtime worker cost.
- Measure the signed `murmur-capture-worker` separately during matched AEC off
  and AEC on meetings. Sample its CPU and RSS once per second, and report peak
  RSS, CPU percentiles, backlog transitions, and recovery transitions. Do not
  include process arguments or audio content in the resource log.
- Report the signed capture-worker size before and after AEC. Record whether
  the release build or signing inputs changed.
- Record listening notes for near-end deletion, pumping, clipping, or
  independently duplicated Me text.

The decision receipt may contain only metrics, hashes, device classes, and a
pass or fail result. Raw audio and transcripts stay on the trusted Mac. Any
failed near-end or double-talk run blocks default enablement. A missing route
or metric leaves the issue open rather than counting as a pass.
