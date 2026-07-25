# AGENTS.md

Privacy-first macOS voice-to-text app. Tauri 2 (Rust + React). Local transcription on the ANE (Core ML), Metal (whisper.cpp), or CPU (sherpa-onnx); local selected-text rewriting through a signed LLM sidecar. Clipboard-first output. No cloud services.

## Commands

```bash
python3 scripts/build_local_llm_sidecar.py  # Build the macOS local-LLM sidecar FIRST (see note)
cd app && npm run tauri dev        # Dev with hot reload
cd app && npm run tauri build      # Production .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1  # Rust unit tests
cd app && npx tsc --noEmit         # TypeScript check
cd app && npm test                 # Frontend vitest — CI runs this too; tsc alone is not enough
```

> **macOS note:** `tauri.macos.conf.json` declares the `murmur-llm-sidecar` externalBin, so
> on macOS `tauri dev`/`tauri build` fail on a fresh clone until the sidecar binary exists at
> `app/src-tauri/binaries/murmur-llm-sidecar-aarch64-apple-darwin`. Run
> `python3 scripts/build_local_llm_sidecar.py` once first (it is a no-op on non-arm64-macOS).
> The binary is gitignored; release CI builds it before bundling.

## Docs

Start here for orientation:

- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — System structure: module map, data flows, windows, threads, design decisions
- **[docs/FEATURES.md](docs/FEATURES.md)** — What ships, breadth-first, with links into each feature doc
- **[docs/reference/](docs/reference/)** — `commands.md` (105 Tauri commands), `events.md`, `hooks.md`, `settings.md`

Read these before working on a feature:

- **[docs/onboarding.md](docs/onboarding.md)** — Setup, permissions, model installation, logs
- **[docs/features/onboarding-flow.md](docs/features/onboarding-flow.md)** — First-launch setup assistant (permissions wizard + model download)
- **[docs/features/recording-modes.md](docs/features/recording-modes.md)** — Hold-down and double-tap modes, state machine, rdev threading
- **[docs/features/transcription.md](docs/features/transcription.md)** — Audio capture, whisper pipeline, status flow
- **[docs/features/cli-command-formatting.md](docs/features/cli-command-formatting.md)** — Spoken CLI detection, grammar, lexicon, safety
- **[docs/features/smart-formatting.md](docs/features/smart-formatting.md)** — Deterministic prose grammar, backtracking, bounds, privacy
- **[docs/features/text-injection.md](docs/features/text-injection.md)** — Clipboard, auto-paste, osascript
- **[docs/features/vad.md](docs/features/vad.md)** — VAD speech filtering
- **[docs/features/overlay.md](docs/features/overlay.md)** — Dynamic Island overlay
- **[docs/features/log-viewer.md](docs/features/log-viewer.md)** — Structured event system and log viewer
- **[docs/features/auto-updater.md](docs/features/auto-updater.md)** — Auto-update system
- **[docs/features/models.md](docs/features/models.md)** — Model management and download
- **[docs/features/per-app-profiles.md](docs/features/per-app-profiles.md)** — Immutable per-recording context, profile precedence, privacy boundaries
- **[docs/features/ide-context.md](docs/features/ide-context.md)** — Opt-in local IDE index, @file grammar, path/privacy boundaries
- **[docs/features/voice-commands.md](docs/features/voice-commands.md)** — Typed replacements, multiline snippets, safe variables, scopes, and clipboard permission
- **[docs/features/vocabulary-aliases.md](docs/features/vocabulary-aliases.md)** — Exact spoken variants mapped to canonical written terms
- **[docs/features/correct-and-teach.md](docs/features/correct-and-teach.md)** — Bounded learned corrections, exact-term teaching, scope and fail-closed rules
- **[docs/features/personal-knowledge-store.md](docs/features/personal-knowledge-store.md)** — Local SQLite store, migrations, backup/recovery, export/import
- **[docs/features/performance-lab.md](docs/features/performance-lab.md)** — Benchmarking, WER tiers, recommendation contract
- **[docs/features/diagnostic-report-comparison.md](docs/features/diagnostic-report-comparison.md)** — Session-only Reports workspace and comparison
- **[docs/features/selected-text-transform.md](docs/features/selected-text-transform.md)** — Local selected-text rewrite (hold key, sidecar LLM, review popover, approve/undo)
- **[docs/features/evaluation-harness.md](docs/features/evaluation-harness.md)** — Versioned local fixtures, deterministic CI, opt-in hardware evaluation, reports, and deletion
- **[docs/features/performance-diagnostics.md](docs/features/performance-diagnostics.md)** — Versioned local run metrics, retention, correlation, scoped resources, and privacy
- **[docs/decisions/DECISIONS.md](docs/decisions/DECISIONS.md)** — Running log of architectural/scope decisions (newest first)

## File Map

### Rust (`app/src-tauri/src/`)

| File | Purpose |
|------|---------|
| `lib.rs` | App wiring: mod declarations, `State`, `MutexExt`, 105 registered commands, setup, tray, `run()` |
| `commands/mod.rs` | Re-exports command sub-modules |
| `commands/recording.rs` | `IdleGuard`, dictation pipeline, file transcription, vocab scan, IDE context commands |
| `commands/permissions.rs` | Permission check/request/reset and audio device commands (incl. in-app mic TCC prompt) |
| `commands/keyboard.rs` | Dictation + transform listener commands, global disable |
| `commands/logging.rs` | Log commands, delegates to telemetry.rs |
| `commands/models.rs` | Model catalog/status queries and the download pipeline |
| `commands/knowledge.rs` | Personal knowledge store CRUD, resolve, preview, export/import |
| `commands/correct_and_teach.rs` | Bounded correction proposals + confirm/discard |
| `commands/benchmark.rs` | Performance Lab run/cancel/save/reveal |
| `commands/performance.rs` | Local run history, resource window, clear |
| `commands/transform_diagnostics.rs` | Per-pass attempt records and consented content captures |
| `commands/tray.rs` | Tray icon rendering (`make_tray_icon_data`, `update_tray_icon`) |
| `commands/overlay.rs` | Notch detection, `OverlayGeometry` contract (`geometry_for()`), `set_overlay_expanded`, show/hide/show-main-window |
| `commands/native_window.rs` | Shared non-activating window treatment (main-thread dispatched) |
| `commands/transform_model.rs` | Transform LLM model download/status/remove/reset |
| `commands/transform_popover.rs` | Transform review window geometry + show/hide/focusable |
| `keyboard.rs` | Hold-down, double-tap, and transform-hold detectors; shared rdev listener thread |
| `audio.rs` | cpal capture, mono conversion, 16kHz resampling |
| `audio_decode.rs` | Imported audio-file decoding |
| `transcriber/` | `TranscriptionBackend` trait + whisper / parakeet / coreml backends |
| `model_runtime.rs` | Model catalog + serialized load/warm/readiness/unload lifecycle |
| `dictation_context.rs` | Immutable per-recording context snapshot |
| `transcript_transform.rs` | Ordered post-recognition pipeline (cleanup → commands → correction → formatting → IDE → CLI) |
| `cleanup.rs` / `correction.rs` / `cli_command.rs` | Individual transform stages |
| `vocab.rs` / `vocabulary_alias.rs` | Code-vocabulary scanning and explicit spoken aliases |
| `voice_commands.rs` | Typed voice command execution and variable expansion |
| `correct_and_teach.rs` | Bounded local diff proposals; never writes without confirmation |
| `knowledge_store/` | SQLite knowledge store: migrations, repository, backup/quarantine |
| `selection.rs` | AX selection capture for transform (secure-field fail-closed) |
| `transform_apply.rs` | Approve/undo write-back (only path that writes to the target app) |
| `transform_flow.rs` | End-to-end transform orchestrator + Tauri commands |
| `transform_presets.rs` | Built-in spoken transform presets (Shorten/Bullets/…) |
| `transform_diagnostics.rs` / `transform_trace.rs` | Per-pass records, consented captures, pass-scoped correlation |
| `llm_sidecar.rs` | Host supervisor for signed local-LLM helper (no in-process llama) |
| `smart_formatting.rs` | Deterministic prose formatting and same-utterance backtracking |
| `ide_context.rs` | Memory-only bounded IDE symbol and root-relative file index |
| `injector.rs` | Clipboard (arboard) + auto-paste (CGEvent, osascript fallback) |
| `file_output.rs` | Numbered `.txt` / `.wav` output |
| `frontmost.rs` | Frontmost-app query + running-application list |
| `state.rs` | `DictationState`, `TransformStatus`, `AppState`, generation counters |
| `telemetry.rs` | Structured event system: TauriEmitterLayer, ring buffer, JSONL, privacy stripping |
| `vad.rs` | Silero VAD speech filtering via whisper-rs (thread-local context cache) |
| `benchmark.rs` / `evaluation.rs` | Performance Lab scoring and the `murmur-eval` fixture harness |
| `performance_metrics/` | SQLite run history, stage timings, resource samples, retention |
| `resource_monitor.rs` | CPU/RSS sampling, 1s heartbeat, idle-timeout enforcement |
| `alloc.rs` | Custom malloc zone separating Rust heap from whisper.cpp's FFI heap |
| `platform/` | Platform abstraction seams (macOS / Linux) |
| `sidecars/local-llm/` | The signed `murmur-llm-sidecar` crate (llama.cpp) |
| `crates/local-llm-protocol` | Host ↔ sidecar protocol types |

### Frontend (`app/src/`)

| File | Purpose |
|------|---------|
| `App.tsx` | Main orchestrator, wires hooks together |
| `lib/settings.ts` | Settings types, defaults, localStorage persistence |
| `lib/onboarding.ts` | First-launch setup-assistant completion flag |
| `lib/events.ts` | Event types, stream/level definitions, color constants |
| `lib/history.ts` | History entry types and localStorage persistence |
| `lib/stats.ts` | Usage metrics: words, WPM, recordings, tokens |
| `lib/dictation.ts` | Tauri command wrappers for dictation pipeline |
| `lib/updater.ts` | Semver parsing, min-version checking, update utilities |
| `lib/log.ts` | Frontend logging via Rust tracing (flog utility) |
| `lib/hooks/useHoldDownToggle.ts` | Hold-down mode (rdev press/release events) |
| `lib/hooks/useDoubleTapToggle.ts` | Double-tap mode (rdev events) |
| `lib/hooks/useCombinedToggle.ts` | Both mode (hold-down + double-tap simultaneous) |
| `lib/hooks/useRecordingState.ts` | Recording status, transcription, toggle logic |
| `lib/hooks/useAutoUpdater.ts` | OTA updates, min-version enforcement |
| `lib/hooks/useHistoryManagement.ts` | Transcription history with localStorage persistence |
| `lib/hooks/useInitialization.ts` | One-time init sequence (initDictation + configure) |
| `lib/hooks/useShowAboutListener.ts` | Listens for show-about tray event |
| `lib/hooks/useEventStore.ts` | Structured event log buffer with live streaming |
| `lib/hooks/useResourceMonitor.ts` | CPU/memory polling with rolling buffer |
| `lib/hooks/useOverlayGeometry.ts` | Overlay geometry contract from Rust (fetch + `overlay-geometry-changed`) |
| `lib/hooks/useOverlayExpansion.ts` | Overlay hover-expand lifecycle; single writer to the native resize path |
| `lib/hooks/useOverlayRuntime.ts` | Overlay cancelled/hotkey-miss flash timers, `app-disabled-changed` mirror |
| `lib/hooks/useOverlaySettingsMirror.ts` | Overlay's localStorage settings snapshot + quick-control actions |
| `lib/hooks/useRecordingControls.ts` | Overlay click/double-click disambiguation, locked mode |
| `lib/hooks/useWaveform.ts` | Overlay audio-level listener + rAF waveform bar animation |
| `lib/hooks/useTransformFlow.ts` | Main-window transform hold-key driver |
| `lib/hooks/useTransformReviewDriver.ts` | Review popover state + approve/retry/cancel/undo |
| `lib/hooks/useEscapeCancel.ts` | Scoped Escape cancellation carrying the exact transform pass ID |
| `lib/hooks/useSettings.ts` | Settings persistence, backend push, optimistic rollback |
| `lib/hooks/useFileTranscription.ts` | Imported-file transcription + busy state |
| `lib/hooks/useKnowledge.ts` | Bounded paged access to the knowledge store |
| `lib/hooks/useVocabScan.ts` | Live code-vocabulary scan progress, correlated by scan ID |
| `lib/hooks/usePerformanceDiagnostics.ts` | Run history + resource samples (pure `mergeRuns`/`mergeResourceSamples`) |
| `lib/hooks/usePerformanceHealth.ts` | Diagnostics store availability summary |
| `lib/hooks/useOpenSettingsListener.ts` | Opens Settings on the overlay's `open-settings` |
| `lib/hooks/useOverlaySettingsSync.ts` | Applies overlay-originated `settings-changed` in the main window |
| `lib/transformSettings.ts` | Transform model + listener command wrappers |
| `lib/performance.ts` / `lib/performancePresentation.ts` | Run/stage models and presentation |
| `lib/diagnosticReports.ts` / `lib/diagnosticComparison.ts` | Portable report schema validation and comparison |
| `lib/benchmark.ts` | Performance Lab request/report types |
| `lib/transformFlow.ts` | Pure reducer for transform press/release |
| `lib/transformReview.ts` | Review state/error types + content guards |
| `components/onboarding/OnboardingFlow.tsx` | First-launch setup assistant (permissions + model wizard) |
| `components/settings/SettingsPanel.tsx` | Settings UI with mode switching (incl. Transform page) |
| `components/settings/TransformsManager.tsx` | Saved transform CRUD UI |
| `components/transform-review/` | Review popover UI (diff, actions, mock driver) |
| `components/settings/PerformanceLab.tsx` | Benchmark UI, scoring tables, report save/export |
| `components/settings/KnowledgeManager.tsx` | Knowledge store browse/edit UI |
| `components/history/CorrectAndTeachDialog.tsx` | Correct-and-Teach review + scope choice |
| `components/log-viewer/LogViewerApp.tsx` | Log viewer shell: Events, Runs, Transforms, Reports tabs |
| `components/overlay/deriveVisual.ts` | Pure: overlay top-bar indicator + flash-priority derivation |
| `components/overlay/OverlayPill.tsx` | Overlay top bar (presentational) |
| `components/overlay/OverlayDropdown.tsx` | Overlay quick-settings dropdown (presentational) |
| `components/OverlayWidget.tsx` | Dynamic Island overlay composition shell (~150 lines) |

## Key Patterns

- **Recording-mode hooks**: all three always called (Rules of Hooks), gated by the `enabled` prop
- **Clipboard-first**: text always goes to the clipboard; auto-paste is layered on top
- **Generation counters**: `recording_id` and `transform_pass_id` are monotonic; every async continuation re-checks ownership before mutating shared state
- **Immutable per-recording context**: model, delivery, profile, and stage config resolve once at recording start; mid-recording changes apply to the next session
- **Warm-on-record**: `spawn_model_preparation` starts model load when recording starts, so load overlaps with speech
- **Ordered transcript pipeline**: one entry point, declared stage order and failure policy, per-stage timing
- **Fail-closed**: unknown model IDs, ambiguous corrections, and unprovable secure-field checks all refuse rather than guess
- **Rust owns window geometry**: pure `geometry_for()` / `popover_geometry_for()`, fixture-asserted on both sides
- **Main-thread `NSWindow` mutation**: dispatch via `run_on_main_thread` — macOS 26 hard-traps otherwise (#325)
- **Mutex poison recovery**: `MutexExt` trait recovers from panics
- **rdev thread safety**: `set_is_main_thread(false)` before `listen()` — prevents macOS TIS/TSM segfault
- **No in-process llama**: the app crate must never link `llama-cpp-2` (ggml ABI clash with whisper)

## MCP Tools

- **Playwright** (`@playwright/mcp`): Browser automation for UI work. When making frontend/UI changes, use `browser_navigate` to `http://localhost:1420` and `browser_take_screenshot` to visually verify your changes. Requires `npm run tauri dev` to be running. Screenshots return inline as images — evaluate them and iterate until the UI looks right.

## Dependencies

- **Rust**: tauri 2, whisper-rs (Metal), FluidAudio (Core ML), sherpa-onnx, cpal, arboard, hound, rusqlite, core-graphics, objc2/objc2-app-kit, rdev (git main branch)
- **Sidecar**: llama-cpp-2 — in `sidecars/local-llm` only, never in the app crate
- **Frontend**: React 18, Tailwind CSS 4, @tauri-apps/api, Vite 6, TypeScript, vitest
