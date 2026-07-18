# Platform abstraction spike

Issue: #148
Audit base: `a952c406841d902b576dd2a2cad4c0ba22927469`

## Recommendation

Use small, compile-time-selected platform modules for behavior that has the same
application-level purpose but different OS implementations. Keep target-specific
dependencies, accelerator support, and build artifacts in Cargo target tables or
at module boundaries. Do not use runtime OS detection: Murmur ships separate
native binaries, and compile-time selection prevents unsupported APIs and
dependencies from entering the wrong build.

A single broad `Platform` trait is not recommended yet. It would group unrelated
concerns (paste, permissions, overlays, CPU sampling) into one interface and make
tests depend on a large mock. Prefer narrow functions or narrow traits grouped by
capability, exposed through `platform/mod.rs`.

## Audit

The audit base contained 102 lines with target OS/architecture selection across
Rust and Cargo (counting `cfg`, `cfg_attr`, `cfg!`, and Cargo target tables). They
fall into two categories:

| Category | Current areas | Direction |
| --- | --- | --- |
| Platform behavior | Paste/focused-field handling (`injector.rs`), permissions, frontmost app, rdev thread setup, CPU sampling, notch/overlay behavior, Linux WebKit setup, macOS reopen handling, heap metrics | Move cohesive behaviors behind narrow APIs in `platform/`, one area at a time |
| Build/dependency or capability configuration | Cargo target dependencies for Metal/CUDA/X11/Core ML/AppKit, Core ML module and integration-test inclusion, accelerator-specific model paths, allocator inclusion, target-specific examples | Keep as compile-time Cargo/module gates; optionally centralize repeated capability labels/constants, but do not hide dependency selection behind runtime code |

The detailed routing is:

| Area | Classification | Reason |
| --- | --- | --- |
| `injector.rs`, `frontmost.rs` | Behavior | Same product operations need different native focus and keystroke mechanisms |
| `commands/permissions.rs` | Behavior | Permission status, prompts, reset, and settings destinations are OS services |
| `commands/overlay.rs` | Behavior | Notch detection and native window operations are macOS behavior; the Linux no-op contract is intentional |
| `keyboard.rs` | Behavior | Listener-thread initialization differs because of macOS TIS/TSM requirements |
| `resource_monitor.rs` | Behavior | CPU sampling is Mach on macOS and `/proc/stat` on Linux |
| `lib.rs` | Mixed | Linux WebKit setup and macOS reopen handling are behavior; allocator and OS-only Tauri types are compile boundaries |
| `benchmark.rs`, `commands/models.rs`, `commands/recording.rs`, `transcriber/` | Mostly build/capability | Core ML and accelerator code is not available on every target; capability labels may later use narrow constants |
| `Cargo.toml`, integration tests, examples | Build/dependency | Target dependencies and target-only code must be excluded before compilation |

Migrate by behavior, not by file or by raw gate count.

## Proof of concept

CPU sampling is the first extracted behavior:

- `platform/mod.rs` selects exactly one implementation using `cfg_attr(path)`.
- `platform/macos.rs` keeps the Mach `host_statistics64` sampler.
- `platform/linux.rs` keeps `/proc/stat` sampling.
- `platform/unsupported.rs` preserves the existing zero-value fallback without
  expanding Windows support.
- `resource_monitor.rs` now calls the stable `platform::cpu_percent()` API and
  contains no target gate for CPU sampling.

The shared tick-delta calculation is unit-tested on every target. Linux parsing
has target-native unit tests. The public Tauri command and resource payload are
unchanged.

## Migration path

1. Land one capability per change, preserving its existing public API and tests.
2. Extract paste simulation next, but keep clipboard ownership and focus-safety
   policy in `injector.rs`; only the OS keystroke mechanism belongs in `platform/`.
3. Extract keyboard listener setup and frontmost-app lookup as narrow functions.
4. Move permissions and overlay behavior only when their non-macOS contracts are
   explicit; their current no-op/error fallbacks are product decisions, not just
   implementation details.
5. Leave Cargo dependency tables and Core ML/Metal/CUDA compilation gates in
   place. Re-audit after each extraction and stop when remaining gates describe
   build capabilities rather than application behavior.

This path keeps the refactor reviewable, preserves macOS and Linux behavior, and
maintains Murmur's local-only privacy boundary.
