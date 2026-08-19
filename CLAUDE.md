# CLAUDE.md

Privacy-first macOS voice-to-text app. Tauri 2 (Rust + React). Local transcription on the ANE (Core ML), Metal (whisper.cpp), or CPU (sherpa-onnx); local selected-text rewriting through a signed LLM sidecar. Clipboard-first output. No cloud services.

## Commands

```bash
python3 scripts/build_local_llm_sidecar.py  # Build the bundled macOS helpers FIRST (see note)
cd app && npm run tauri:dev        # Dev with hot reload and isolated dev bundle/data
cd app && npm run tauri build      # Production .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1  # Rust unit tests
cd app && npx tsc --noEmit         # TypeScript check
cd app && npm test                 # Frontend vitest — CI runs this too; tsc alone is not enough
PR_HEAD_SHA="$(gh pr view --json headRefOid --jq .headRefOid)"
python3 scripts/murmur_bench_fleet.py --baseline origin/main --candidate "$PR_HEAD_SHA" --preset quick  # Trusted-Mac PR performance gate
```

> **macOS note:** `tauri.macos.conf.json` declares the local-LLM and capture helpers as externalBin, so
> on macOS `tauri dev`/`tauri build` fail on a fresh clone until the sidecar binary exists at
> `app/src-tauri/binaries/murmur-llm-sidecar-aarch64-apple-darwin`. Run
> `python3 scripts/build_local_llm_sidecar.py` once first (it is a no-op on non-arm64-macOS).
> The binaries are gitignored; release CI builds them before bundling.

## Murmur Bench Gate

Murmur Bench is the private, repeatable personal-corpus harness documented in
`docs/features/internal-performance-harness.md`. Raw reports can contain
reference and recognized transcript text: keep them on the trusted benchmark
Mac and put only a content-free metric summary in GitHub.

- Before merging a PR that can change recognition latency, accuracy,
  delivered-text output, or memory, resolve the pushed PR head with
  `gh pr view --json headRefOid --jq .headRefOid`, verify the trusted benchmark
  Mac can resolve that commit after fetching `origin`, and run
  `scripts/murmur_bench_fleet.py` against `origin/main` and that immutable SHA.
  This includes changes to
  VAD, transcription backends, model runtime, transcript transforms, benchmarked
  execution paths, or performance-sensitive Rust dependencies.
- Use `quick` for the normal PR gate and `standard` for shared cross-model or
  pipeline changes. Record an explicit `Murmur Bench: N/A — <reason>` for PRs
  that cannot affect benchmarked behavior.
- Record the exact baseline ref, candidate SHA, preset, models, thresholds,
  aggregate deltas, and pass/fail. Any later push, rebase, merge from
  main, or conflict resolution invalidates the result and requires a rerun.
- Before every release, compare the previous release tag with `origin/main` on
  `standard`. Use `thorough` when the release contains any benchmark-sensitive
  change.
- Never use `--no-fail` to satisfy a merge or release gate. If a comparison
  fails, rerun once with `--candidate-first` to expose order/thermal bias. A
  repeated regression blocks the operation; mixed results are inconclusive and
  also require investigation or explicit user acceptance before continuing.
- Murmur Bench replays saved WAV files. It does not replace native capture
  smoke tests or the post-release production check for Core Audio startup,
  device switching, first PCM, clipboard, or paste behavior.

## Docs

Start here for orientation:

- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — System structure: module map, data flows, windows, threads, design decisions
- **[docs/FEATURES.md](docs/FEATURES.md)** — What ships, breadth-first, with links into each feature doc
- **[docs/reference/](docs/reference/)** — `commands.md` (176 Tauri commands), `events.md`, `hooks.md`, `settings.md`

Read these before working on a feature:

- **[docs/onboarding.md](docs/onboarding.md)** — Setup, permissions, model installation, logs
- **[docs/features/onboarding-flow.md](docs/features/onboarding-flow.md)** — First-launch setup assistant (permissions wizard + model download)
- **[docs/features/history-workspace.md](docs/features/history-workspace.md)** — Transcript search, filters, and export
- **[docs/features/command-palette.md](docs/features/command-palette.md)** — ⌘K launcher and main-window shortcuts
- **[docs/features/silence-auto-stop.md](docs/features/silence-auto-stop.md)** — Hands-free trailing-silence finish for recordings not started by holding the key
- **[docs/features/recording-modes.md](docs/features/recording-modes.md)** — Hold-down and double-tap modes, state machine, rdev threading
- **[docs/features/transcription.md](docs/features/transcription.md)** — Audio capture, whisper pipeline, status flow
- **[docs/features/microphone-input-test.md](docs/features/microphone-input-test.md)** — Capture-only Settings meter, device switching, privacy and ownership
- **[docs/features/cli-command-formatting.md](docs/features/cli-command-formatting.md)** — Spoken CLI detection, grammar, lexicon, safety
- **[docs/features/smart-formatting.md](docs/features/smart-formatting.md)** — Deterministic prose grammar, backtracking, bounds, privacy
- **[docs/features/text-injection.md](docs/features/text-injection.md)** — Clipboard, auto-paste, osascript
- **[docs/features/vad.md](docs/features/vad.md)** — VAD speech filtering
- **[docs/features/overlay.md](docs/features/overlay.md)** — Dynamic Island overlay
- **[docs/features/log-viewer.md](docs/features/log-viewer.md)** — Structured events and the embedded Performance workspace
- **[docs/features/log-shipping.md](docs/features/log-shipping.md)** — Zero-config diagnostic log upload to central receiver
- **[docs/features/auto-updater.md](docs/features/auto-updater.md)** — Auto-update system
- **[docs/features/models.md](docs/features/models.md)** — Model management and download
- **[docs/features/per-app-profiles.md](docs/features/per-app-profiles.md)** — Immutable per-recording context, profile precedence, privacy boundaries
- **[docs/features/ide-context.md](docs/features/ide-context.md)** — Opt-in local IDE index, @file grammar, path/privacy boundaries
- **[docs/features/voice-commands.md](docs/features/voice-commands.md)** — Typed replacements, multiline snippets, safe variables, scopes, and clipboard permission
- **[docs/features/vocabulary-aliases.md](docs/features/vocabulary-aliases.md)** — Exact spoken variants mapped to canonical written terms
- **[docs/features/correct-and-teach.md](docs/features/correct-and-teach.md)** — Bounded learned corrections, exact-term teaching, scope and fail-closed rules
- **[docs/features/personal-knowledge-store.md](docs/features/personal-knowledge-store.md)** — Local SQLite store, migrations, backup/recovery, export/import
- **[docs/features/performance-lab.md](docs/features/performance-lab.md)** — Benchmarking, WER tiers, recommendation contract
- **[docs/features/internal-performance-harness.md](docs/features/internal-performance-harness.md)** — Private personal-corpus build, Fleet runner, and mandatory PR/release performance gates
- **[docs/features/diagnostic-report-comparison.md](docs/features/diagnostic-report-comparison.md)** — Session-only Reports workspace and comparison
- **[docs/features/selected-text-transform.md](docs/features/selected-text-transform.md)** — Local selected-text rewrite (hold key, sidecar LLM, review popover, approve/undo)
- **[docs/features/evaluation-harness.md](docs/features/evaluation-harness.md)** — Versioned local fixtures, deterministic CI, opt-in hardware evaluation, reports, and deletion
- **[docs/features/performance-diagnostics.md](docs/features/performance-diagnostics.md)** — Versioned local run metrics, retention, correlation, scoped resources, and privacy
- **[docs/decisions/DECISIONS.md](docs/decisions/DECISIONS.md)** — Running log of architectural/scope decisions (newest first)

## File Map

### Rust (`app/src-tauri/src/`)

| File | Purpose |
|------|---------|
| `lib.rs` | App wiring: mod declarations, `State`, `MutexExt`, 176 registered commands, setup, tray, `run()` |
| `commands/mod.rs` | Re-exports command sub-modules |
| `commands/integrations.rs` | Local availability probes for optional companion apps |
| `commands/recording.rs` | `IdleGuard`, dictation pipeline, file transcription, vocab scan, IDE context commands |
| `commands/permissions.rs` | Permission check/request/reset and audio device commands (incl. in-app mic TCC prompt) |
| `commands/microphone_preview.rs` | Main-window microphone test commands, lifecycle bridge, exact-owner teardown |
| `commands/keyboard.rs` | Dictation + transform listener commands, global disable |
| `commands/export.rs` | `save_text_export` — validated, atomic user-chosen text export sink |
| `commands/settings_store.rs` | Durable `settings.json`, `history.json`, `stats.json`, and main-only `theme-library.json`: bounded opaque blobs, atomic write, clear, corrupt-file quarantine |
| `commands/logging.rs` | Log commands, delegates to telemetry.rs |
| `commands/models.rs` | Model catalog/status queries and the download pipeline |
| `commands/knowledge.rs` | Personal knowledge store CRUD, resolve, preview, export/import |
| `commands/correct_and_teach.rs` | Bounded correction proposals + confirm/discard |
| `commands/benchmark.rs` | Performance Lab run/cancel/save/reveal |
| `commands/microphone_startup_benchmark.rs` | Exact-owner production microphone startup cycles, progress, cancellation, and typed report export |
| `commands/performance.rs` | Local run history, resource window, clear |
| `commands/theme.rs` | Main-window-gated, bounded theme-file import/export transport |
| `commands/transform_diagnostics.rs` | Per-pass attempt records and consented content captures |
| `commands/tray.rs` | Tray icon rendering plus the update-check item, version label, and wake observer |
| `commands/overlay.rs` | Notch detection, `OverlayGeometry` contract (`geometry_for()`), `set_overlay_expanded`, show/hide/show-main-window |
| `commands/native_window.rs` | Shared non-activating window treatment (main-thread dispatched) |
| `commands/transform_model.rs` | Transform LLM model download/status/remove/reset |
| `commands/transform_popover.rs` | Transform review window geometry + show/hide/focusable |
| `keyboard.rs` | Hold-down, double-tap, and transform-hold detectors; shared rdev listener thread |
| `audio.rs` / `audio_lifecycle.rs` | cpal capture plus the single-owner async initialization supervisor, cancellation/recovery, join ownership, preview level routing, mono conversion, and 16kHz resampling |
| `audio_inventory.rs` | Shared versioned microphone inventory, passive topology invalidation, coalesced idle-only refresh, stale-cache policy, and privacy-safe aggregate |
| `audio_decode.rs` | Imported audio-file decoding |
| `capture_agent_probe.rs` / `capture_helper_probe.rs` | Signed helper registration, callback-health probes, cancellation, and confirmed-termination evidence |
| `code_signing.rs` / `managed_child.rs` | Runtime helper identity validation and direct-child/process-group ownership |
| `transcriber/` (`whisper.rs`, `parakeet.rs`, `coreml.rs`) | `TranscriptionBackend` trait and backend implementations |
| `model_runtime.rs` | Model catalog + serialized load/warm/readiness/unload lifecycle |
| `microphone_preview.rs` | Capture-only preview coordinator, signal aggregation, stable quiet/clipping classification |
| `dictation_context.rs` | Immutable per-recording context snapshot |
| `transcript_transform.rs` | Ordered post-recognition pipeline (cleanup → commands → correction → formatting → IDE → CLI) |
| `cleanup.rs` / `correction.rs` / `cli_command.rs` | Individual transform stages |
| `vocab.rs` / `vocabulary_alias.rs` | Code-vocabulary scanning and explicit spoken aliases |
| `voice_commands.rs` | Typed voice command execution and variable expansion |
| `correct_and_teach.rs` | Bounded local diff proposals; never writes without confirmation |
| `knowledge_store/` (`repository.rs`, `types.rs`) | SQLite knowledge store: migrations, repository, typed records, backup/quarantine |
| `selection.rs` | AX selection capture for transform (secure-field fail-closed) |
| `transform_apply.rs` | Approve/undo write-back (only path that writes to the target app) |
| `transform_flow.rs` | End-to-end transform orchestrator + Tauri commands |
| `transform_presets.rs` | Built-in spoken transform presets (Shorten/Bullets/…) |
| `transform_diagnostics.rs` / `transform_trace.rs` | Per-pass records, consented captures, pass-scoped correlation |
| `llm_sidecar.rs` | Host supervisor for signed local-LLM helper (no in-process llama) |
| `log_shipper.rs` | Zero-config diagnostic log upload (tails events.jsonl → central ingest) |
| `hang_diagnostics.rs` | Consented, bounded capture-hang diagnostic arming and probe collection |
| `audio_graph_snapshot.rs` | Deadline-guarded Core Audio HAL graph introspection, content-free counts, and Murmur's own audio-owner readout |
| `smart_formatting.rs` | Deterministic prose formatting and same-utterance backtracking |
| `spoken_numbers.rs` / `spoken_structure.rs` | Deterministic spoken-number, punctuation, layout, symbol, and backtracking grammar |
| `ide_context.rs` | Memory-only bounded IDE symbol and root-relative file index |
| `injector.rs` | Clipboard (arboard) + auto-paste (CGEvent, osascript fallback) |
| `file_output.rs` | Numbered `.txt` / `.wav` output |
| `frontmost.rs` | Frontmost-app query + running-application list |
| `state.rs` | `DictationState`, `TransformStatus`, `AppState`, generation counters |
| `telemetry.rs` | Structured event system: TauriEmitterLayer, ring buffer, JSONL, privacy stripping |
| `vad.rs` | Silero VAD speech filtering via whisper-rs (thread-local context cache) |
| `benchmark.rs` / `evaluation.rs` | Performance Lab scoring and the `murmur-eval` fixture harness |
| `performance_metrics/` (`repository.rs`, `types.rs`) | SQLite run history, typed stage/resource records, retention |
| `resource_monitor.rs` | CPU/RSS sampling, 1s heartbeat, idle-timeout enforcement |
| `alloc.rs` | Custom malloc zone separating Rust heap from whisper.cpp's FFI heap |
| `platform/` | macOS CPU/resource metrics seam |
| `sidecars/capture/` | Killable production capture worker and isolated audio callback boundary |
| `sidecars/local-llm/` | The signed `murmur-llm-sidecar` crate (llama.cpp) |
| `crates/capture-helper-protocol` | Host ↔ capture worker framing, handshake, and control protocol |
| `crates/local-llm-protocol` | Host ↔ sidecar protocol types |

### Frontend (`app/src/`)

| File | Purpose |
|------|---------|
| `App.tsx` | Main orchestrator, wires hooks together |
| `lib/settings.ts` | Settings types, defaults, durable-source/localStorage-cache persistence |
| `lib/onboarding.ts` | First-launch setup-assistant completion flag |
| `lib/events.ts` | Event types, stream/level definitions, color constants |
| `lib/history.ts` | History entries, rolling trim, search + match segmentation, export rendering |
| `lib/durableUserData.ts` | History/stats/theme-library disk hydration, localStorage migration, write-through and clear |
| `lib/appearance/` | Semantic resolver, active storage/boot, durable library, VS Code conversion, and bounded Open VSX package ingestion |
| `lib/hooks/useAppearance.ts` | Main-only active appearance and theme-library controller |
| `lib/historyExport.ts` | Clipboard and save-dialog wrappers for history exports |
| `lib/commandPalette.ts` | Palette command type, tiered scoring, filtering, selection movement |
| `lib/keyboardShortcuts.ts` | Pure main-window keydown → action mapping (⌘K/⌘F/⌘,/⌘L) |
| `lib/silenceAutoStop.ts` | Deterministic trailing-silence detector (pure per-sample fold) |
| `lib/stats.ts` | Usage metrics: words, WPM, recordings, tokens |
| `lib/dictation.ts` | Tauri command wrappers for dictation pipeline |
| `lib/microphonePreview.ts` | Microphone preview command wrappers, event types, and meter presentation helpers |
| `lib/updater.ts` | Semver parsing, min-version checking, update utilities |
| `lib/log.ts` | Frontend logging via Rust tracing (flog utility) |
| `lib/hooks/useHoldDownToggle.ts` | Hold-down mode (rdev press/release events) |
| `lib/hooks/useDoubleTapToggle.ts` | Double-tap mode (rdev events) |
| `lib/hooks/useCombinedToggle.ts` | Both mode (hold-down + double-tap simultaneous) |
| `lib/hooks/useRecordingState.ts` | Recording status, transcription, toggle logic |
| `lib/hooks/useAutoUpdater.ts` | OTA updates, min-version enforcement |
| `lib/hooks/useHistoryManagement.ts` | Transcription history: add/update/clear with durable write-through persistence |
| `lib/hooks/useSilenceAutoStop.ts` | Ends a hands-free (not hold-started) recording after trailing silence |
| `lib/hooks/useRecordingOrigin.ts` | Tracks whether the in-flight recording is hold- or toggle-started |
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
| `lib/microphoneStartupBenchmark.ts` | Strict startup-diagnostic IPC/report boundary, local retention, summaries, and typed export |
| `lib/transformFlow.ts` | Pure reducer for transform press/release |
| `lib/transformReview.ts` | Review state/error types + content guards |
| `components/onboarding/OnboardingFlow.tsx` | First-launch setup assistant (permissions + model wizard) |
| `components/CommandPalette.tsx` | ⌘K command palette dialog |
| `components/history/HistoryPanel.tsx` | History workspace: search, filters, export menu |
| `components/settings/SettingsPanel.tsx` | Settings UI with mode switching (incl. Transform page) |
| `components/settings/MicrophoneInputTest.tsx` | Live capture-only microphone meter and safe input switching |
| `components/settings/TransformsManager.tsx` | Saved transform CRUD UI |
| `components/transform-review/` | Review popover UI (diff, actions, mock driver) |
| `components/settings/PerformanceLab.tsx` | Benchmark UI, scoring tables, report save/export |
| `components/settings/MicrophoneStartupBenchmark.tsx` | Five-cycle microphone startup progress, backend attempts, refusal, cancellation, and report UI |
| `components/settings/KnowledgeManager.tsx` | Knowledge store browse/edit UI |
| `components/history/CorrectAndTeachDialog.tsx` | Correct-and-Teach review + scope choice |
| `components/log-viewer/DiagnosticsWorkspace.tsx` | Embedded Performance workspace: Events, Performance, Runs, Transforms, Reports |
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

- **Playwright** (`@playwright/mcp`): Browser automation for UI work. When making frontend/UI changes, use `browser_navigate` to `http://localhost:1420` and `browser_take_screenshot` to visually verify your changes. Requires `npm run tauri:dev` to be running. Screenshots return inline as images — evaluate them and iterate until the UI looks right.

## Dependencies

- **Rust**: tauri 2, whisper-rs (Metal), FluidAudio (Core ML), sherpa-onnx, cpal, arboard, hound, rusqlite, core-graphics, objc2/objc2-app-kit, rdev (git main branch)
- **Sidecar**: llama-cpp-2 — in `sidecars/local-llm` only, never in the app crate
- **Frontend**: React 18, Tailwind CSS 4, @tauri-apps/api, Vite 6, TypeScript, vitest
