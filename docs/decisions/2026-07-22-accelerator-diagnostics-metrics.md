# Honest accelerator metrics for Diagnostics

- **Status:** accepted
- **Date:** 2026-07-22
- **Issue:** [#354](https://github.com/georgenijo/murmur-app/issues/354)
- **Parent:** [#350](https://github.com/georgenijo/murmur-app/issues/350)

## Decision

Diagnostics must not display a GPU or ANE utilization percentage. Murmur's
pinned inference runtimes do not expose the command buffers, encoders, or
execution-plan data needed to derive one from public APIs, and Core ML does not
provide a public production API for reporting which compute unit executed an
operation.

The production contract should ship backend identity, end-to-end timing,
throughput where meaningful, correctly scoped memory, and an explicit
unavailable state. Command-buffer timing, Metal counters, Metal allocation
accounting, Metal HUD/capture, and Core ML Instruments remain developer
diagnostics until the relevant pinned runtime exposes an integration seam and a
production rehearsal proves the result.

This spike changes no production UI, telemetry schema, dependency, sidecar
protocol, entitlement, or signing configuration.

## Production backend matrix

Every backend has one disposition and exact user-facing metric names:

| Backend | Exact metrics | Public source | Precision and scope | Availability, overhead, and release impact | Disposition |
| --- | --- | --- | --- | --- | --- |
| Whisper / Metal | `Backend: Whisper / Metal`; `Inference duration (ms)`; `Real-time factor`; `Main-process RSS (MiB)`; `GPU utilization unavailable` | Existing Rust `Instant` timing, decoded audio duration, and `memory-stats` task RSS | Integer-millisecond request wall time; ratio against audio seconds. RSS is the main process and includes non-model memory. No percentage is inferred. | Apple Silicon, macOS 14+; negligible new cost; no entitlement, signing, or dependency change. | **SHIP** |
| Local LLM / Metal sidecar | `Backend: Local LLM / Metal (sidecar)`; `Generation duration (ms)`; `Output throughput (tokens/s)`; `Sidecar RSS (MiB)`; `GPU utilization unavailable` | Existing `Instant` duration and protocol `output_tokens`; public task/process RSS measured by the helper | Integer-millisecond request wall time; token rate derived from returned output-token count. RSS must be measured inside the helper, not attributed to the main process. | Apple Silicon, macOS 14+; timing is already available. Helper RSS needs a reviewed RSS implementation plus protocol, tests, and split-entitlement signing rehearsal; no privilege or private API. | **SHIP** |
| Core ML | `Backend: Core ML / automatic compute-unit selection`; `Inference duration (ms)`; `Real-time factor`; `Main-process RSS (MiB)`; `Accelerator utilization unavailable` | Existing request timing and FluidAudio `processing_time`; public Core ML model configuration; `memory-stats` RSS | Integer-millisecond wall/model timing and ratio against audio seconds. `computeUnits` is an allowed execution set, not proof that ANE or GPU executed an operation. | Apple Silicon, macOS 14+; negligible new sampling cost; no entitlement/signing change or new dependency. | **SHIP** |
| CPU | `Backend: CPU`; `Inference duration (ms)`; `Real-time factor`; `Main-process RSS (MiB)`; `Host CPU utilization (%)` | Existing Rust `Instant`, audio duration, `memory-stats`, and public Mach `host_statistics64` | Integer-millisecond request wall time. The existing CPU value is system-wide, so it must say `Host`, not process. RSS remains main-process scoped. | Murmur's supported macOS 14+ and Linux paths; existing Diagnostics cadence; no new entitlement, signing, or dependency change. | **SHIP** |

`SHIP` is the recommendation for the Diagnostics production follow-up, not an
implementation in this evidence spike. The unavailable labels are part of the
contract: absence must not be rendered as zero. ANE activity must never be
presented as GPU activity.

## Public API and tool matrix

| Candidate | What it can honestly measure | Integration boundary | Hardware / OS and overhead | Entitlements, signing, dependencies | Disposition |
| --- | --- | --- | --- | --- | --- |
| `MTLCommandBuffer.gpuStartTime` / `gpuEndTime` | `Metal command-buffer GPU elapsed time (ms)` for an accessible completed command buffer | The pinned whisper.cpp and llama.cpp runtimes own multiple internal command buffers and expose no callback. Summing buffers could overcount overlap; dividing by wall time would not be whole-device utilization. | Public API compiled against the current deployment target. Standalone M4 probe median wall delta: 0.75–1.31 µs for its owned synthetic dispatch. | No special entitlement in the probe. Production use requires maintained runtime patches; the sidecar also requires protocol and signed-helper changes. | **DEVELOPER-ONLY** |
| Metal counter sample buffers | `Metal timestamp counter delta (ticks)` and only counters the device reports | Requires access to an encoder/pass descriptor. The standalone M4 exposed only `timestamp/GPUTimestamp`, with stage-boundary sampling only. | Hardware-dependent; runtime checks required. Standalone median wall delta: 4.31–6.71 µs. No utilization counter was available on the test host. | No special entitlement in the probe. Production use requires runtime integration and a Metal binding or native bridge. | **DEVELOPER-ONLY** |
| `MTLDevice.currentAllocatedSize` | `Metal resource allocation (MiB)` for resources visible through that process's device | The probe tracked its explicit 64 MiB buffer exactly, but did not prove that either pinned runtime's allocations are observable. It cannot observe the helper from the main process. | Public API; unified-memory accounting is not VRAM, residency, bandwidth, or utilization. Cheap to sample but can overlap conceptually with RSS. | No special entitlement in the probe. Helper reporting would change its protocol and signed binary. | **DEVELOPER-ONLY** |
| Metal Performance HUD, GPU capture, Xcode Instruments | Developer frame/command/counter reports | Appropriate for an engineer-controlled run, not a stable in-app data source. | Availability and counters vary by Xcode, OS, and hardware; capture can materially perturb workloads. | Developer tooling; not a shipped dependency or entitlement. | **DEVELOPER-ONLY** |
| Core ML Instruments | Developer-only operation placement and timing evidence | Profiles a selected run; it is not a production telemetry API and does not authorize labeling normal runs as ANE or GPU. | Requires a compatible Xcode/macOS/hardware combination; profiling overhead is tool-dependent. | Developer tooling; no production integration. | **DEVELOPER-ONLY** |
| Privileged power samplers or private frameworks | Host-level estimated power/activity, not a stable per-Murmur utilization contract | Requires privileges or undocumented interfaces and cannot be constrained honestly to Murmur's command buffers. | Host- and OS-dependent; estimates may be inaccurate. | Root/private API requirements violate the release boundary. | **DO NOT SHIP** |

## Pinned-runtime inspection

The conclusion is specific to the versions Murmur ships:

- `whisper-rs 0.15.1` / `whisper-rs-sys 0.14.1`, vendoring whisper.cpp
  `v1.7.6`. Its public Metal header exposes context/buffer/graph and capture
  controls, but no command-buffer timing or counter callback.
- `llama-cpp-2 0.1.151` / `llama-cpp-sys-2 0.1.151`, whose vendored llama.cpp
  revision is `9e3b928fd8c9d14dbf15a8768b9fdd7e5c721d66`. Its public Metal header likewise
  has no timing/counter callback. It runs in Murmur's separately signed,
  sandboxed helper.
- `fluidaudio-rs` is pinned to
  `2d1083314104c812944b5150866d1e334db8eed7` (FluidAudio 0.14.1). Core ML's
  public model configuration selects permitted compute units; developer
  profiling can inspect a run, but production code cannot report honest
  per-operation ANE/GPU utilization.

A runtime patch would be more than reading a timestamp: ggml can submit several
command buffers, and a valid request-level value needs a defined aggregation
rule, failure behavior, concurrency behavior, and regression coverage. Such a
patch also creates an upstream-rebase obligation. The LLM path additionally
changes the helper protocol and must repeat the split-entitlement signing
rehearsal.

## Memory semantics

The existing `Main-process RSS (MiB)` is derived from `memory-stats` and maps to
task resident memory on macOS. It includes Rust, native runtimes, models, audio,
UI, and shared/mapped pages attributed by the OS. It is not accelerator memory.

`Metal resource allocation (MiB)` would be a different measure: Metal resource
bytes associated with a device in one process. On Apple Silicon, CPU and GPU
share physical memory, so it must not be called VRAM or added to RSS. The LLM
sidecar is a separate process; its RSS and Metal allocation must be measured and
labeled there.

## Evidence and reproducibility

The disposable public-API probe and three-run results are in
[`spikes/354-metal-metrics`](../../spikes/354-metal-metrics/RESULTS.md). The
probe proves timestamp/counter overhead and `currentAllocatedSize` behavior only
for work it owns. It explicitly does not prove access through Murmur's pinned
runtimes.

Primary public evidence:

- Apple:
  [`MTLCommandBuffer.gpuStartTime`](https://developer.apple.com/documentation/metal/mtlcommandbuffer/gpustarttime),
  [counter sample buffers](https://developer.apple.com/documentation/metal/sampling-gpu-data-into-counter-sample-buffers),
  [`MTLDevice.currentAllocatedSize`](https://developer.apple.com/documentation/metal/mtldevice/currentallocatedsize),
  [Metal Performance HUD](https://developer.apple.com/documentation/xcode/generating-performance-reports-with-metal-performance-hud),
  [`MLModelConfiguration`](https://developer.apple.com/documentation/coreml/mlmodelconfiguration),
  and
  [Core ML performance reports](https://developer.apple.com/videos/play/wwdc2022/10027/?time=1040).
- Pinned upstream headers:
  [whisper.cpp v1.7.6 `ggml-metal.h`](https://github.com/ggml-org/whisper.cpp/blob/v1.7.6/ggml/include/ggml-metal.h)
  and
  [llama.cpp pinned `ggml-metal.h`](https://github.com/ggml-org/llama.cpp/blob/9e3b928fd8c9d14dbf15a8768b9fdd7e5c721d66/ggml/include/ggml-metal.h).

## Follow-up boundary

Issue #350 can implement the four fallback rows without inventing accelerator
percentages. This spike does not open another production issue for Metal
timestamps, counters, or allocation: the standalone probe proved the public
APIs themselves, but not access through the pinned runtimes. A future issue is
justified only after an upstream callback or a disposable runtime patch proves
same-process visibility and defines the multi-command-buffer aggregation
contract.
