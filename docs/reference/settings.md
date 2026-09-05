# User Settings Reference

This document describes Murmur's user-configurable settings. Settings are managed on the frontend by the `useSettings` hook, persisted durably to a Rust-owned `settings.json`, and cached in `localStorage` for synchronous reads. Relevant settings are pushed to the Rust backend via the `configure_dictation` command.

For the hook that manages settings, see [hooks.md](hooks.md). For the backend command that receives configuration, see [commands.md](commands.md).

---

## Settings Overview

Dictation settings are one JSON object, stored durably in `settings.json` and
cached in `localStorage` under the key `dictation-settings`. Appearance is an
independent versioned active document under `murmur-appearance`, with saved
themes in `murmur-theme-library`; neither is merged into this interface or
emitted through `dictation-settings`.

The native Settings workspace opens on **Customize**, an ordered hub with four
rows: **Text & Vocabulary**, **Voice Commands**, **Styles**, and **Transforms**.
The gear button, `⌘,`, and overlay Settings action all open this same overview.
Each row opens the existing owning page or editor, shows **Back to Customize**,
and restores focus to its source row on return. Contextual links from Home use
the same routes, so they cannot leave the user trapped in a detail page.

The persistent navigation (`SETTINGS_CATEGORIES` in `SettingsPanel.tsx`) keeps
the complete workspace available: Customize, General, Recording, Delivery,
Meetings, Text & Vocabulary, AI & Models, and Appearance. Performance Lab and
Diagnostics remain in the Tools group. The search field routes matching rows
to their owning page. Power-user controls are collapsed under **Advanced**
disclosures.

Modes may optionally activate from exact browser hosts under Delivery →
Advanced → Modes. The global switch is off by default. Each rule stores only a
rule ID, allowed browser bundle ID, exact host, Mode ID, and enabled flag. Live
URL identity is memory-only and reduced to the host before it crosses the
native boundary; paths, page text, selections, clipboard content, and browsing
history are never part of Settings.

Changing pages only changes presentation. It does not rename, discard, or
reinterpret persisted fields. A round-trip compatibility test serializes and
reloads every current `Settings` field, including tri-state app overrides and
IDE roots.

**Source file:** `app/src/lib/settings.ts`

The live microphone monitor is operational state, not a setting. It starts
automatically on Recording Settings, uses the persisted `microphone` ID, keeps
no audio, yields to real dictation, resumes afterward, and stops when Dictation
Settings is left or hidden. See
[Microphone Input Test](../features/microphone-input-test.md).

The microphone list is operational state too. One Rust-owned schema-v1 snapshot
is invalidated by a passive signed-worker Core Audio listener, refreshed at
startup, and guarded by a bounded five-minute backend fallback, never by
Settings focus. Refresh work coalesces and defers through every capture-owned
phase until joined-worker `Idle`. Settings accepts only strictly validated,
same-or-newer revisions; a failed refresh keeps the previous list visibly stale
but cannot prove an explicit device is available. Display names and stable IDs
stay local, while shipped state receives only a count/default-available/success
aggregate.

**TypeScript interface** (full current shape — see `settings.ts` for the per-field comments):

```typescript
interface Settings {
  // Transcription
  model: ModelOption;
  language: string;
  vadSensitivity: number;
  idleTimeoutMinutes: number;

  // Recording
  recordingMode: RecordingMode;
  doubleTapKey: DoubleTapKey;
  hotkeyMissFeedback: boolean;
  autoStopSilenceMs: number;               // 0 = off (default)
  microphone: string;
  microphoneIdMigrationComplete: boolean;  // durable proof marker
  disabled: boolean;

  // Transform (selected-text rewrite)
  transformHoldKey: TransformKey | null;   // null = disabled (default)

  // Voice Query (configured CLI)
  queryHotkey: QueryKey | null;            // null = disabled (default)
  queryProvider: QueryProviderId;           // preset metadata; default "custom"
  queryExecutable: string;                 // absolute executable path
  queryArguments: string[];                // fixed argv before question
  queryTimeoutSeconds: number;             // 5–300, default 60
  queryContextLevel: QueryContextLevel;     // none | application | selection
  queryAutomaticallyCopyAnswers: boolean;  // default true; immutable per pass
  retainQueryHistory: boolean;             // opt-in local question/answer history

  // Delivery
  autoPaste: boolean;
  autoPasteDelayMs: number;
  pasteLastShortcut: PasteLastShortcut | null;
  correctionShortcutEnabled: boolean;
  retainHistory: boolean;
  meetingRetainAudio: boolean;
  meetingEchoCancellationEnabled: boolean;
  meetingRetentionDays: number;            // 0 = no age limit
  meetingMaxSessions: number;
  soundCuesEnabled: boolean;
  soundCueVolume: number;
  meetingSoundCuesEnabled: boolean;
  saveTranscript: boolean;
  saveAudio: boolean;
  mirrorToNotchPill: boolean;
  outputDir: string;

  // Text intelligence
  smartPunctuation: boolean;
  cleanupEnabled: boolean;
  cleanupRemoveFiller: boolean;
  cleanupCapitalize: boolean;
  smartFormattingEnabled: boolean;
  correctionEnabled: boolean;
  correctionFuzzy: boolean;
  voiceCommandsEnabled: boolean;
  voiceCommands: VoiceCommand[];           // legacy pairs, migration-only
  vocabularyEntries: VocabularyEntry[];
  customVocabulary: string;                // @deprecated derived mirror
  codeVocabEnabled: boolean;
  codeVocabFolder: string;
  codeVocabLastScan: VocabScanSummary | null;

  // Per-app
  modes: MurmurMode[];                     // user Modes; built-ins are code-owned
  activeModeId: string;                    // last native/manual Mode selection
  appProfiles: AppProfile[];

  // Performance Lab
  benchmarkOutputDir: string;
  benchmarkAutoSave: boolean;

  // System
  launchAtLogin: boolean;
  overlayVerticalOffset: number;           // integer points, -12 through +12
}
```

---

## Transcription Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `model` | `ModelOption` | Core ML Parakeet v3 | Seven catalog identifiers listed below | The exact transcription model to use. Unknown identifiers fail closed; Murmur does not automatically choose another model. |
| `language` | `string` | `'en'` | Any language code string | Transcription language. The runtime capability catalog disables language selection for English-only models. |

### Model Options

| Value | Label | Size | Backend |
|-------|-------|------|---------|
| `parakeet-tdt-0.6b-v3-coreml` | Parakeet Core ML | ~470 MB | FluidAudio (Apple Neural Engine) |
| `parakeet-tdt-0.6b-v2-fp16` | Parakeet TDT 0.6B (English, fast) | ~1.2 GB | sherpa-onnx (CPU) |
| `tiny.en` | Whisper Tiny (English) | ~75 MB | Whisper (Metal GPU) |
| `base.en` | Whisper Base (English) | ~150 MB | Whisper (Metal GPU) |
| `small.en` | Whisper Small (English) | ~500 MB | Whisper (Metal GPU) |
| `medium.en` | Whisper Medium (English) | ~1.5 GB | Whisper (Metal GPU) |
| `large-v3-turbo` | Whisper Large Turbo | ~3 GB | Whisper (Metal GPU) |

New installs default to Core ML. Rust initializes `base.en` until the frontend
applies the persisted default. Runtime capabilities, install state, and
lifecycle state are not settings and are never persisted to localStorage.
Settings exposes this through the single model selector and marks the supported
Core ML entry Recommended; there is no second accelerator switch with hidden
model-selection side effects.

---

## Recording Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `recordingMode` | `RecordingMode` | `'hold_down'` | `'hold_down'`, `'double_tap'`, `'both'` | How recording is triggered via keyboard. Hold-down: press-and-hold to record. Double-tap: double-tap to start, single-tap to stop. Both: combined mode with deferred hold promotion. |
| `doubleTapKey` | `DoubleTapKey` | `'shift_l'` | `'shift_l'` (Shift), `'alt_l'` (Option), `'ctrl_r'` (Control) | The modifier key used for recording triggers. Used by all three recording modes as the trigger key. Label in the settings UI changes based on `recordingMode`. |
| `hotkeyMissFeedback` | `boolean` | `false` | `true` / `false` | In Double-Tap or Both mode, briefly flashes the overlay amber when the 400ms second-tap window expires. It does not fire for holds, modifier shortcuts, processing skips, or successful gestures. Frontend/overlay only. |
| `autoStopSilenceMs` | `number` | `0` | `0` (Off), `1500`, `2500`, `4000` | Trailing silence after which a recording stops itself. Applies to any recording **not started by holding the trigger key** (double-tap, button, overlay, locked mode); while the key is physically held the release owns the stop. The detector arms only after it has heard speech, so a silent start never self-terminates. Any value outside the allow-list — including a tampered or absent one — coerces back to Off. Frontend only. See [features/silence-auto-stop.md](../features/silence-auto-stop.md). |
| `vadSensitivity` | `number` | `50` | 0 (Off), 5-100; step 5 in UI | Voice Activity Detection sensitivity. Off skips VAD for the lowest post-release latency but gives up no-speech rejection. For non-zero values, higher values keep more audio and lower values trim silence more aggressively; the backend threshold is `1.0 - (sensitivity / 100.0)`. Clamped to 0-100 by the backend. |
| `disabled` | `boolean` | `false` | `true` / `false` | Global disable. Mirrors the tray "Disable Murmur" check item and the overlay's power button; the hover quick-settings card stays reachable while disabled so the overlay can turn Murmur back on. |
| `idleTimeoutMinutes` | `number` | `5` | `5`, `15`, `0` (Never) | How long an idle loaded model stays resident before the runtime releases it. `0` keeps it loaded indefinitely. |
| `meetingRetainAudio` | `boolean` | `false` | `true` / `false` | Keeps each meeting chunk WAV after its transcript commits. Off deletes the WAV only after the corresponding SQLite transaction succeeds. |
| `meetingEchoCancellationEnabled` | `boolean` | `false` | `true` / `false` | Experimental helper-side speaker-echo reduction for the Me channel. Processing failures bypass to the original microphone stream; Them remains unchanged. |
| `meetingRetentionDays` | `number` | `0` | `0` or 1–3650 days | Age cap applied before starting a meeting; `0` preserves completed sessions by age. |
| `meetingMaxSessions` | `number` | `100` | 1–10,000 | Maximum completed/interrupted sessions retained when pruning before a new meeting. |
| `soundCuesEnabled` | `boolean` | `true` | `true` / `false` | Plays output-only start, stop, delivery-success, and failure cues for dictation. |
| `soundCueVolume` | `number` | `45` | Integer 0–100 | Volume used for runtime cues and Settings previews. |
| `meetingSoundCuesEnabled` | `boolean` | `false` | `true` / `false` | Opts meeting capture into lifecycle cues; recurring meeting cues are suppressed by default. |

### Transform Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `transformHoldKey` | `TransformKey \| null` | `null` | `'alt_r'` (Right Option), `'ctrl_l'` (Left Control), `'shift_r'` (Right Shift), or `null` to disable | The independent hold key for selected-text transform. Deliberately a distinct id set from `doubleTapKey` so the two shortcuts coexist; the picker rejects the active dictation key. Anything unrecognized — including an absent field on pre-feature settings — coerces back to `null` rather than silently arming a shortcut.<br><br>The transform **model**, saved transforms, and presets are not localStorage settings: the model install lives on disk under the app models directory, and saved transforms are knowledge-store records. See [Selected-text Transform](../features/selected-text-transform.md). |

### Voice Query

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `queryHotkey` | `QueryKey \| null` | `null` | `'alt_r'`, `'ctrl_l'`, `'shift_r'`, or `null` | Dedicated double-tap-to-start / single-tap-to-stop shortcut. It may not equal `transformHoldKey`; a persisted conflict disables Voice Query. |
| `queryProvider` | `QueryProviderId` | `'custom'` | `'claude'`, `'codex'`, `'grok'`, `'cursor'`, or `'custom'` | Chooses discovery/auth metadata, recommended argv, and the post-spawn output adapter. Claude recommends bounded safe-mode stream JSON, Codex recommends `exec --json`, and Cursor uses Ask mode with explicit trust for Murmur's private isolated app-data workspace; every provider still uses the generic direct-spawn bridge. When Voice Query is enabled, changing provider temporarily clears the shortcut, validates and preflights the exact new preset command, and restores the same shortcut only if that current check succeeds. Failure leaves it disabled; stale checks cannot re-enable a later provider. Changing provider while already disabled does not enable it. Unknown persisted values fail closed to Custom. |
| `queryExecutable` | `string` | `''` | Absolute executable path, at most 4096 bytes | Exact CLI program to spawn. Murmur never provides a default and never invokes a shell. |
| `queryArguments` | `string[]` | `[]` | At most 32 fixed arguments, 4096 bytes each and 32 KiB total | Passed literally before the locally transcribed question and any opted-in context, which together remain one final argv element. |
| `queryTimeoutSeconds` | `number` | `60` | Integer 5–300 | Deadline after which the owned CLI process group is terminated and confirmed empty. |
| `queryContextLevel` | `QueryContextLevel` | `'none'` | `'none'`, `'application'`, `'selection'` | Opt-in immutable per-pass context. Application adds the native app name and AX window title; Selection additionally includes selected text bounded to 8 KiB on a UTF-8 boundary after secure-field-aware capture. All context stays inside the one final prompt argv element and is summarized visibly in the popover. |
| `queryAutomaticallyCopyAnswers` | `boolean` | `true` | `true` / `false` | Snapshotted at pass start. When true, one current valid terminal success copies its final bounded answer exactly once, unless a later Murmur clipboard write superseded it; clipboard failure leaves the provider outcome Ready. When false, the pass does not read or write the clipboard and the requester-gated review popover's manual Copy action remains available. Failed, cancelled, timed-out, empty, and stale passes never copy. A malformed structured response can remain a Ready raw-fallback answer, but reports `auto_copy_unavailable` and requires manual Copy. Voice Query never auto-pastes. A missing or malformed persisted value migrates to `true` to preserve the behavior shipped before this setting. |
| `retainQueryHistory` | `boolean` | `false` | `true` / `false` | Snapshotted at pass start. When true, every recognized pass stores its original question and answer plus bounded content-free metadata in the separate Rust-owned query-history SQLite store, including passes that appended app context or used raw structured-provider fallback. Context and composed prompts are never stored as separate fields, but a retained answer may quote context sent to a CLI. Stderr/detail, argv, paths, environment, and secrets never enter the store. Turning retention off affects new passes; existing records remain until History → Queries → Delete all query history. |

Declared provider environment values are not settings fields. Rust stores only `CLAUDE_CONFIG_DIR` and/or `CODEX_HOME` under owner-only app data; `HOME`, base allowlist overrides, API keys, tokens, and arbitrary names are rejected. Webviews can stage a replacement value for Save, but saved values are never returned to the frontend or copied into localStorage. Settings receives configured names only.

The Settings disclosure explicitly states that the configured CLI may send the question, enabled context, or answer to cloud services and that Murmur cannot control its network behavior. Enabling validates the configuration immediately; Test runs a bounded preset auth probe and keeps its stdout/stderr within Settings. The known nested Codex macOS-platform-binary `ENOENT` is replaced Rust-side with a concise reinstall/update diagnosis, so its Node stack and install path are not returned. Provider-reported numeric usage is shown per Ready pass and aggregated into the local Stats store by provider and stable failure code; Reset Stats clears those counters. No query content is stored in Stats. The automatic-copy and opt-in query-history settings do not restart a listener or alter the active pass; each changed value applies to the next pass. See [Voice Query](../features/voice-query.md).

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
| `autoPaste` | `boolean` | `false` | `true` / `false` | Stored preference for automatically pasting transcribed text after clipboard copy. Requires macOS Accessibility permission. Text is always copied. When either file-output toggle is on, the UI shows auto-paste unavailable without overwriting this preference. An enabled preference is labeled paused and resumes when file output is off; a disabled preference remains off. |
| `autoPasteDelayMs` | `number` | `0` | 0-500 ms, step 10 in UI | Delay in milliseconds before auto-paste fires, to allow window focus to settle. Zero uses the immediate native-paste fast path; increase it only for apps that move focus asynchronously. The backend clamps this value to the 0-500 range. The UI slider only appears when `autoPaste` is enabled. |
| `correctionShortcutEnabled` | `boolean` | `false` | `true` / `false` | Enables ⌘⇧E to start or finish a spoken correction of the latest delivery. The configured microphone and local transform model are used. |
| `pasteLastShortcut` | `PasteLastShortcut \| null` | `null` | `'command_shift_v'` (⌘⇧V), `'command_option_v'` (⌘⌥V), `'command_control_v'` (⌘⌃V), or `null` to disable | Configures the global Paste Last / Retry Delivery chord. Changes take effect immediately; invalid or Murmur-owned conflicting identifiers fail closed, and native registration failure leaves the last working value selected. Retried text stays memory-only and uses the normal secure delivery checks. Settings v4 migrates the former `pasteLastShortcutEnabled: true` to ⌘⇧V and `false`/absent to Disabled. |
| `retainHistory` | `boolean` | `true` | `true` / `false` | Persist new microphone and imported-file transcripts in the local History workspace. When false, delivery and content-free statistics continue but new transcript content is discarded at the history boundary. Existing entries remain until explicitly cleared. |
| `saveTranscript` | `boolean` | `false` | `true` / `false` | When enabled, each live dictation's transcript is written to a sequentially numbered `.txt` (`murmur-0001`, `murmur-0002`, …) in the output folder. When `saveTranscript` or `saveAudio` is on, auto-paste is suppressed (clipboard copy still happens). |
| `saveAudio` | `boolean` | `false` | `true` / `false` | When enabled, each live dictation's audio is written to a matching `.wav` (16kHz mono, 16-bit PCM) in the output folder. |
| `mirrorToNotchPill` | `boolean` | `false` | `true` / `false` | When enabled, the final transcript of each dictation is mirrored to `~/Library/Application Support/local-dictation/latest-caption.json` as `{text, timestamp}`, so NotchPill can display what was just said. The toggle is shown only when macOS Launch Services finds NotchPill's `com.local.notchpill` bundle. If NotchPill is removed, mirroring becomes dormant and the caption file is deleted without overwriting the saved preference; reinstalling the app restores the prior choice. Off by default. The file is owner-only (`0600`), holds only the most recent caption — each write replaces the last — and is written after the clipboard, best-effort, so it can never delay or affect dictation output. Nothing leaves the device. Switching the setting off deletes the file. |
| `outputDir` | `string` | `''` | Any absolute folder path, or `''` for default | Destination for saved transcript/audio files. Empty means the app default (`Documents/Murmur`, created on first write). Set via a folder picker (`dialog:allow-open`). |
| `benchmarkOutputDir` | `string` | `''` | Any absolute folder path, or `''` for default | Destination for saved Performance Lab benchmark reports (`benchmark-<version>-<machine>-<createdAt>.json`). Empty means the app default (`Documents/Murmur`, created on first write). Kept separate from `outputDir` so benchmark JSON doesn't mix with dictation transcripts/audio. Set via a folder picker in the Performance Lab. |
| `benchmarkAutoSave` | `boolean` | `false` | `true` / `false` | When enabled, each completed benchmark run is written to `benchmarkOutputDir` automatically (in addition to the 10-slot in-app history), so reports survive the localStorage cap. Best-effort: a write failure surfaces an error but does not fail the run. |

## Vocabulary Settings

`vocabularyEntries` is an array of `{ id, written, aliases, enabled, scope }`. `written` is the canonical surface form used by Whisper prompt bias and post-model correction. `aliases` contains exact spoken variants applied locally on every backend. Settings currently creates `{ kind: 'global' }` scopes; typed app/project scopes are selected from the existing immutable dictation context. The legacy `customVocabulary` string is migration-only and is re-derived from enabled global canonical terms. Built-in and scanned developer terms are a separate pool: the global toggle makes that pool available, while a matching Code / technical profile or Local IDE project-context opt-in activates it for a recording. Unmatched apps never receive that pool.

Aliases are limited to 16 per entry and values to 256 characters. Ambiguous aliases, canonical collisions, Voice Command collisions, and direct or indirect cycles are rejected atomically.

## Personal Knowledge

Settings → Knowledge manages a separate local SQLite store for replacement rules, vocabulary terms, and snippets. It is not part of the `Settings` object or `localStorage`. Users can search and filter bounded pages, inspect scope/provenance, create and edit records, enable/disable them, export/import a versioned JSON file, delete individual records, and delete all records with typed confirmation.

The store reports recovered, reinitialized, and unavailable states visibly. Enabled replacement rules apply through Smart Correction; snippet triggers remain inert until their separate Voice Commands integration. See [Personal Knowledge Store](../features/personal-knowledge-store.md) and [Correct and Teach](../features/correct-and-teach.md).

## Per-App Profiles

`appProfiles` is an array of `{ bundleId, label, writingStyle, autoPasteOverride, cleanupOverride, smartFormattingOverride, cliFormattingOverride, ideContextEnabled, ideProjectRoots, queryContextExcluded }`. `writingStyle` is `null` (Inherit), `conversational`, `polished`, `code_technical`, `verbatim`, or `notes`. It is an explicit user choice; bundle identifiers and labels never classify apps automatically. Boolean overrides fine-tune the resolved style/global value for a matching frontmost bundle identifier; `null` means "inherit." Existing, missing, and malformed persisted style/override fields migrate to `null`.

`ideContextEnabled` defaults to `false` and must be enabled on the exact matching profile. `ideProjectRoots` persists only the explicit user-selected root strings, trimmed, deduplicated, and capped at four. Filenames, symbols, source snippets, and scan results are memory-only and are not settings fields. The roots therefore remain visible in Settings and in any direct inspection or backup of the existing settings JSON; there is no hidden export path.

`queryContextExcluded` defaults to `false`. When true on the first matching profile, it forces Voice Query context off for that app even when the global or provider-preset level requests app/window or selection context. It is deny-only and cannot opt an app into context.

`smartFormattingEnabled` is a separate boolean setting, off by default. It enables deterministic list, email/URL, extended Spoken Structure, and bounded same-utterance correction rules for live prose. Missing or malformed persisted values migrate safely to `false`; it is independent of `smartPunctuation`. `smartFormattingOverride` gives profiles the same Default/On/Off choice.

`cliFormattingOverride` uses the immutable recording-start context. `true` enables profile-mode CLI recognition, `false` disables implicit CLI formatting for that app, and `null` keeps conservative automatic recognition. An explicit spoken `command` trigger remains available in every mode.

At recording start, the backend resolves one immutable context using global settings → matching style → matching profile fine-tuning → one-session overrides. It also records one Spoken Structure policy: Off, Basic from Voice Commands, Extended from Smart Formatting, or Union when both are enabled. Settings or focus changes during recording apply only to the next session. Explicit IDE opt-in disables extended Smart Formatting for that recording but retains Basic structure when Voice Commands remains enabled, and can capture only the matching profile's fresh local index. See [Spoken Structure](../features/spoken-structure.md), [Per-App Dictation Context](../features/per-app-profiles.md), and [Local IDE Symbols and `@file` Context](../features/ide-context.md).

## Voice Commands

`voiceCommandsEnabled` remains the global execution switch. Legacy `voiceCommands` pairs in `dictation-settings` are migration-only compatibility input: Rust imports them once into the personal knowledge store as global `text_replacement` commands, while retaining the old in-memory path if the store is unavailable.

New text replacements and snippets are Rust-owned knowledge records rather than localStorage settings. They support global/app scope, enabled state, multiline snippet bodies, deterministic `{{date}}` / `{{time}}`, and explicitly granted `{{clipboard}}`. See [Voice Commands 2.0](../features/voice-commands.md).

---

## System Settings

| Setting | Type | Default | Valid Options/Range | Description |
|---------|------|---------|-------------------|-------------|
| `microphone` | `string` | `'system_default'` | `'system_default'` or a descriptor `id` from the shared input inventory | Stable audio input ID for recording. On CoreAudio this is the raw device UID, without CPAL's host prefix. Display names are presentation-only. When set to `'system_default'`, the frontend sends `null` and the backend resolves the live system default at recording start. A missing explicit ID fails closed; it never records from another physical microphone. |
| `microphoneIdMigrationComplete` | `boolean` | `true` | `true` / `false` | Durable proof that `microphone` is the System Default sentinel or a stable ID selected/proven against an authoritative inventory. Schema v3 marks old `system_default` values complete without an inventory request. Other old strings remain pending because opaque CoreAudio UIDs cannot be distinguished syntactically from legacy display names. Exact ID membership or one unique display-name match completes migration; ambiguous/missing names remain unresolved. |
| `launchAtLogin` | `boolean` | `false` | `true` / `false` | Whether the app starts automatically on macOS login. Uses `@tauri-apps/plugin-autostart` with `MacosLauncher::LaunchAgent`. On mount, the hook checks the actual OS autostart state and reconciles with the stored setting (handles the case where the user removed the login item from System Settings). |
| `overlayVerticalOffset` | `number` | `0` | Integer `-12` through `12` | Confirmed native overlay fine-tuning in logical points. Settings previews the real window; Cancel restores the baseline, Preview default is transient, and Save or inactive Reset persists. Schema v2 resets offsets from the former broken drag flow once. |

---

## Persistence and Migration

### Storage

- **Durable copy:** `settings.json` in the per-bundle app data directory, owned by `commands/settings_store.rs`. This is the source of truth: it survives a manual reinstall and WebKit storage eviction, which `localStorage` does not.
- **Cache:** `localStorage` under the key `dictation-settings`, written through on every save so `loadSettings()` stays synchronous — its callers (overlay hooks in particular) read settings during render.
- **Format:** Full `Settings` object serialized as JSON, plus `settingsVersion`. The same string is written to both places.

The blob is opaque to Rust. The host validates only the container — at most 1 MiB, and it must parse as a JSON object — so every schema, default, and migration rule stays in `lib/settings.ts` and a settings change never requires a Rust change.

### Boot Hydration

Each window entry (`main.tsx`, `overlay.tsx`, `transform-review.tsx`, `query-review.tsx`, `diagnostics.tsx`) awaits `hydrateSettingsFromDisk()` before its first render:

1. Outside Tauri (plain browser, tests) it is a no-op and `localStorage` remains the only store.
2. `load_settings_blob` returns a blob → it is written into `localStorage` verbatim. Disk wins; the cache may be stale or evicted.
3. `load_settings_blob` returns `null` → first run, or an existing install whose settings only ever lived in `localStorage`. A non-null cached blob is migrated to disk once via `save_settings_blob`.
4. Any failure is logged and swallowed. Boot never blocks on the settings store, and `localStorage` stays the fallback for the session.

On the first v2 load, Murmur resets any pre-v2 overlay calibration to zero and
deletes the old standalone `murmur-overlay-vertical-offset` key. Subsequent v2
offsets are integer-clamped to ±12 and remain inside the durable Settings blob.

On the first v3 load, Murmur adds `microphoneIdMigrationComplete`. System
Default is complete immediately. Every other pre-v3 microphone string remains
pending until an available shared inventory proves exact ID membership or a
unique legacy display-name match; stale/unavailable inventory never migrates it.

Hydration is idempotent, so every window can run it regardless of creation order; concurrent first-run writes are serialized in Rust and write identical content.

### Saving

`saveSettings()` (and the migration re-persist inside `loadSettings()`) serializes once, writes `localStorage` synchronously, then fires `save_settings_blob` without awaiting it. A rejected or unavailable backend is logged and never fails the save — the cache is already written and the next boot repairs the durable copy.

### Corruption

A `settings.json` that is oversized, not UTF-8, not valid JSON, or not a JSON object is renamed to `settings.json.corrupt-<unix-seconds>` and reported as "no settings on disk", so the window falls back to its `localStorage` cache. Corrupt files are never deleted.

### Loading Behavior

`loadSettings()` performs the following:
1. Reads from `localStorage` under `dictation-settings` (seeded from disk at boot).
2. If found, parses as JSON and merges with `DEFAULT_SETTINGS` (stored values override defaults). Legacy comma/newline-separated `customVocabulary` values migrate to enabled global `vocabularyEntries` with no aliases.
3. Applies migration: if `recordingMode` is missing or invalid (including the legacy `'hotkey'` value), resets to `'hold_down'`.
4. Strips the legacy `hotkey` field if present.
5. Validates `model` against the macOS catalog allow-list. Any invalid or removed model (e.g. `moonshine-tiny`, `moonshine-base`) resets to Core ML Parakeet v3. `language` is validated against `LANGUAGE_OPTIONS` the same way.
6. Coerces `transformHoldKey` to `null` unless it matches `TRANSFORM_KEY_OPTIONS`, and sanitizes every structured field — `appProfiles`, `voiceCommands`, `vocabularyEntries`, `codeVocabLastScan` — dropping malformed entries and clamping list lengths so a tampered blob can't reach the Rust side or render bad numbers.
7. Removes fields from deleted features (`hotkey`, `liveTranscriptPreview`).
8. If not found or on parse error, returns `DEFAULT_SETTINGS`.

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
| `mirrorToNotchPill` | `mirrorToNotchPill` | Yes |
| `outputDir` | `outputDir` | Yes |
| `doubleTapKey` | _(sent via `update_keyboard_key`)_ | Via keyboard hooks |
| `recordingMode` | _(controls which hook is active)_ | Frontend only |
| `hotkeyMissFeedback` | _(controls overlay rejection feedback)_ | Frontend only |
| `autoStopSilenceMs` | _(drives the frontend silence detector)_ | Frontend only |
| `microphone` | _(sent as param to `start_native_recording`)_ | Per recording |
| `launchAtLogin` | _(sent via autostart plugin)_ | Via OS API |
| `overlayVerticalOffset` | _(sent via `set_overlay_vertical_offset`)_ | Frontend/native window only |
| `benchmarkOutputDir` | _(sent as param to `save_benchmark_report` / `open_benchmark_output_folder`)_ | On save/reveal |
| `benchmarkAutoSave` | _(read in the Performance Lab; drives auto-save after each run)_ | Frontend only |

**Optimistic updates with rollback:** If `configure_dictation` fails, the affected settings (model, language, autoPaste, autoPasteDelayMs, vadSensitivity) revert to their previous values. Similarly, if the autostart toggle fails, `launchAtLogin` reverts. A versioned configure ref prevents stale rollbacks from overwriting newer settings.

---

## Related Durable User Data

History, usage statistics, and the saved theme library are not part of the
`Settings` object, but use the same durable-source/localStorage-cache contract.
On main-window boot, hydration loads `history.json`, `stats.json`, and
`theme-library.json` before React renders. Disk wins over stale caches; when a
durable file is absent, the corresponding existing localStorage blob migrates
to disk once. Failures are isolated per file and never block boot. Theme-library
disk commands are additionally restricted to the main window.

Voice Query history is intentionally outside this cache contract. Its content
stays in `query-history/query-history.sqlite3` and enters main-window memory
only through bounded paging while History → Queries is active. It has no
localStorage key, cache migration, dictation export, or Stats mirror.

Other localStorage caches and browser-scoped state:

| Key | Purpose | Used By |
|-----|---------|---------|
| `dictation-history` | Synchronous cache for durable `history.json` entries (rolling max 200) | `useHistoryManagement` |
| `murmur-appearance` | Versioned appearance mode/theme configuration plus a strictly validated derived light/dark token cache. Independent from `Settings`; imports discard and regenerate revision/cache data. | Main appearance controller (writer/native theme) |
| `murmur-theme-library` | Versioned, revisioned saved-theme collection with stable IDs, supported appearance variants, and local or Open VSX provenance. Mirrored to main-window-only durable `theme-library.json`; bounded to 1 MiB and 128 entries. | Main appearance controller and Appearance Settings |
| `dictation-stats` | Synchronous cache for durable `stats.json` usage aggregates | `lib/stats.ts` |
| `skipped-update-version` | Version string the user chose to skip | `useAutoUpdater` |
| `updater-last-check` | Timestamp of last update check | `useAutoUpdater` |
| `resource-monitor-collapsed` | Whether the resource monitor panel is collapsed | ResourceMonitor component |
