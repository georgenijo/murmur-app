# Murmur — agent guide (CLAUDE.md / AGENTS.md)

Privacy-first macOS voice-to-text app. Tauri 2 (Rust + React). Local transcription on the ANE (Core ML), Metal (whisper.cpp), or CPU (sherpa-onnx); local selected-text rewriting through a signed LLM sidecar. Clipboard-first output. Optional Voice Query runs a user-configured local CLI, which may send questions, answers, or context to a cloud service.

## Commands

```bash
python3 scripts/build_local_llm_sidecar.py  # Build the bundled macOS helpers FIRST (see note)
cd app && npm run tauri:dev        # Dev with hot reload and isolated dev bundle/data
cd app && npm run tauri:bench:dev  # Dev build for benchmark runs
cd app && npm run tauri build      # Production .app and .dmg
cd app/src-tauri && cargo test -- --test-threads=1  # Rust unit tests
cd app && npx tsc --noEmit         # TypeScript check
cd app && npm test                 # Frontend vitest — CI runs this too; tsc alone is not enough
cd app && npm run audit:dead-code   # Audit unused exports and files
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
- **[docs/reference/](docs/reference/)** — `commands.md` (199 Tauri commands), `events.md`, `hooks.md`, `settings.md`

Read these before working on a feature:

- **[docs/onboarding.md](docs/onboarding.md)** — Setup, permissions, model installation, logs
- **[docs/features/onboarding-flow.md](docs/features/onboarding-flow.md)** — First-launch setup assistant (permissions wizard + model download)
- **[docs/features/history-workspace.md](docs/features/history-workspace.md)** — Transcript search, filters, and export
- **[docs/features/command-palette.md](docs/features/command-palette.md)** — ⌘K launcher and main-window shortcuts
- **[docs/features/silence-auto-stop.md](docs/features/silence-auto-stop.md)** — Hands-free trailing-silence finish for recordings not started by holding the key
- **[docs/features/recording-modes.md](docs/features/recording-modes.md)** — Hold-down and double-tap modes, state machine, rdev threading
- **[docs/features/modes.md](docs/features/modes.md)** — Named behavior bundles, app and browser-site bindings, and temporary mode selection
- **[docs/features/transcription.md](docs/features/transcription.md)** — Audio capture, whisper pipeline, status flow
- **[docs/features/microphone-input-test.md](docs/features/microphone-input-test.md)** — Capture-only Settings meter, device switching, privacy and ownership
- **[docs/features/cli-command-formatting.md](docs/features/cli-command-formatting.md)** — Spoken CLI detection, grammar, lexicon, safety
- **[docs/features/smart-formatting.md](docs/features/smart-formatting.md)** — Deterministic prose grammar, backtracking, bounds, privacy
- **[docs/features/spoken-structure.md](docs/features/spoken-structure.md)** — Spoken punctuation, symbols, layout commands, and backtracking
- **[docs/features/spoken-number-formatting.md](docs/features/spoken-number-formatting.md)** — Deterministic formatting for spoken numbers, money, dates, times, and measurements
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
- **[docs/features/voice-query.md](docs/features/voice-query.md)** — Spoken questions sent to an explicitly configured CLI provider
- **[docs/features/vocabulary-aliases.md](docs/features/vocabulary-aliases.md)** — Exact spoken variants mapped to canonical written terms
- **[docs/features/correct-and-teach.md](docs/features/correct-and-teach.md)** — Bounded learned corrections, exact-term teaching, scope and fail-closed rules
- **[docs/features/personal-knowledge-store.md](docs/features/personal-knowledge-store.md)** — Local SQLite store, migrations, backup/recovery, export/import
- **[docs/features/performance-lab.md](docs/features/performance-lab.md)** — Benchmarking, WER tiers, recommendation contract
- **[docs/features/internal-performance-harness.md](docs/features/internal-performance-harness.md)** — Private personal-corpus build, Fleet runner, and mandatory PR/release performance gates
- **[docs/features/diagnostic-report-comparison.md](docs/features/diagnostic-report-comparison.md)** — Session-only Reports workspace and comparison
- **[docs/features/selected-text-transform.md](docs/features/selected-text-transform.md)** — Local selected-text rewrite (hold key, sidecar LLM, review popover, approve/undo)
- **[docs/features/dictation-correction.md](docs/features/dictation-correction.md)** — Spoken correction of the latest delivered dictation
- **[docs/features/meeting-capture.md](docs/features/meeting-capture.md)** — Local microphone and system-audio meeting capture with durable segments
- **[docs/features/meeting-review-workspace.md](docs/features/meeting-review-workspace.md)** — Evidence-linked meeting summaries, edits, labels, and exports
- **[docs/features/meeting-diarization.md](docs/features/meeting-diarization.md)** — Local remote-speaker diarization with a separately managed Core ML model
- **[docs/features/appearance.md](docs/features/appearance.md)** — Semantic themes, local imports, saved themes, and community-theme installation
- **[docs/features/ui-design-system.md](docs/features/ui-design-system.md)** — UI tokens, shared controls, layout rules, and visual regression checks
- **[docs/features/evaluation-harness.md](docs/features/evaluation-harness.md)** — Versioned local fixtures, deterministic CI, opt-in hardware evaluation, reports, and deletion
- **[docs/features/performance-diagnostics.md](docs/features/performance-diagnostics.md)** — Versioned local run metrics, retention, correlation, scoped resources, and privacy
- **[docs/decisions/DECISIONS.md](docs/decisions/DECISIONS.md)** — Running log of architectural/scope decisions (newest first)

## File Map

### Rust (`app/src-tauri/src/`)

| File | Purpose |
|------|---------|
| `lib.rs` | App wiring: mod declarations, `State`, `MutexExt`, 199 registered commands, setup, tray, `run()` |
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
| `commands/dictation_preview.rs` | Live dictation preview popover geometry + show/hide (under-notch, click-through) |
| `commands/meeting.rs` | Starts and stops meetings and exposes meeting storage, review, diarization, and export commands |
| `commands/meeting_summary.rs` | Coordinates local meeting-summary jobs, status, cancellation, and generation ownership |
| `commands/query_popover.rs` | Sizes, shows, expands, and hides the non-activating Voice Query popover |
| `commands/query_history.rs` | Provides main-window-gated Voice Query history listing and deletion commands |
| `commands/mode_runtime.rs` | Resolves modes from app and browser-site context, applies temporary overrides, and publishes changes |
| `commands/dictation_diagnostics.rs` | Provides window-gated commands to arm, inspect, delete, and upload private dictation captures |
| `commands/updater.rs` | Reports the update-install environment and runs the updater canary protocol |
| `commands/corpus.rs` | Records and manages the private, capture-only personal benchmark corpus |
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
| `sqlite_support.rs` | Shared low-level SQLite helpers (pragma configuration, quick_check, schema_version, sidecar/quarantine handling, newest-first listing) used by `knowledge_store`, `query_history`, `meeting_store`, and `performance_metrics` |
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
| `browser_site.rs` | Reads supported browsers' current hosts through Accessibility and matches normalized site-mode rules |
| `capture_health.rs` | Reconstructs and stores bounded, content-free capture startup and fallback health observations |
| `coreml_installer.rs` | Supervises killable Core ML installation workers with bounded protocol parsing and confirmed termination |
| `correction_shortcut.rs` | Registers and interprets the global shortcut that starts correction of the latest dictation |
| `delivery_recovery.rs` | Retains the latest delivered text and retries clipboard and paste delivery on demand |
| `diarization_assignment.rs` | Validates speaker turns and assigns remote-speaker IDs to meeting transcript segments |
| `diarization_audio.rs` | Owns, bounds, writes, and cleans temporary whole-meeting audio used for diarization |
| `diarization_model.rs` | Downloads, validates, leases, reports, and removes the pinned diarization Core ML model |
| `dictation_correction.rs` | Interprets spoken correction instructions and validates corrected text before delivery |
| `dictation_diagnostics.rs` | Stores bounded transcript content only for an explicitly armed live-dictation diagnostic capture |
| `dictation_diagnostics_contract.rs` | Tests consent, single-recording ownership, bounds, retention, and corrupt-store handling for private captures |
| `dictation_telemetry.rs` | Correlates accepted recordings with privacy-safe terminal outcomes and delivery metadata |
| `main.rs` | Dispatches helper worker modes before starting the Tauri application |
| `meeting_artifact.rs` | Parses, validates, chunks, merges, and renders source-linked meeting summary artifacts |
| `meeting_capture.rs` | Owns meeting microphone and system-audio capture, helper lifecycle, segmentation, and status publication |
| `meeting_diarization.rs` | Schedules and supervises isolated diarization workers with cancellation and foreground-priority control |
| `meeting_diarization_probe.rs` | Runs debug-only diarization hardware and preemption checks through the production worker path |
| `meeting_review.rs` | Validates, edits, restores, and exports evidence-linked meeting review documents |
| `meeting_store/migrations.rs` | Migrates and validates the meeting SQLite schema and reports database failures |
| `meeting_store/repository.rs` | Persists meeting sessions, segments, labels, artifacts, reviews, retention, and owned audio files |
| `meeting_store/types.rs` | Defines the serialized meeting store, session, segment, speaker, page, and detail contracts |
| `microphone_auto.rs` | Selects a Smart Auto microphone from the cached idle-time inventory and returns a stable device ID |
| `model_artifact.rs` | Rejects undersized or response-document payloads before downloaded model files reach a runtime |
| `query_adapter.rs` | Streams supported provider output as answer updates and falls back exactly to raw output on malformed data |
| `query_flow.rs` | Coordinates local query capture and ASR, direct CLI execution, answer streaming, history, and cancellation |
| `query_history/repository.rs` | Persists bounded Voice Query history with migration, validation, privacy permissions, and recovery |
| `query_history/types.rs` | Defines durable Voice Query history entries, pages, token counts, and page-size limits |
| `query_provider.rs` | Discovers provider executables and manages presets, safe arguments, authentication probes, and protected environment paths |

### Diagnostic tools

| Path | Purpose |
|------|---------|
| `tools/murmur-diag` | Runs a local read-only MCP server for health, event, log, session, and keyboard-correlation queries |

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
| `lib/voiceQuerySettings.ts` | Pure Voice Query configuration, probe-result, and request-fingerprint helpers |
| `lib/voiceQuerySettings.test.ts` | Unit coverage for Voice Query configuration and probe presentation helpers |
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
| `lib/hooks/useVoiceQuerySettings.ts` | Voice Query settings state, effects, validation, and provider handlers |
| `lib/hooks/useTransformModelSettings.ts` | Selected-text model status, download, shortcut, removal, and reset controller |
| `lib/hooks/useDevUpdaterMock.ts` | Always-called development updater-state driver with production passthrough |
| `lib/hooks/useDeliveryRecoveryListeners.ts` | Delivery retry and correction-start failure event listeners |
| `lib/hooks/useInitialization.ts` | One-time init sequence (initDictation + configure) |
| `lib/hooks/useShowAboutListener.ts` | Listens for show-about tray event |
| `lib/hooks/useEventStore.ts` | Structured event log buffer with live streaming |
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
| `components/settings/VoiceQuerySettings.tsx` | Voice Query provider, privacy, shortcut, and response settings page |
| `components/settings/TransformModelSettings.tsx` | Selected-text rewrite model, shortcut, and saved-transform settings page |
| `components/settings/SettingToggle.tsx` | Shared labeled settings-row toggle control |
| `components/settings/AppearanceSettings.tsx` | Appearance editor, local import preview, saved library, and community-theme dialog |
| `components/settings/MicrophoneInputTest.tsx` | Live capture-only microphone meter and safe input switching |
| `components/settings/TransformsManager.tsx` | Saved transform CRUD UI |
| `components/transform-review/` | Review popover UI (diff, actions, mock driver) |
| `components/settings/PerformanceLab.tsx` | Benchmark UI, scoring tables, report save/export |
| `components/settings/MicrophoneStartupBenchmark.tsx` | Five-cycle microphone startup progress, backend attempts, refusal, cancellation, and report UI |
| `components/settings/KnowledgeManager.tsx` | Knowledge store browse/edit UI |
| `components/history/CorrectAndTeachDialog.tsx` | Correct-and-Teach review + scope choice |
| `components/log-viewer/DiagnosticsWorkspace.tsx` | Embedded diagnostics tabs: Events, Runs, Performance, Latency, Compare, Transform, and Dictation |
| `components/overlay/deriveVisual.ts` | Pure: overlay top-bar indicator + flash-priority derivation |
| `components/overlay/OverlayPill.tsx` | Overlay top bar (presentational) |
| `components/overlay/OverlayDropdown.tsx` | Overlay quick-settings dropdown (presentational) |
| `components/OverlayWidget.tsx` | Dynamic Island overlay composition shell |
| `components/AboutModal.tsx` | Shows the app version and local-processing summary in a reduced-motion-aware dialog |
| `components/FileTranscriptionToasts.tsx` | Shows per-file transcription progress, completion, failure, and dismissal controls |
| `components/MainErrorBanner.tsx` | Displays and dismisses the current main-window error message |
| `components/MainHeader.tsx` | Renders dictation or meeting status, recording controls, shortcut hints, updates, and Settings navigation |
| `components/ModelDownloader.tsx` | Presents speech-model selection, download progress, validation errors, and completion |
| `components/PermissionsBanner.tsx` | Tracks microphone, Accessibility, and system-audio permission failures and offers repair actions |
| `components/UpdateIndicator.tsx` | Shows compact update checking, availability, and failure states with retry or detail actions |
| `components/UpdateModal.tsx` | Presents release notes and update download, retry, skip, and dismissal actions |
| `components/UsageDashboard.tsx` | Renders dictation and Voice Query totals, recent trends, and the activity heatmap |
| `components/WhatsNewModal.tsx` | Shows the installed version's release notes after an update |
| `components/dictation-preview/DictationPreviewApp.tsx` | Displays a bounded live partial transcript in the dictation preview window |
| `components/history/MeetingReviewWorkspace.tsx` | Edits, restores, labels, and exports meeting reviews alongside source transcript evidence |
| `components/history/MeetingsPanel.tsx` | Lists, searches, selects, and deletes saved meeting sessions |
| `components/history/QueryHistoryPanel.tsx` | Filters, pages, displays, and clears retained Voice Query questions and answers |
| `components/home/HomeDashboard.tsx` | Composes the home recording controls, recent dictations, personalization status, and setup tip |
| `components/home/HomeInsightsRail.tsx` | Summarizes recent dictation activity, words, recording time, and personalization progress |
| `components/home/HomeRecordingBar.tsx` | Presents the primary recording control, mode status, hotkey hint, and audio level |
| `components/home/HomeSidebar.tsx` | Provides responsive navigation between Home, Notetaker, Queries, and Insights |
| `components/home/InsightsView.tsx` | Hosts the full usage dashboard and back navigation for the Insights destination |
| `components/home/PersonalizationCard.tsx` | Shows vocabulary personalization progress and links to vocabulary settings |
| `components/log-viewer/DiagnosticsWindowApp.tsx` | Hosts the standalone diagnostics window and applies requested tab changes |
| `components/log-viewer/DictationDiagnosticsView.tsx` | Arms, lists, inspects, deletes, and uploads consented private dictation captures |
| `components/log-viewer/EventRow.tsx` | Renders one structured event with level, stream, timing, correlation, and detail fields |
| `components/log-viewer/LatencyMapView.tsx` | Summarizes UI transition latency by route and lists recent measured transitions |
| `components/log-viewer/LevelFilter.tsx` | Toggles structured event log levels |
| `components/log-viewer/PerformanceChart.tsx` | Draws keyboard-accessible CPU or memory time series on a shared cursor timeline |
| `components/log-viewer/PerformanceStoreHealthBanner.tsx` | Explains diagnostics-store failures and offers retry or reinitialization actions |
| `components/log-viewer/PerformanceView.tsx` | Presents runtime health, resource measurements, and CPU and memory charts |
| `components/log-viewer/ProductionComparison.tsx` | Compares compatible production-run cohorts across app versions and reports exclusions |
| `components/log-viewer/ReportCompareView.tsx` | Imports two diagnostic or benchmark reports and presents compatibility and metric differences |
| `components/log-viewer/RunDetail.tsx` | Shows one run's outcome, stages, timing waterfall, resources, and correlated events |
| `components/log-viewer/RunsView.tsx` | Filters and pages stored performance runs and opens run detail |
| `components/log-viewer/StreamChips.tsx` | Toggles structured event streams |
| `components/log-viewer/TransformDiagnosticsView.tsx` | Lists transform attempts and manages consented local transform-content captures |
| `components/query-review/QueryReviewApp.tsx` | Renders streamed Voice Query answers, context, usage, errors, copy, cancel, and sign-in actions |
| `components/settings/AppOverridesEditor.tsx` | Creates per-app writing, delivery, IDE-context, and Voice Query context overrides |
| `components/settings/CommunityThemeDialog.tsx` | Searches, sorts, previews, and installs compatible themes from Open VSX |
| `components/settings/CorpusRecorder.tsx` | Guides private benchmark-corpus recording and displays prompt and audio-quality status |
| `components/settings/CustomizationHub.tsx` | Routes the customization overview to text, voice commands, app styles, and transforms |
| `components/settings/KnowledgeEditorModal.tsx` | Creates and edits one knowledge-store entry with kind, scope, variants, and replacement fields |
| `components/settings/MeetingDiarizationSettings.tsx` | Controls meeting diarization and manages the local speaker model download |
| `components/settings/ModesManager.tsx` | Creates modes and binds them to applications or supported browser sites |
| `components/settings/OverlayCalibrationControl.tsx` | Previews and saves the overlay's vertical offset |
| `components/settings/SettingsBranch.tsx` | Keeps dependent controls mounted while removing collapsed content from focus and accessibility navigation |
| `components/settings/SettingsEditorsWindow.tsx` | Hosts focused vocabulary, alias, knowledge, transform, command, and project-scan editors |
| `components/settings/SettingsSection.tsx` | Applies active-page visibility and shared card layout to a settings section |
| `components/settings/SettingsSurfaceContext.ts` | Shares whether the settings window is active with nested components and hooks |
| `components/settings/ThemeLibrary.tsx` | Manages saved themes, local imports, exports, previews, activation, and deletion |
| `components/settings/VocabScanStrip.tsx` | Displays IDE vocabulary scan progress, cancellation, and completion summaries |
| `components/settings/VocabTermsModal.tsx` | Sorts and displays vocabulary terms found by an IDE scan |
| `components/settings/VocabularyAliasesEditor.tsx` | Creates, edits, enables, and deletes spoken-to-written vocabulary aliases |
| `components/settings/VoiceCommandsManager.tsx` | Creates, edits, scopes, enables, and deletes voice-command replacements |
| `components/ui/DashboardPrimitives.tsx` | Provides shared dashboard surfaces, headers, metrics, and empty-state layouts |
| `components/ui/DayChart.tsx` | Renders an accessible daily bar chart with hover and keyboard inspection |
| `components/ui/Select.tsx` | Provides the shared grouped select control and option contract |
| `components/ui/WindowHeader.tsx` | Renders the shared draggable window title bar and close control |
| `components/ui/activity-graph/activity-graph.tsx` | Renders an accessible contribution-style activity grid with selectable days |
| `components/ui/animated-dropdown/animated-dropdown.tsx` | Provides animated dropdown menus with grouped items, keyboard navigation, and focus restoration |
| `components/ui/animated-switch/animated-switch.tsx` | Provides the animated accessible switch used by settings toggles |
| `components/ui/fluid-tabs/fluid-tabs.tsx` | Provides animated tab selection with keyboard navigation and optional badges |
| `components/ui/fluid-tooltip/fluid-tooltip.tsx` | Provides delayed pointer and keyboard tooltips with bounded positioning |
| `components/ui/hold-to-delete-button/hold-to-delete-button.tsx` | Requires a timed pointer or keyboard hold before firing a destructive action |
| `components/ui/smart-overflow/smart-overflow.tsx` | Moves lower-priority actions into an overflow menu when horizontal space runs out |
| `lib/audioDevices.ts` | Validates audio-inventory payloads and resolves selected and missing input devices |
| `lib/buildFlavor.ts` | Exposes whether the frontend was built for internal benchmark use |
| `lib/captureHealth.ts` | Validates capture-health history and derives startup and fallback presentation summaries |
| `lib/corpusRecorder.ts` | Defines benchmark prompts and wraps corpus recording, summary, and folder commands |
| `lib/correctAndTeach.ts` | Wraps correction proposal, confirmation, and discard commands with their typed outcomes |
| `lib/deliveryRecovery.ts` | Wraps last-delivery retry and global retry-shortcut commands |
| `lib/dictationDiagnostics.ts` | Validates and wraps private dictation capture status, listing, retrieval, deletion, and upload |
| `lib/dictationPresentation.ts` | Maps stable dictation status and action codes to user-facing messages and recovery actions |
| `lib/eventFilters.ts` | Matches structured events against recording, transform, scan, and query correlation filters |
| `lib/fileQueue.ts` | Provides pure ordered queue updates for independent multi-file transcription outcomes |
| `lib/homeDashboard.ts` | Derives recent dictations, usage summaries, and personalization stages for the home dashboard |
| `lib/hooks/useAudioInputInventory.ts` | Subscribes to the shared microphone inventory and rejects stale command responses |
| `lib/hooks/useDictationPartial.ts` | Tracks bounded partial transcripts for the current recording and controls the preview window |
| `lib/hooks/useMeetings.ts` | Owns meeting capture, saved-session paging, review, deletion, summary, and diarization actions |
| `lib/hooks/useModeRuntime.ts` | Tracks the resolved mode and exposes cycle and temporary-override controls |
| `lib/hooks/useOverlayRecordingStatus.ts` | Mirrors dictation status into the overlay and ignores invalid event payloads |
| `lib/hooks/usePerformanceStoreHealth.ts` | Refreshes diagnostics-store health and exposes recovery actions |
| `lib/hooks/useQueryFlow.ts` | Drives query hotkeys, capture generations, listener lifecycle, and cancellation |
| `lib/hooks/useQueryHistory.ts` | Owns Voice Query history filters, paging, refresh, expansion, and clear state |
| `lib/hooks/useQueryReviewDriver.ts` | Owns query-review events, streamed answer state, copy, cancellation, and provider sign-in retry |
| `lib/hooks/useSoundCues.ts` | Plays configured recording, success, and failure cues from status transitions |
| `lib/hooks/useTransformReviewMockDriver.ts` | Simulates transform-review states and content for development-only visual testing |
| `lib/hotkeyFeedback.ts` | Validates hotkey-rejection events and defines their overlay flash duration |
| `lib/knowledge.ts` | Defines knowledge-store contracts and wraps CRUD, preview, export, and import commands |
| `lib/meetings.ts` | Defines meeting contracts and wraps capture, storage, review, summary, diarization, and export commands |
| `lib/modelDownload.ts` | Correlates speech-model download attempts and derives progress labels and percentages |
| `lib/modelRuntime.ts` | Validates model-runtime catalog and status responses and wraps their commands |
| `lib/numeric.ts` | Provides total median and percentile calculations without mutating caller input |
| `lib/overlayGeometry.ts` | Validates the Rust-owned overlay geometry contract and shared fixture cases |
| `lib/overlayMotion.ts` | Defines overlay animation timing, spring, hover, and flash constants |
| `lib/productionComparison.ts` | Builds compatible production-run cohorts and compares timing and resource metrics |
| `lib/queryHistory.ts` | Validates durable Voice Query history payloads and wraps listing and clear commands |
| `lib/queryProviders.ts` | Wraps provider discovery, validation, environment, test, sign-in, and shared polling operations |
| `lib/queryReview.ts` | Defines and validates Voice Query review states, content, visibility, and pass IDs |
| `lib/queryUsage.ts` | Validates provider IDs and content-free Voice Query token usage |
| `lib/sona-motion.ts` | Defines shared easing curves and reduced-motion-aware transitions |
| `lib/sona-utils.ts` | Merges conditional Tailwind class names with conflict resolution |
| `lib/soundCues.ts` | Synthesizes short Web Audio cues for recording and delivery events |
| `lib/transformDiagnostics.ts` | Validates and wraps transform attempt and consented-content diagnostic commands |
| `lib/typeGuards.ts` | Provides shared record, number, integer, string, boolean, and string-array guards |
| `lib/types.ts` | Defines and validates dictation runtime status values |
| `lib/uiLatency.ts` | Measures, stores, publishes, and summarizes bounded UI transition samples |
| `lib/updaterEnvironment.ts` | Reads the native update-install environment for updater eligibility and recovery UI |
| `lib/vocabulary.ts` | Normalizes vocabulary terms and detects command, alias, and scope conflicts |

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
