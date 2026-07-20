# User Settings Reference

This document describes Murmur's user-configurable settings. Settings are managed on the frontend by the `useSettings` hook and persisted to `localStorage`. Relevant settings are pushed to the Rust backend via the `configure_dictation` command.

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
  hotkeyMissFeedback: boolean;
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
| `hotkeyMissFeedback` | `boolean` | `false` | `true` / `false` | In Double-Tap or Both mode, briefly flashes the overlay amber when the 400ms second-tap window expires. It does not fire for holds, modifier shortcuts, processing skips, or successful gestures. Frontend/overlay only. |
| `liveTranscriptPreview` | `boolean` | `true` | `true` / `false` | Shows session-scoped provisional Whisper text below the physical notch during long recordings. Parakeet/Core ML are final-only and surface that limitation explicitly. Provisional text remains memory-only and never enters delivery, history, files, stats, or logs. |
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
| `saveTranscript` | `boolean` | `false` | `true` / `false` | When enabled, each live dictation's transcript is written to a sequentially numbered `.txt` (`murmur-0001`, `murmur-0002`, …) in the output folder. When `saveTranscript` or `saveAudio` is on, auto-paste is suppressed (clipboard copy still happens). |
| `saveAudio` | `boolean` | `false` | `true` / `false` | When enabled, each live dictation's audio is written to a matching `.wav` (16kHz mono, 16-bit PCM) in the output folder. |
| `outputDir` | `string` | `''` | Any absolute folder path, or `''` for default | Destination for saved transcript/audio files. Empty means the app default (`Documents/Murmur`, created on first write). Set via a folder picker (`dialog:allow-open`). |

## Vocabulary Settings

`vocabularyEntries` is an array of `{ id, written, aliases, enabled, scope }`. `written` is the canonical surface form used by Whisper prompt bias and post-model correction. `aliases` contains exact spoken variants applied locally on every backend. Settings currently creates `{ kind: 'global' }` scopes; typed app/project scopes are selected from the existing immutable dictation context. The legacy `customVocabulary` string is migration-only and is re-derived from enabled global canonical terms.

Aliases are limited to 16 per entry and values to 256 characters. Ambiguous aliases, canonical collisions, Voice Command collisions, and direct or indirect cycles are rejected atomically.

## Per-App Profiles

`appProfiles` is an array of `{ bundleId, label, writingStyle, autoPasteOverride, cleanupOverride, smartFormattingOverride, cliFormattingOverride, ideContextEnabled, ideProjectRoots }`. `writingStyle` is `null` (Inherit), `conversational`, `polished`, `code_technical`, `verbatim`, or `notes`. It is an explicit user choice; bundle identifiers and labels never classify apps automatically. Boolean overrides fine-tune the resolved style/global value for a matching frontmost bundle identifier; `null` means "inherit." Existing, missing, and malformed persisted style/override fields migrate to `null`.

`ideContextEnabled` defaults to `false` and must be enabled on the exact matching profile. `ideProjectRoots` persists only the explicit user-selected root strings, trimmed, deduplicated, and capped at four. Filenames, symbols, source snippets, and scan results are memory-only and are not settings fields. The roots therefore remain visible in Settings and in any direct inspection or backup of the existing settings JSON; there is no hidden export path.

`smartFormattingEnabled` is a separate boolean setting, off by default. It enables deterministic list, explicit structured-token, and bounded same-utterance correction rules for live prose. Missing or malformed persisted values migrate safely to `false`; it is independent of `smartPunctuation`. `smartFormattingOverride` gives profiles the same Default/On/Off choice.

`cliFormattingOverride` uses the immutable recording-start context. `true` enables profile-mode CLI recognition, `false` disables implicit CLI formatting for that app, and `null` keeps conservative automatic recognition. An explicit spoken `command` trigger remains available in every mode.

At recording start, the backend resolves one immutable context using global settings → matching style → matching profile fine-tuning → one-session overrides. Settings or focus changes during recording apply only to the next session. Explicit IDE opt-in also disables Smart Formatting for that recording and can capture only the matching profile's fresh local index. See [Per-App Dictation Context](../features/per-app-profiles.md) and [Local IDE Symbols and `@file` Context](../features/ide-context.md).

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
2. If found, parses as JSON and merges with `DEFAULT_SETTINGS` (stored values override defaults). Legacy comma/newline-separated `customVocabulary` values migrate to enabled global `vocabularyEntries` with no aliases.
3. Applies migration: if `recordingMode` is missing or invalid (including the legacy `'hotkey'` value), resets to `'hold_down'`.
4. Strips the legacy `hotkey` field if present.
5. Validates `model` against the current allow-list. Any invalid or removed model (e.g. `moonshine-tiny`, `moonshine-base`) is reset to `'base.en'`.
6. If not found or on parse error, returns `DEFAULT_SETTINGS`.

### Backend Synchronization

When settings change, `useSettings.updateSettings` pushes the following fields to the Rust backend via `configure_dictation`:

| Frontend Field | Backend Field | Sent On Change |
|----------------|--------------|----------------|
| `model` | `model` | Yes |
| `language` | `language` | Yes |
| `autoPaste` | `autoPaste` | Yes |
| `autoPasteDelayMs` | `autoPasteDelayMs` | Yes |
| `vadSensitivity` | `vadSensitivity` | Yes |
| `saveTranscript` | `saveTranscript` | Yes |
| `saveAudio` | `saveAudio` | Yes |
| `outputDir` | `outputDir` | Yes |
| `doubleTapKey` | _(sent via `update_keyboard_key`)_ | Via keyboard hooks |
| `recordingMode` | _(controls which hook is active)_ | Frontend only |
| `hotkeyMissFeedback` | _(controls overlay rejection feedback)_ | Frontend only |
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
