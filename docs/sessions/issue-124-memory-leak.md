# Issue #124 — Rust Heap Memory Leak (~60-70 MB/min at Idle)

## Overview

The app leaked ~60-70 MB/min of Rust heap memory while completely idle (no recording, no transcription). RSS grew unboundedly until the process was killed.

**Root cause**: The cpal CoreAudio audio callback continued running after `stop_recording()` returned, pushing samples into a `Vec<f32>` buffer that nobody consumed. The Vec doubled its capacity on each growth, producing the characteristic exponential heap expansion.

---

## Evidence

Zone telemetry from heartbeat logs showed Rust heap climbing steadily at idle:

| Time | RSS (MB) | Rust Heap (MB) | FFI Heap (MB) |
|------|----------|-----------------|----------------|
| +0 min | ~180 | ~8 | ~170 |
| +1 min | ~240 | ~68 | ~170 |
| +2 min | ~310 | ~138 | ~170 |
| +5 min | ~520 | ~348 | ~170 |

FFI heap (whisper.cpp, sherpa-onnx) stayed flat. The Rust heap was the sole contributor to RSS growth — growing ~60-70 MB per minute at idle.

---

## Session 1: Binary search isolation

Systematically disabled suspected subsystems to isolate the leak source.

### Test 1: Bypass sysinfo CPU refresh

Skipped `sys.refresh_cpu_usage()` in `get_resource_usage()`.

**Result**: Leak rate dropped by roughly half (~30-35 MB/min → still leaking). Confirmed sysinfo was a partial contributor but not the sole cause.

### Test 2: Hardcode get_resource_usage

Replaced the entire `get_resource_usage()` body with hardcoded values (no sysinfo calls at all).

**Result**: Leak continued at the reduced rate. Something else was still allocating.

### Test 3: Disable tracing heartbeat emission

Stopped the 60-second heartbeat from emitting tracing events.

**Result**: Leak continued. Tracing/event emission was not the source.

### Test 4: Zone diagnostics — block count analysis

Added `malloc_zone_statistics()` logging to the heartbeat to track allocation block counts alongside size.

**Result**: Block count stayed stable (not growing). But `size_in_use` kept doubling — meaning a single allocation was being reallocated to ever-larger sizes. This is the signature of a `Vec` that keeps getting `extend()`ed without ever being drained.

### Test 5: Disable heartbeat entirely

Commented out the entire heartbeat spawned task.

**Result**: Heap was stable at idle... until a transcription was triggered. After one record/stop cycle, the leak resumed at ~60-70 MB/min even with no further interaction.

**Key insight**: The leak was triggered by recording, not by the heartbeat. The heartbeat was an innocent bystander (except for sysinfo's own allocations).

---

## Session 2: Identifying the doubling allocation

Used macOS memory tools to narrow down the leak:

- **`heap <pid>`**: Showed a massive `MALLOC_TINY`/`MALLOC_SMALL` region under the RustHeapZone growing over time, consistent with Vec reallocation.
- **`leaks <pid>`**: Did not flag the buffer as a leak because it was still reachable — the `Arc<Mutex<Vec<f32>>>` in `RecordingState.shared` held a live reference. This is a logical leak (unbounded growth of a reachable object), not a classical dangling-pointer leak.

---

## Session 3: Allocator instrumentation (alloc-spy)

### Technique

Overrode `GlobalAlloc::realloc` in the custom `RustZoneAllocator` to intercept large reallocations:

```rust
unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    if new_size > 1_000_000 {
        // Log to stderr using raw write() (async-signal-safe, no allocation)
        backtrace_symbols_fd(...);
    }
    malloc_zone_realloc(rust_zone(), ptr as *mut c_void, new_size) as *mut u8
}
```

Key details:
- Threshold at 1 MB to filter out noise
- Used `backtrace_symbols_fd()` + raw `write()` to stderr — these are async-signal-safe and don't allocate, so they won't recurse into the allocator
- Ran with `RUST_BACKTRACE=1` for symbol resolution

### Result

Every single large-realloc backtrace showed the same stack:

```
CoreAudio HALC_IOThread
  → cpal input_data_callback
    → audio::build_mono_input_stream closure
      → Vec<f32>::extend
```

The cpal audio stream's CoreAudio callback was still firing on the HAL IO thread, pushing samples into the shared `Vec<f32>`, long after `stop_recording()` had returned. Nobody was draining the buffer, so it grew without bound.

---

## Root cause

`stop_recording()` sent `AudioCommand::Stop` through the channel and joined the audio thread, but the **cpal stream was not explicitly stopped**. On macOS, cpal's CoreAudio backend continues calling the input callback on its own HAL IO thread even after the `Stream` object's owning thread has exited. The stream only stops when it is dropped — but the drop was racing with the thread join, and in practice the callback kept firing.

The callback closure captured an `Arc<Mutex<Vec<f32>>>` and called `extend()` on every CoreAudio buffer delivery (~every 5-10ms). With no consumer, the Vec doubled its allocation on each capacity exhaustion, producing ~60-70 MB/min of Rust heap growth.

---

## Fix (commit 929a4ef)

### Core fix: AtomicBool active flag

```rust
// In audio.rs — shared between recording state and cpal callback
active: Arc<AtomicBool>

// In the cpal callback (build_mono_input_stream macro):
if !active_ref.load(Ordering::Relaxed) {
    return;  // Stop accumulating immediately
}

// In stop_recording() — set BEFORE sending Stop command:
state_guard.active.store(false, Ordering::SeqCst);
```

The `AtomicBool` gives instant, lock-free signaling to the CoreAudio HAL IO thread. Setting it `false` before sending `AudioCommand::Stop` ensures zero buffer growth during teardown.

### Explicit stream.pause()

```rust
// At end of run_audio_capture, before stream drops:
let _ = stream.pause();
```

Tells CoreAudio to stop delivering audio buffers before the stream is dropped.

### mem::take instead of clone

```rust
// Old: copied the entire buffer (potentially hundreds of MB)
guard.clone()

// New: moves the buffer out, leaving an empty Vec
std::mem::take(&mut *guard)
```

### sysinfo removal

Replaced the `sysinfo` crate (which contributed ~half the leak via `refresh_cpu_usage()`) with direct macOS `host_statistics64()` FFI calls. This also removed transitive dependencies (rayon, crossbeam-deque, ntapi).

### malloc_zone_realloc

Added `realloc` override to `RustZoneAllocator` so Vec growth can happen in-place when the zone has room, instead of the default alloc+copy+free path.

### Heartbeat refactor

Moved the heartbeat spawned task and idle timeout checker from `lib.rs` into `resource_monitor.rs` as `start_heartbeat()`.

---

## Key findings

| What was tested | Result | Conclusion |
|----------------|--------|------------|
| Bypass sysinfo refresh | Leak halved | sysinfo was a partial contributor |
| Hardcode resource usage | Still leaking | Another source existed |
| Disable tracing heartbeat | Still leaking | Tracing was not the cause |
| Zone block count analysis | Blocks stable, size doubling | Single Vec growing unboundedly |
| Disable heartbeat entirely | Stable until first recording | Leak triggered by record/stop cycle |
| alloc-spy backtrace | CoreAudio → cpal → Vec::extend | cpal callback was the sole remaining source |

---

## Reusable debugging technique: alloc-spy

For future Rust heap leak investigations on macOS:

1. **Custom `GlobalAlloc::realloc`** — override realloc in your zone allocator to intercept large reallocations (threshold at 1 MB+)
2. **`backtrace_symbols_fd` + `write()`** — async-signal-safe backtrace capture that doesn't recurse into the allocator
3. **Filter by size** — most noise is small allocations; large reallocs point directly to the unbounded buffer
4. **Zone statistics** — `malloc_zone_statistics()` gives block count vs size-in-use; stable blocks + growing size = single allocation doubling (Vec pattern)

This technique identified the exact callsite in one iteration after multiple hours of indirect binary-search debugging.

---

## Key files

| File | Change |
|------|--------|
| `app/src-tauri/src/audio.rs` | AtomicBool active flag in cpal callback, stream.pause(), mem::take |
| `app/src-tauri/src/resource_monitor.rs` | macOS-native CPU via host_statistics64(), heartbeat + idle timeout moved here |
| `app/src-tauri/src/alloc.rs` | malloc_zone_realloc override for in-place Vec growth |
| `app/src-tauri/src/lib.rs` | Removed inline heartbeat task, calls resource_monitor::start_heartbeat() |
| `app/src-tauri/Cargo.toml` | Removed sysinfo dependency |

## PR

https://github.com/georgenijo/murmur-app/pull/125
