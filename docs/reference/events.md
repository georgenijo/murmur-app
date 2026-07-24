# Tauri Events Reference

Every event emitted from the Rust backend to the frontend via Tauri's event system, plus the two window-to-window events the frontend emits itself. The frontend subscribes with `listen()` from `@tauri-apps/api/event`.

For commands see [commands.md](commands.md). For the hooks that consume these events see [hooks.md](hooks.md).

---

## Recording and transcription

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `audio-level` | `f32` (RMS 0.0–1.0) | `audio.rs` | Continuously during capture, throttled to ~60fps (16ms minimum gap). | Overlay (`useWaveform`), main window (`useRecordingState`). |
| `recording-status-changed` | `string` — `"idle"` \| `"recording"` \| `"processing"` | `commands/recording.rs` | Every dictation state transition. Suppressed when the recording has been superseded by a newer generation. | Main window (`useRecordingState`), overlay (visual state). |
| `transcription-complete` | `{recordingId, text, duration, teachingContext}` | `commands/recording.rs` | After a non-empty transcription is delivered. Broadcast to all windows. `teachingContext` seeds Correct and Teach. | Main window (`useRecordingState` → history, stats, display). |
| `recording-cancelled` | `{recordingId}` | `commands/recording.rs` | A recording was discarded without transcription (speculative Both-mode hold, explicit cancel). | Main window, overlay (clears in-flight UI). |
| `auto-paste-failed` | `string` (hint) | `commands/recording.rs` via `injector.rs` | Auto-paste failed or timed out. The text is already on the clipboard. | Main window (`useRecordingState`, shown for 5s). |
| `file-output-failed` | `string` (hint) | `commands/recording.rs` | Saving the transcript/audio file failed; clipboard delivery still happened. | Main window. |
| `file-transcription-status-changed` | `boolean` | `commands/recording.rs` | `true` when an imported-file transcription starts, `false` when it finishes or aborts. Gates dictation and transform. | Main window (`useFileTranscription`). |

## Models

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `download-progress` | `{received, total}` (bytes) | `commands/models.rs` | Periodically during transcription-model and VAD downloads. `total` may be 0 when the server omits `Content-Length`. | Settings download UI, `ModelDownloader`, onboarding. |
| `model-runtime-status-changed` | `ModelRuntimeSnapshot` | `model_runtime.rs` | On every lifecycle transition (selecting, loading, warming, ready, unloading, failed). Generation-ordered, so a stale load can't overwrite a newer status. | Settings, onboarding, Performance Lab. |
| `transform-model-download-progress` | `{received, total, phase}` — `phase` is `"downloading"` \| `"installed"` | `commands/transform_model.rs` | While streaming the pinned local-LLM GGUF, and once on successful publication. | Settings → Transform. |

## Keyboard

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `hold-down-start` | `()` | `keyboard.rs` | Hold key pressed (immediately in hold-down mode; after the 200ms promotion timer in Both mode). | `useHoldDownToggle`, `useCombinedToggle`. |
| `hold-down-stop` | `()` | `keyboard.rs` | Hold key released after a valid hold. Also emitted by `update_keyboard_key` when the key changes mid-hold, so no recording is stranded. | `useHoldDownToggle`, `useCombinedToggle`. |
| `double-tap-toggle` | `()` | `keyboard.rs` | A valid double-tap sequence completes. In Both mode, emitted on release when the hold was never promoted. | `useDoubleTapToggle`, `useCombinedToggle`. |
| `hotkey-tap-rejected` | `{reason: "second_tap_expired", mode: "double_tap" \| "both"}` | `keyboard.rs` | An idle first tap is not followed by a second within 400ms. Never for holds, combos, processing skips, or valid double-taps. | Overlay — amber timing-miss flash, only when `hotkeyMissFeedback` is on. |
| `keyboard-listener-error` | `string` | `keyboard.rs` | The rdev listener thread errors. | All three recording hooks; they wait 2s and restart the listener. |
| `app-disabled-changed` | `boolean` | `commands/keyboard.rs` | Global disable toggled from the tray or the overlay's power button. | Main window, overlay (`useOverlayRuntime`). |

**Dead listener:** `useCombinedToggle` registers `hold-down-cancel`, which nothing emits. In Both mode an unpromoted tap emits nothing at all, because no recording was ever started.

## Selected-text transform

All transform events carry a `transformPassId` where a pass exists, so a delayed handler can prove whether it still owns the flow. None of them carry instruction, selection, or proposal text.

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `transform-key-pressed` | `{transformPassId}` | `keyboard.rs` | The transform hold key goes down. The pass ID is assigned here, in the rdev callback. | Main window (`useTransformFlow`). |
| `transform-key-released` | `{transformPassId}` | `keyboard.rs`, `commands/keyboard.rs` | The transform key is released, or the listener is torn down while it is held. | Main window (`useTransformFlow`). |
| `transform-state-changed` | `{state, transformPassId, errorCode?}` — `state` is `listening` \| `thinking` \| `ready` \| `failed` \| `applied`; `errorCode` is a stable enum (`model_not_downloaded`, `timeout`, `output_invalid`, `crashed`, `target_gone`, `selection_changed`, `paste_failed`, …) | `transform_flow.rs` | Every review-state transition. Deliberately carries no text — the popover fetches content separately via `get_transform_review_content`. | Transform popover (`useTransformReviewDriver`). |
| `transform-review-hidden` | `()` | `transform_flow.rs` | The popover has been hidden (cancel, linger expiry, teardown). | Popover, main window. |
| `transform-busy` | `()` | `transform_flow.rs` | A transform keypress was refused because dictation, a benchmark, a file transcription, or another transform owns the pipeline. | Overlay — amber busy flash, so the press is never silently ignored. |
| `transform-secure-field` | `()` | `transform_flow.rs` | Capture refused because the focused element is (or cannot be proven not to be) a secure field. No content is shown. | Overlay flash only. |
| `transform-apply-failed` | `string` (stable error code) | `transform_apply.rs` | Apply or undo write-back failed. | Popover — surfaces the failure inline while keeping Undo available. |
| `escape-cancel` | `{transformPassId}` | `keyboard.rs` | Escape pressed during Capturing / Listening / Thinking, or the brief ReviewPending window before the popover is focusable. Snapshots the pass ID at press time. | Main window (`useEscapeCancel`) → scoped `cancel_transform`. |

## Overlay

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `overlay-geometry-changed` | `OverlayGeometry` (never null) | `commands/overlay.rs` | Display configuration changes (monitor plug/unplug, lid open/close), via an `NSApplicationDidChangeScreenParametersNotification` observer. Carries the recomputed contract. | Overlay: `useOverlayGeometry` re-renders from it; `useOverlayExpansion` treats it as an authoritative reset — cancels timers, forces collapsed, issues one corrective resize. |
| `overlay-visible-changed` | `boolean` | `commands/overlay.rs` | After `show_overlay` / `hide_overlay`. **No live emitter in production** — the overlay is shown once at setup and stays visible, so nothing calls these today. | Overlay: gates the expansion controller's cursor poller. Defaults to visible on mount so first hover works regardless. |

## Diagnostics and benchmarking

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `performance-run-completed` | `PerformanceRunV1` | `performance_metrics/mod.rs` | A dictation, file, or transform run finishes and is persisted. | Log viewer (`usePerformanceDiagnostics`). |
| `performance-resource-sample` | `ResourceSampleV1` | `performance_metrics/mod.rs` | Once per second from the resource heartbeat: host CPU, main-process CPU/RSS/Rust-heap/FFI-heap, and sidecar process figures. | Log viewer charts. |
| `performance-diagnostics-cleared` | `()` | `performance_metrics/mod.rs` | Local run history and samples were deleted. | Log viewer (resets views). |
| `benchmark-progress` | `BenchmarkProgress {completed, …}` | `benchmark.rs` | During a Performance Lab run, per completed model/fixture unit. | Performance Lab. |
| `vocab-scan-progress` | `VocabScanProgress {scanId, files, skipped, terms, …}` | `commands/recording.rs` | Throttled during a code-vocabulary folder walk, plus once at completion. Correlated by `scanId` so a superseded scan can't overwrite a newer one. | Settings (`useVocabScan`, `VocabScanStrip`). |

## Structured logging

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `app-event` | `AppEvent` | `telemetry.rs` (`TauriEmitterLayer`) | For **every** `tracing` event in the Rust backend. | Log viewer (`useEventStore`). Release `pipeline` strings are stripped; `transform` strings are restricted by key **and** value to an explicit stable vocabulary in all builds. |

## Frontend-emitted (window to window)

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `settings-changed` | `()` | `useSettings`, `useOverlaySettingsMirror` | A window mutates persisted settings, so the other windows re-read localStorage. | Main window, overlay. |
| `open-settings` | `()` | `useOverlaySettingsMirror` | The overlay's quick-settings card asks the main window to open Settings. | Main window (`useOpenSettingsListener`). |

**Dead listener:** `useShowAboutListener` listens for `show-about`, but the tray menu no longer has an About item (it is Show Murmur / Disable Murmur / Quit), so nothing emits it.

---

## Event payload types

### AppEvent

```typescript
interface AppEvent {
  timestamp: string;              // ISO timestamp
  stream: StreamName;             // tracing target
  level: LevelName;
  summary: string;                // the tracing message
  data: Record<string, unknown>;  // structured fields, after privacy stripping
}

type StreamName = 'pipeline' | 'audio' | 'keyboard' | 'transform' | 'system';
type LevelName  = 'trace' | 'debug' | 'info' | 'warn' | 'error';
```

Streams correspond to Rust tracing targets; levels to standard tracing severities. Color mappings for both live in `app/src/lib/events.ts`.
