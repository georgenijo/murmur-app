# Issue 354 public Metal API probe

This is a disposable, standalone probe. It owns its `MTLDevice`, command queue,
command buffers, encoders, and buffers. It does not load Murmur, whisper.cpp,
llama.cpp, FluidAudio, or Core ML.

Accordingly, these results prove behavior only for work and allocations created
by the probe. They do **not** prove that Murmur can observe work or allocations
made through its pinned runtimes.

## Reproduce

Run on macOS with the Xcode command-line tools:

```bash
xcrun swiftc -warnings-as-errors -O -framework Metal \
  metal_metrics_probe.swift \
  -o /tmp/murmur-metal-metrics-probe-354
/tmp/murmur-metal-metrics-probe-354
```

The probe uses only the public Metal framework. It does not request root,
entitlements, private frameworks, or privileged sampling tools.

## Test host

- Hardware: Apple M4, unified memory
- Architecture: arm64
- OS: macOS 26.5.1 (Build 25F80)
- Sample size: 20 warmups, then 300 interleaved dispatches per mode
- Workload: a public Metal compute kernel over 262,144 `UInt32` values

## Repeated results

Three consecutive runs produced:

| Measurement | Run 1 | Run 2 | Run 3 |
| --- | ---: | ---: | ---: |
| Baseline median wall duration (ms) | 0.1913 | 0.1850 | 0.1937 |
| Command-buffer timestamp median wall delta (µs) | 1.31 | 1.27 | 0.75 |
| Command-buffer timestamp median wall overhead | 0.69% | 0.69% | 0.39% |
| `Metal command-buffer GPU elapsed time (ms)`, median | 0.01317 | 0.01294 | 0.01317 |
| Counter sample buffer median wall delta (µs) | 4.31 | 5.48 | 6.71 |
| Counter sample buffer median wall overhead | 2.25% | 2.96% | 3.46% |
| `Metal timestamp counter delta (ticks)`, median | 13,166 | 13,083 | 13,167 |

The host exposed one counter set, `timestamp`, containing only
`GPUTimestamp`. Stage-boundary sampling was supported; draw, dispatch, tile,
and blit boundary sampling were not. Runtime capability checks are therefore
mandatory, and this host supplies no public utilization counter from which an
honest GPU percentage could be calculated.

The measured overhead is a microbenchmark result for a very small synthetic
dispatch. It is not an inference-overhead estimate. Run-to-run scheduler noise
is larger than the command-buffer timestamp delta at the tail, so the useful
conclusion is that both public mechanisms worked for an owned command buffer,
not that they have a universal fixed overhead.

## `currentAllocatedSize`

Each run reported the same values:

| Point | Bytes |
| --- | ---: |
| Before allocation | 1,540,096 |
| After requesting a 64 MiB Metal buffer | 68,648,960 |
| Immediate after release | 1,540,096 |
| After an empty command-buffer drain | 1,540,096 |

The observed increase was exactly 67,108,864 bytes, the requested buffer size.
This proves that `MTLDevice.currentAllocatedSize` tracked an explicit buffer
owned by this process on this host.

It does not prove visibility into allocations made by Murmur's pinned
whisper.cpp runtime or by its separate llama.cpp helper. On Apple Silicon this
is unified-memory Metal resource allocation, not dedicated VRAM, current
resident GPU memory, bandwidth, or utilization. It can overlap conceptually
with process RSS and must not be added to RSS as if the values were disjoint.

## Evidence boundary

- `gpuStartTime` / `gpuEndTime` measure the scheduled GPU interval for a
  command buffer the caller can access. They do not produce utilization.
- Counter sample buffers require access to the encoder or pass descriptor. The
  counters and sampling points available vary by device.
- `currentAllocatedSize` describes resources allocated through a Metal device.
  It does not identify which model or backend owns them.
- The probe cannot cross a process boundary. The local-LLM helper would need to
  measure and report its own data, changing its protocol and signed-helper
  implementation.
- No result in this file establishes access through Murmur's pinned runtimes.
