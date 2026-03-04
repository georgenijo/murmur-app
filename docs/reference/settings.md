# User Settings Reference

This document describes all 9 user-configurable settings in Murmur. Settings are managed on the frontend by the `useSettings` hook and persisted to `localStorage`. Relevant settings are pushed to the Rust backend via the `configure_dictation` command.

For the hook that manages settings, see [hooks.md](hooks.md). For the backend command that receives configuration, see [commands.md](commands.md).

---

## Settings Overview

All settings are stored in `localStorage` under the key `dictation-settings` as a single JSON object.

**Source file:** `app/src/lib/settings.ts`

**TypeScript interface:**

```typescript
interface Settings {
  model: ModelOption;
  doubleTapKey: DoubleTapKey;
  language: string;
  autoPaste: boolean;
  autoPasteDelayMs: number;
  recordingMode: RecordingMode;
  microphone: string;
  launchAtLogin: boolean;
  vadSensitivity: number;
}
```

---

## Transcription Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `model` | `ModelOption` | `'base.en'` | `'tiny.en'`, `'base.en'`, `'small.en'`, `'medium.en'`, `'large-v3-turbo'` | The transcription model to use. All models use the Whisper backend with Metal GPU acceleration. See the model options table below. |
| `language` | `string` | `'en'` | Any language code string | Transcription language. Passed to the Whisper backend. No validation on the frontend. |

### Model Options

| Value | Label | Size | Backend |
|-------|-------|------|---------|
| `tiny.en` | Whisper Tiny (English) | ~75 MB | Whisper (Metal GPU) |
| `base.en` | Whisper Base (English) | ~150 MB | Whisper (Metal GPU) |
| `small.en` | Whisper Small (English) | ~500 MB | Whisper (Metal GPU) |
| `medium.en` | Whisper Medium (English) | ~1.5 GB | Whisper (Metal GPU) |
| `large-v3-turbo` | Whisper Large Turbo | ~3 GB | Whisper (Metal GPU) |

Both the Rust-side `DictationState::default()` and the frontend default use `base.en`.

---

## Recording Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `recordingMode` | `RecordingMode` | `'hold_down'` | `'hold_down'`, `'double_tap'`, `'both'` | How recording is triggered via keyboard. Hold-down: press-and-hold to record. Double-tap: double-tap to start, single-tap to stop. Both: combined mode with deferred hold promotion. |
| `doubleTapKey` | `DoubleTapKey` | `'shift_l'` | `'shift_l'` (Shift), `'alt_l'` (Option), `'ctrl_r'` (Control) | The modifier key used for recording triggers. Used by all three recording modes as the trigger key. Label in the settings UI changes based on `recordingMode`. |
| `vadSensitivity` | `number` | `50` | 0-100, step 5 in UI | Voice Activity Detection sensitivity. Higher values keep more audio; lower values trim silence more aggressively. The backend converts this to a threshold: `1.0 - (sensitivity / 100.0)`. Clamped to 0-100 by the backend. |

### Recording Mode Details

| Mode | Trigger Key Label | Behavior |
|------|------------------|----------|
| `hold_down` | "Hold Key" | Hold to start recording, release to stop and transcribe. |
| `double_tap` | "Double-Tap Key" | Double-tap to start recording, single-tap to stop. |
| `both` | "Trigger Key" | Hold to record (with 200ms promotion delay), or double-tap to start and single-tap to stop. |

---

## Output Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `autoPaste` | `boolean` | `false` | `true` / `false` | Whether to automatically paste transcribed text after copying it to the clipboard. Requires macOS Accessibility permission. Text is always copied to the clipboard regardless of this setting. |
| `autoPasteDelayMs` | `number` | `50` | 10-500 ms, step 10 in UI | Delay in milliseconds before auto-paste fires, to allow window focus to settle. The backend clamps this value to the 10-500 range. The UI slider only appears when `autoPaste` is enabled. |

---

## System Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `microphone` | `string` | `'system_default'` | `'system_default'` or any device name from `list_audio_devices` | Audio input device for recording. When set to `'system_default'`, the frontend sends `null` to the backend, which uses the system default input device. Available devices are fetched via the `list_audio_devices` command when the settings panel opens. |
| `launchAtLogin` | `boolean` | `false` | `true` / `false` | Whether the app starts automatically on macOS login. Uses `@tauri-apps/plugin-autostart` with `MacosLauncher::LaunchAgent`. On mount, the hook checks the actual OS autostart state and reconciles with the stored setting (handles the case where the user removed the login item from System Settings). |

---

## Persistence and Migration

### Storage

- **Key:** `dictation-settings`
- **Method:** `localStorage.setItem` / `localStorage.getItem`
- **Format:** Full `Settings` object serialized as JSON

### Loading Behavior

`loadSettings()` performs the following:
1. Reads from `localStorage` under `dictation-settings`.
2. If found, parses as JSON and merges with `DEFAULT_SETTINGS` (stored values override defaults).
3. Applies migration: if `recordingMode` is missing or invalid (including the legacy `'hotkey'` value), resets to `'hold_down'`.
4. Strips the legacy `hotkey` field if present.
5. If not found or on parse error, returns `DEFAULT_SETTINGS`.

### Backend Synchronization

When settings change, `useSettings.updateSettings` pushes the following fields to the Rust backend via `configure_dictation`:

| Frontend Field | Backend Field | Sent On Change |
|----------------|--------------|----------------|
| `model` | `model` | Yes |
| `language` | `language` | Yes |
| `autoPaste` | `autoPaste` | Yes |
| `autoPasteDelayMs` | `autoPasteDelayMs` | Yes |
| `vadSensitivity` | `vadSensitivity` | Yes |
| `doubleTapKey` | _(sent via `update_keyboard_key`)_ | Via keyboard hooks |
| `recordingMode` | _(controls which hook is active)_ | Frontend only |
| `microphone` | _(sent as param to `start_native_recording`)_ | Per recording |
| `launchAtLogin` | _(sent via autostart plugin)_ | Via OS API |

**Optimistic updates with rollback:** If `configure_dictation` fails, the affected settings (model, language, autoPaste, autoPasteDelayMs, vadSensitivity) revert to their previous values. Similarly, if the autostart toggle fails, `launchAtLogin` reverts. A versioned configure ref prevents stale rollbacks from overwriting newer settings.

---

## Related localStorage Keys

Other data persisted to localStorage by the application (not part of the `Settings` object):

| Key | Purpose | Used By |
|-----|---------|---------|
| `dictation-history` | Transcription history entries (max 50) | `useHistoryManagement` |
| `dictation-stats` | Cumulative transcription statistics | `lib/stats.ts` |
| `skipped-update-version` | Version string the user chose to skip | `useAutoUpdater` |
| `updater-last-check` | Timestamp of last update check | `useAutoUpdater` |
| `resource-monitor-collapsed` | Whether the resource monitor panel is collapsed | ResourceMonitor component |
