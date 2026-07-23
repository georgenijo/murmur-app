# Tauri Events Reference

This document lists all events emitted from the Rust backend to the frontend via Tauri's event system. The frontend subscribes to these events using `listen()` from `@tauri-apps/api/event`.

For commands invoked from the frontend to the backend, see [commands.md](commands.md). For hooks that consume these events, see [hooks.md](hooks.md).

---

## Recording and Transcription Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `audio-level` | `f32` (RMS value, 0.0-1.0) | `audio.rs` | Continuously during recording, throttled to ~60fps (16ms minimum gap between emissions). | Overlay window (waveform visualization), main window (`useRecordingState` stores in `audioLevel` state). |
| `recording-status-changed` | `string` (`"idle"`, `"recording"`, `"processing"`) | `commands/recording.rs` | At every dictation state transition: start recording, stop recording, begin processing, finish processing. | Main window (`useRecordingState` syncs status), overlay window (drives visual state). |
| `transcription-complete` | `{text: string, duration: number}` | `commands/recording.rs` | After successful transcription produces non-empty text. Broadcast to all windows. Duration is in whole seconds (integer division). | Main window (`useRecordingState` updates history, stats, and transcription display). |
| `auto-paste-failed` | `string` (hint message, e.g., "Text is in your clipboard -- press Cmd+V to paste manually.") | `commands/recording.rs` (via `injector.rs`) | When auto-paste fails or times out (2-second timeout). Text is already in the clipboard. | Main window (`useRecordingState` shows error for 5 seconds then auto-clears). |

## Model Download Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `download-progress` | `{received: number, total: number}` (byte counts) | `commands/models.rs` | Periodically during model and VAD model streaming downloads. `total` may be 0 if the server does not provide `Content-Length`. | Main window (SettingsPanel download progress bar, ModelDownloader progress bar). |

## Keyboard Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `double-tap-toggle` | `()` (empty) | `keyboard.rs` | When the double-tap detector recognizes a valid double-tap sequence on the trigger key. In "both" mode, emitted on key release when the hold was not promoted but the double-tap sequence completed. | Main window (`useDoubleTapToggle` calls `onToggle`, `useCombinedToggle` calls `onToggle`). |
| `hold-down-start` | `()` (empty) | `keyboard.rs` | When the hold-down detector recognizes a key press. In hold-down-only mode, emitted immediately on key press. In "both" mode, emitted after the 200ms promotion timer confirms the key is still held. | Main window (`useHoldDownToggle` calls `onStart`, `useCombinedToggle` calls `onStart`). |
| `hold-down-stop` | `()` (empty) | `keyboard.rs` | When the hold-down key is released (after a valid hold). Also emitted by `update_keyboard_key` if the hotkey is changed while the key is held down, to prevent stuck recording state. | Main window (`useHoldDownToggle` calls `onStop`, `useCombinedToggle` calls `onStop`). |
| `hotkey-tap-rejected` | `{ reason: "second_tap_expired", mode: "double_tap" \| "both" }` | `keyboard.rs` | When an idle first tap is not followed by a second tap within 400ms. Emitted at timer expiry; never emitted for holds, combos, processing skips, or valid double-taps. | Overlay window (shows the amber timing-miss flash only when `hotkeyMissFeedback` is enabled). |
| `keyboard-listener-error` | `string` (error message) | `keyboard.rs` | When the rdev listener thread encounters an error. | Main window (all three keyboard hooks listen; on error, they wait 2 seconds then attempt to restart the listener). |

**Note on `hold-down-cancel`:** The frontend `useCombinedToggle.ts` registers a listener for the event name `hold-down-cancel`, but this event is never emitted from any Rust code. In "both" mode, short taps that are not promoted to holds simply emit nothing -- the recording was never started. The frontend listener is dead code.

## Overlay Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `overlay-geometry-changed` | `OverlayGeometry` (never null) | `commands/overlay.rs` | When display configuration changes (monitor plug/unplug, lid open/close). Triggered by an NSNotificationCenter observer watching `NSApplicationDidChangeScreenParametersNotification`; carries the recomputed geometry contract (a synthetic fallback notch substitutes when none is detected, so the payload is never null). | Overlay window: `useOverlayGeometry` updates the geometry it renders from; the expansion controller (`useOverlayExpansion`) treats this as an authoritative reset — it cancels timers, forces `collapsed`, and issues one corrective collapse resize. |
| `overlay-visible-changed` | `boolean` | `commands/overlay.rs` | After `show_overlay` (`true`) / `hide_overlay` (`false`). **Not currently invoked in production** — the overlay is shown once at setup (`overlay_win.show()` in `lib.rs`) and stays visible for the app's lifetime, so this event has no live emitter today. | Overlay window: gates the expansion controller's cursor poller so it performs no IPC while hidden. Defaults to visible on mount, so first-hover works even though nothing emits this yet. |

## Transform Review Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `transform-state-changed` | `{state: "listening" \| "thinking" \| "ready" \| "failed" \| "applied", errorCode?: "model_not_downloaded" \| "timeout" \| "output_invalid" \| "crashed"}` | Not yet emitted — contract locked in PR-C1 (`lib/transformReview.ts`), real emitter arrives with PR-C2's transform pipeline. | Every review-state-machine transition. Deliberately carries no instruction/original/proposed text (fetched separately via `get_transform_review_content`) so potentially sensitive text is never broadcast as an event payload. | Transform review popover window (`useTransformReviewDriver` sets state/errorCode and re-fetches content). |

## Structured Logging Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `app-event` | `AppEvent {timestamp: string, stream: StreamName, level: LevelName, summary: string, data: Record<string, unknown>}` | `telemetry.rs` (TauriEmitterLayer) | For every `tracing` event in the entire Rust backend. Every log statement becomes a structured event. | Log viewer window (`useEventStore` appends to buffer). Release `pipeline` strings are stripped; `transform` strings are always restricted by key and value to explicit stable enum/bucket vocabularies. |

## Tray Menu Events

| Event | Payload | Source | When It Fires | Listeners |
|-------|---------|--------|---------------|-----------|
| `show-about` | `()` (empty) | `lib.rs` (tray menu setup) | When the user selects the "About" item from the tray menu (if present). | Main window (`useShowAboutListener` sets `showAbout` state to `true`, opening the AboutModal). |

---

## Event Payload Types

### AppEvent

```typescript
interface AppEvent {
  timestamp: string;        // ISO timestamp
  stream: StreamName;       // "pipeline" | "audio" | "keyboard" | "transform" | "system"
  level: LevelName;         // "trace" | "debug" | "info" | "warn" | "error"
  summary: string;          // The tracing message
  data: Record<string, unknown>;  // Structured fields from the tracing event
}
```

### Stream and Level Types

```typescript
type StreamName = 'pipeline' | 'audio' | 'keyboard' | 'transform' | 'system';
type LevelName = 'trace' | 'debug' | 'info' | 'warn' | 'error';
```

Streams correspond to Rust tracing targets. Levels correspond to standard tracing severity levels. Color mappings for both streams and levels are defined in `app/src/lib/events.ts`.
