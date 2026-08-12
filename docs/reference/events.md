# Tauri Events Reference

Every event emitted from the Rust backend to the frontend via Tauri's event system, plus the two window-to-window events the frontend emits itself. The frontend subscribes with `listen()` from `@tauri-apps/api/event`.

For commands see [commands.md](commands.md). For the hooks that consume these events see [hooks.md](hooks.md).

---

## Recording and transcription

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `audio-level` | `f32` (RMS 0.0–1.0) | `audio.rs` | Continuously during capture, throttled to ~60fps (16ms minimum gap). | Overlay (`useWaveform`), main window (`useRecordingState`). |
| `microphone-preview-level` | `{previewId, rms, peak, classification}` | `audio.rs` | Targeted to the main window at display rate while the exact Preview owner is active. Samples are aggregated between emissions; classification is `no_signal`, `too_quiet`, `signal_detected`, or `clipping`. Preview never emits `audio-level`. | Settings (`MicrophoneInputTest`, requestAnimationFrame direct DOM updates). |
| `microphone-preview-vad` | `{previewId, sensitivity, decision}` | `microphone_preview.rs` | When the actual Silero VAD decision changes for the exact Preview generation. A bounded trailing window is analyzed off the capture thread at most every 400 ms. Decision is `speech_detected`, `no_speech`, or `unavailable`; results from older slider revisions are discarded. | Settings (`MicrophoneInputTest`). |
| `microphone-preview-status` | `{previewId, state, stillConnecting, errorKind, message}` | `commands/microphone_preview.rs` | On generation-aware preview transitions: connecting, active, stopping, error, and joined-worker idle. Terminal errors retain a message with null ownership. | Settings (`MicrophoneInputTest`). |
| `recording-status-changed` | `string` — `"idle"` \| `"starting"` \| `"recording"` \| `"recovering"` \| `"processing"` | `commands/recording.rs` | Every dictation state transition. Suppressed when the recording has been superseded by a newer generation. | Main window (`useRecordingState`), overlay (visual state). |
| `audio-initialization-stalled` | `{recordingId}` | `commands/recording.rs` | The current generation has spent 5 seconds in `Starting`; this is informational, not failure. | Overlay slow-connecting cue. |
| `audio-recovery-started` | `{recordingId, reason}` | `commands/recording.rs` | A starting/recording owner was cancelled, including when the 30-second hard deadline begins recovery. The app reports `recovering` and retains exclusive logical ownership until its worker exits, then returns to `idle`. A hard deadline also emits the failure event below. | Overlay recovering cue and diagnostics. |
| `recording-initialization-failed` | `{recordingId, error, errorKind}` | `commands/recording.rs` | Initialization/runtime capture failed or crossed the 30-second hard deadline. A hard deadline emits this after `audio-recovery-started`. `error` is a fixed user-facing message and `errorKind` is a stable content-free enum; raw CPAL messages are never emitted. Emitted exactly once per generation. | Main error surface; overlay bounded failure flash. |
| `recording-recovery-stalled` | `{recordingId}` | `commands/recording.rs` | A normal recording stop/teardown has remained blocked for the recovery grace period. | Main persistent, truthfully scoped restart guidance. |
| `recording-interrupted` | `{recordingId, reason, deliveredSamples, durationMs, autoTranscribe}` | `commands/recording.rs` | The isolated capture worker ended unexpectedly after delivering zero or more samples. At least 8,000 delivered samples sets `autoTranscribe` and preserves a partial transcript; shorter captures fail without transcription. | Main window (`useRecordingState`) finalizes or discards the partial capture; overlay (`useOverlayRuntime`) flashes failure. |
| `transcription-complete` | `{recordingId, text, duration, teachingContext}` | `commands/recording.rs` | After a non-empty transcription is delivered. Broadcast to all windows. `teachingContext` seeds Correct and Teach. | Main window (`useRecordingState` → history, stats, display). |
| `recording-cancelled` | `{recordingId}` | `commands/recording.rs` | A recording was discarded without transcription (speculative Both-mode hold, explicit cancel). | Main window, overlay (clears in-flight UI). |
| `auto-paste-failed` | `string` (hint) | `commands/recording.rs` via `injector.rs` | Auto-paste failed or timed out. The text is already on the clipboard. | Main window (`useRecordingState`, shown for 5s). |
| `file-output-failed` | `string` (hint) | `commands/recording.rs` | Saving the transcript/audio file failed; clipboard delivery still happened. | Main window. |
| `file-transcription-status-changed` | `boolean` | `commands/recording.rs` | `true` when an imported-file transcription starts, `false` when it finishes or aborts. Gates dictation and transform. | Main window (`useFileTranscription`). |

## Meeting capture

Meeting transcript text is carried only by the targeted finalized-segment UI
event and local SQLite reads. Structured meeting logs are independently
sanitized, and the fleet log shipper drops the entire `meeting` stream.

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `meeting-status-changed` | `MeetingRuntimeStatus {generation, sessionId, phase, elapsedMs, microphoneActive, systemAudioActive, errorCode}` | `meeting_capture.rs` | On start/setup/channel-ready/stop/processing/failure transitions. Contains no transcript or path. | Main (`useMeetings`), overlay (distinct meeting state). |
| `meeting-segment-finalized` | `MeetingSegment` | `meeting_capture.rs` | After a pending chunk transcribes and its final segment transaction commits. | Main (`useMeetings` bounded live transcript). |
| `meeting-segment-failed` | `{sessionId, segmentId, errorCode}` | `meeting_capture.rs` | A durable pending chunk could not be read or transcribed and transitioned to failed. Contains no text/path. | Main (`useMeetings` actionable warning). |
| `system-audio-permission-changed` | `"unknown" \| "granted" \| "denied" \| "unsupported"` | `commands/meeting.rs` | After the user explicitly runs the short-lived permission probe. | Main permission banner and `useMeetings`. |

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
| `query-toggle` | `{queryPassId, action: "start" \| "stop"}` | `keyboard.rs` | A dedicated query-key double-tap starts a pass, or its next single tap stops capture. There is no spoken-keyword trigger. | Main window (`useQueryFlow`). |

**Dead listener:** `useCombinedToggle` registers `hold-down-cancel`, which nothing emits. In Both mode an unpromoted tap emits nothing at all, because no recording was ever started.

## Selected-text transform

All transform events carry a `transformPassId` where a pass exists, so a delayed handler can prove whether it still owns the flow. None of them carry instruction, selection, or proposal text.

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `transform-key-pressed` | `{transformPassId}` | `keyboard.rs` | The transform hold key goes down. The pass ID is assigned here, in the rdev callback. | Main window (`useTransformFlow`). |
| `transform-key-released` | `{transformPassId}` | `keyboard.rs`, `commands/keyboard.rs` | The transform key is released, or the listener is torn down while it is held. | Main window (`useTransformFlow`). |
| `transform-state-changed` | `{state, transformPassId, errorCode?}` — `state` is `connecting` \| `recovering` \| `listening` \| `thinking` \| `ready` \| `failed` \| `applied`; `errorCode` is a stable enum (`audio_stalled`, `audio_recovery_stalled`, `audio_not_ready`, `model_not_downloaded`, `timeout`, `output_invalid`, `crashed`, `target_gone`, `selection_changed`, `paste_failed`, …) | `transform_flow.rs` | Every review-state transition. Deliberately carries no text — the popover fetches content separately via `get_transform_review_content`. | Transform popover (`useTransformReviewDriver`). |
| `transform-review-hidden` | `()` | `transform_flow.rs` | The popover has been hidden (cancel, linger expiry, teardown). | Popover, main window. |
| `transform-busy` | `()` | `transform_flow.rs` | A transform keypress was refused because dictation, a benchmark, a file transcription, or another transform owns the pipeline. | Overlay — amber busy flash, so the press is never silently ignored. |
| `transform-secure-field` | `()` | `transform_flow.rs` | Capture refused because the focused element is (or cannot be proven not to be) a secure field. No content is shown. | Overlay flash only. |
| `transform-capture-failed` | `()` | `transform_flow.rs` | The isolated audio worker interrupted a selected-text transform. The backend clears the active transform pass and returns to idle before emitting. | No current frontend listener; retained as a backend failure notification contract. |
| `transform-apply-failed` | `string` (stable error code) | `transform_apply.rs` | Apply or undo write-back failed. | Popover — surfaces the failure inline while keeping Undo available. |
| `escape-cancel` | `{transformPassId, queryPassId}` | `keyboard.rs` | Escape snapshots the active transform first, otherwise the active query, otherwise dictation. Exact pass IDs prevent delayed cancellation from reaching a newer flow. | Main window (`useEscapeCancel`) routes to scoped cancellation. |

## Voice Query

Question and context content never appears in events. Only the dedicated review webview receives answer chunks; it pulls the context summary through the gated review-content command.

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `query-state-changed` | `{queryPassId, state, errorCode, usage}` | `query_flow.rs` | Every content-free state transition: connecting, listening, transcribing, running, ready, or failed. `usage` is null until a structured provider reports numeric token/cache/reasoning counts and optional cost; it never contains content. Typed provider failures use `provider_error`; known authentication failures use the repairable `provider_not_authenticated`. Stderr and provider detail are never included. | Query popover (`useQueryReviewDriver`) and main-window content-free stats aggregation (`useQueryFlow`). |
| `query-context-resolved` | `{queryPassId}` | `query_flow.rs` | The immutable context snapshot is ready. Targeted to `query-review`; carries no app, window, selection, or summary content and only triggers a gated refresh. | Query popover only. |
| `query-answer-chunk` | `{queryPassId, sequence, text, replace}` | `query_flow.rs` | A decoded raw chunk or structured Claude/Codex answer chunk accepted within the 256 KiB cap. `replace: true` atomically resets optimistic structured text to the complete raw stream when JSONL parsing falls back, including incomplete EOF without a typed terminal event; otherwise text appends. Targeted with `emit_to("query-review", …)`, never broadcast. | Query popover only. |
| `query-review-hidden` | `()` | `query_flow.rs` | Exact-pass cancellation/close completes and the popover is hidden. | Main window and query popover. |
| `query-busy` | `()` | `keyboard.rs`, `query_flow.rs` | A query press is refused because another pass or pipeline owner is active. | Reserved for UI feedback. |

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
| `app-event` | `AppEvent` | `telemetry.rs` (`TauriEmitterLayer`) | For **every** `tracing` event in the Rust backend. | Log viewer (`useEventStore`). Release `pipeline` strings are stripped; `transform` and `meeting` strings are restricted by key **and** value to explicit stable vocabularies in all builds. |

Transcript stage telemetry uses the versioned stage vocabulary from
`PerformanceStageV1`, including `spokenStructure`. It carries only stage
identity, timing, outcome, and changed state; transcript and command content
remain excluded.

Live dictation lifecycle telemetry is separately correlated by a positive
`recording_id` and the stable codes `pipeline.dictation_requested`,
`audio.capture_started`, `audio.capture_ready`,
`pipeline.dictation_stop_handoff`, and `pipeline.dictation_terminal`. Audio
stages must also carry `owner_kind: "dictation"`. The terminal event contains
only an allowlisted `outcome`, an allowlisted `error_code`, numeric
`char_count`, and the correlation ID. Release stripping preserves those exact
vocabularies while dropping arbitrary string values. See
[Transcription Pipeline](../features/transcription.md#per-recording-lifecycle-telemetry)
for stage denominators and terminal semantics.

## Updater

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `check-for-updates-requested` | `()` | Native tray menu | The user selects `Check for Updates…` or the versioned update action. The main window is shown and focused first. | Main `App` → manual `checkForUpdate`. |
| `updater-background-check-requested` | `()` | `commands/tray.rs` | `NSWorkspaceDidWakeNotification` fires after macOS wakes. | `useAutoUpdater`; a local six-hour gate decides whether any network request is due. |

## Frontend-emitted (window to window)

| Event | Payload | Source | When it fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `appearance-changed` | `{revision, reason}` where `reason` is `"user"` \| `"repair"` \| `"reset"` \| `"import"` | Main window appearance controller | After main commits a user edit, reset, import, derived-cache repair, or explicit high-water revision rollover to `1` in `murmur-appearance`. The event carries no colors or file contents. System-mode OS changes do not emit. | Log viewer appearance controller; it reloads storage with a bounded retry, ignores stale revisions, and accepts the single high-water-to-`1` repair rollover. |
| `settings-changed` | `()` | `useSettings`, `useOverlaySettingsMirror` | A window mutates persisted settings, so the other windows re-read localStorage. | Main window, overlay. |
| `open-settings` | `()` | `useOverlaySettingsMirror` | The overlay's quick-settings card asks the main window to open Settings. | Main window (`useOpenSettingsListener`). |

**Dead listener:** `useShowAboutListener` listens for `show-about`, but the tray
menu has no About item (it is Show Murmur / Check for Updates / Disable Murmur /
Quit), so nothing emits it.

---

## Event payload types

### AppEvent

```typescript
interface AppEvent {
  timestamp: string;              // ISO timestamp
  stream: StreamName;             // tracing target
  level: LevelName;
  summary: string;                // the tracing message
  data: Record<string, unknown>;  // structured fields after privacy stripping;
                                  // high-value outcomes may include allowlisted event_code
}

type StreamName = 'pipeline' | 'audio' | 'keyboard' | 'transform' | 'meeting' | 'query' | 'system';
type LevelName  = 'trace' | 'debug' | 'info' | 'warn' | 'error';
```

Streams correspond to Rust tracing targets; levels to standard tracing severities. Color mappings for both live in `app/src/lib/events.ts`.

### AppearanceChangedEvent

```typescript
interface AppearanceChangedEvent {
  revision: number;
  reason: 'user' | 'repair' | 'reset' | 'import';
}
```

Main is the only emitter and themed runtime. It handles
`matchMedia('(prefers-color-scheme: dark)')` locally while the saved mode is
System, so OS transitions do not emit appearance events.
