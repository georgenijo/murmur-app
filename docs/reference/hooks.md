# React Hooks Reference

The 28 custom React hooks under `app/src/lib/hooks/`, grouped by the window that uses them. Hooks are where nearly all frontend behavior lives — `App.tsx` and `OverlayWidget.tsx` are thin composition shells.

For the commands these hooks call see [commands.md](commands.md). For the events they subscribe to see [events.md](events.md). For settings managed by `useSettings` see [settings.md](settings.md).

---

## Recording and dictation (main window)

### `useRecordingState`
The dictation state machine. Owns `status` (`idle` / `recording` / `processing`), audio level, locked mode, and the error/hint banners. Subscribes to `recording-status-changed`, `transcription-complete`, `recording-cancelled`, `audio-level`, `auto-paste-failed`, and `file-output-failed`; invokes `start_native_recording` / `stop_native_recording` / `cancel_native_recording`.

**`transcription-complete` is the single source of truth** for history and stats — entries are never added in `handleStop()`, because the overlay can start a recording independently and that would double-count.

`statusRef` mirrors `status` so hotkey callbacks always read current state instead of a stale closure capture; callbacks themselves are stored in refs so listener setup doesn't re-run on identity changes.

### `useHoldDownToggle`
Hold-down mode. Listens for `hold-down-start` / `hold-down-stop`, plus `keyboard-listener-error` for error recovery and auto-restart after 2s. Gated by `enabled`.

### `useDoubleTapToggle`
Double-tap mode. Listens for `double-tap-toggle` and syncs `recording` back into the detector via `set_keyboard_recording`. Gated by `enabled`.

### `useCombinedToggle`
Both modes at once. `holdActiveRef` prevents the double-tap path from firing on hold release; calls `cancel_native_recording` to discard speculative recordings from short taps. Gated by `enabled`.

All three are always called (Rules of Hooks) and switched by the `enabled` prop rather than conditional invocation.

### `useFileTranscription`
Imported-file transcription: file selection, `transcribe_file`, and the `file-transcription-status-changed` busy state. Adds the result to history through the same `addEntry` path as live dictation.

### `useHistoryManagement`
Transcription history with localStorage persistence (`dictation-history`, max 50 entries), clear-with-confirmation, and the Correct-and-Teach entry point on the newest entry.

---

## Configuration and lifecycle (main window)

### `useSettings`
Loads and persists `Settings` to localStorage, pushes the backend-relevant subset to `configure_dictation`, and reconciles `launchAtLogin` with the actual OS autostart state on mount. Updates are optimistic with rollback: if `configure_dictation` fails, the affected fields revert, and a versioned configure ref prevents a stale rollback from clobbering newer settings. Emits `settings-changed` so the overlay re-reads.

### `useInitialization`
One-time init sequence on mount: `init_dictation`, then `configure_dictation` with the loaded settings.

### `useAutoUpdater`
OTA updates: background check on launch and every 24h, semver comparison, min-version enforcement (forced updates drop Skip/Later), skip/dismiss persistence, download progress, install, and auto-relaunch. Fires a native macOS notification when a background check finds an update.

Returns `{updateStatus, checkForUpdate, startDownload, skipVersion, dismissUpdate}`. Reads `min_version` from the `latest-v2.json` channel; persists `skipped-update-version` and `updater-last-check` to localStorage.

### `useOpenSettingsListener`
Listens for `open-settings` from the overlay's gear button and opens the Settings panel — showing the main window isn't enough, since panel visibility is local React state.

### `useOverlaySettingsSync`
The other direction: listens for `settings-changed` emitted by the overlay's quick controls and applies the persisted settings to main-window state and the backend.

### `useShowAboutListener`
Listens for `show-about`. **Currently inert** — the tray menu no longer has an About item, so nothing emits this event.

---

## Selected-text transform

### `useTransformFlow` (main window)
Drives the transform hold key. Listens for `transform-key-pressed` / `transform-key-released` and calls `start_transform_capture` / `finish_transform_instruction`, carrying the `transformPassId` from the event rather than re-deriving it. Pure press/release reduction lives in `lib/transformFlow.ts`.

### `useEscapeCancel` (main window)
Listens for `escape-cancel` and issues a **scoped** `cancel_transform` with the pass ID snapshotted at key-press time, so a delayed Escape cannot cancel the pass that replaced it. Ready/failed reviews keep their own popover-local Escape handling.

### `useTransformReviewDriver` (popover window)
The review popover's state machine. Subscribes to `transform-state-changed`, re-fetching `get_transform_review_content` on each transition (content is never carried in the event payload), and exposes approve / retry / cancel / undo. Also handles `transform-apply-failed` and `transform-review-hidden`.

### `useTransformReviewMockDriver` (dev only)
Demo driver reachable via `?mock=1` (or `?mock=<state>`) on the popover URL, so the whole review UI can be exercised without a backend. Gated on `import.meta.env.DEV` by its caller; never in a production code path.

---

## Overlay window

### `useOverlayGeometry`
Owns geometry sourced from Rust — the single source of truth. Fetches `get_overlay_geometry` with a retry/backoff schedule (a single failed fetch used to leave the transparent overlay blank until the next display change) and re-reads on `overlay-geometry-changed`.

### `useOverlayExpansion`
The hover-expand lifecycle, owned end to end: 150ms dwell intent gate, cursor polling, and the **only** writer to the native resize path (`set_overlay_expanded`). Awaits the applied frame before revealing the dropdown, so CSS never animates into a window that hasn't grown yet. Treats `overlay-geometry-changed` as an authoritative reset — cancels timers, forces collapsed, issues one corrective resize.

### `useOverlayRuntime`
Overlay runtime flashes and mirrors: cancelled and hotkey-miss timers, the `transform-busy` and `transform-secure-field` flashes, and the `app-disabled-changed` state mirror.

### `useOverlaySettingsMirror`
The overlay's snapshot of persisted settings (read straight from localStorage — there is no shared React context across windows) plus the quick-control actions: auto-paste toggle, global disable, and `open-settings`.

### `useRecordingControls`
Click disambiguation on the island: single click (250ms debounce) stops recording or exits locked mode; double-click toggles locked mode.

### `useWaveform`
Owns the `audio-level` listener and the rAF bar-height animation. Writes bar heights through direct DOM refs, bypassing React reconciliation to hold 60fps. Center bars are taller (envelope shaping) with jitter for organic movement.

---

## Diagnostics (log viewer)

### `useEventStore`
The structured event buffer: hydrates from `get_event_history`, streams live `app-event`s, batches rendering via rAF, and provides stream/level filtering and clear.

### `usePerformanceDiagnostics`
Run history and resource samples. Hydrates from `list_performance_runs` / `get_performance_resource_window`, then merges live `performance-run-completed` and `performance-resource-sample` events (the exported `mergeRuns` / `mergeResourceSamples` are pure and unit-tested), and resets on `performance-diagnostics-cleared`. Gated by an `enabled` flag so a hidden tab does no work.

### `usePerformanceHealth`
Summary health of the diagnostics store (availability, counts) with an explicit `refresh`.

### `useResourceMonitor`
CPU/memory polling with a rolling 60-reading buffer. Only polls while the panel is expanded.

---

## Settings surfaces

### `useKnowledge`
Bounded, paged access to the personal knowledge store (`list_knowledge`) with search/filter, driven by a request object and an `active` gate so closed panels don't query.

### `useVocabScan`
Live code-vocabulary scan: starts `scan_code_vocab`, streams `vocab-scan-progress` correlated by `scanId`, and reports non-adoption when a newer scan or a settings change supersedes the walk.
