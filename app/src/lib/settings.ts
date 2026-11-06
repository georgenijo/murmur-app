import { nextSourceWriteVersion } from './sourceWriteVersion';
import { presentationStorage } from './presentationStorage';
import { persistSourceState, type OwnedEdits } from './webkitLifecycle';
import rustSettingsDefaults from '../../src-tauri/crates/murmur-core/src/state/settings-defaults.json';
import { invoke, isTauri } from '@tauri-apps/api/core';

export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

const MODIFIER_DICTATION_KEY_IDS = ['shift_l', 'alt_l', 'ctrl_r'] as const;
const FUNCTION_DICTATION_KEY_IDS = [
  'f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'f8', 'f9', 'f10',
  'f11', 'f12', 'f13', 'f14', 'f15', 'f16', 'f17', 'f18', 'f19', 'f20',
] as const;
const DICTATION_KEY_IDS = [
  ...MODIFIER_DICTATION_KEY_IDS,
  ...FUNCTION_DICTATION_KEY_IDS,
] as const;

export type DictationKey = (typeof DICTATION_KEY_IDS)[number];
export type FunctionDictationKey = (typeof FUNCTION_DICTATION_KEY_IDS)[number];

export type PasteLastShortcut =
  | 'command_shift_v'
  | 'command_option_v'
  | 'command_control_v';

export type WritingStyle =
  | 'conversational'
  | 'polished'
  | 'code_technical'
  | 'verbatim'
  | 'notes';

export type WritingStyleChoice = WritingStyle | 'inherit';

export const WRITING_STYLE_OPTIONS: { value: WritingStyleChoice; label: string }[] = [
  { value: 'inherit', label: 'Inherit current settings' },
  { value: 'conversational', label: 'Conversational' },
  { value: 'polished', label: 'Polished prose' },
  { value: 'code_technical', label: 'Code / technical' },
  { value: 'verbatim', label: 'Verbatim' },
  { value: 'notes', label: 'Notes' },
];

/** Concrete (non-`'inherit'`) writing-style values, derived from `WRITING_STYLE_OPTIONS`. */
export const WRITING_STYLE_VALUES: readonly WritingStyle[] = WRITING_STYLE_OPTIONS
  .map(option => option.value)
  .filter((value): value is WritingStyle => value !== 'inherit');

/**
 * Per-app dictation profile. When the frontmost macOS app's bundle id matches
 * `bundleId`, each `*Override` (when non-null) replaces the corresponding global
 * setting. `null` means "no override — use the global setting".
 */
export interface AppProfile {
  bundleId: string;
  label: string;
  autoPasteOverride: boolean | null;
  cleanupOverride: boolean | null;
  smartFormattingOverride: boolean | null;
  cliFormattingOverride: boolean | null;
  /** Explicit deterministic writing policy. `null` preserves current behavior. */
  writingStyle: WritingStyle | null;
  /** Explicit opt-in to a memory-only local project index for this profile. */
  ideContextEnabled: boolean;
  /** User-selected local roots. Index contents are never persisted. */
  ideProjectRoots: string[];
  /** Reusable Mode applied before this profile's fine-tuning overrides. */
  modeId?: string | null;
}

export type ModeVocabularyPolicy = 'inherit' | 'general' | 'technical';
export type ModeContextPolicy = 'none' | 'project';

export interface MurmurMode {
  id: string;
  name: string;
  builtIn: boolean;
  enabled: boolean;
  writingStyle: WritingStyle | null;
  cleanupEnabled: boolean | null;
  smartFormattingEnabled: boolean | null;
  cliFormattingEnabled: boolean | null;
  vocabularyPolicy: ModeVocabularyPolicy;
  contextPolicy: ModeContextPolicy;
  modelId: ModelOption | null;
  language: string | null;
  autoPaste: boolean | null;
}

export const SITE_MODE_BROWSERS = [
  { bundleId: 'com.apple.Safari', label: 'Safari' },
  { bundleId: 'com.google.Chrome', label: 'Google Chrome' },
  { bundleId: 'com.microsoft.edgemac', label: 'Microsoft Edge' },
  { bundleId: 'com.brave.Browser', label: 'Brave' },
  { bundleId: 'company.thebrowser.Browser', label: 'Arc' },
  { bundleId: 'org.chromium.Chromium', label: 'Chromium' },
] as const;

export interface BrowserSiteRule {
  id: string;
  browserBundleId: string;
  host: string;
  modeId: string;
  enabled: boolean;
}

export function normalizeSiteHost(value: string): string | null {
  const host = value.trim().toLowerCase().replace(/\.$/, '');
  if (!host || host.length > 253 || host.includes(':')) return null;
  if (host === 'localhost') return host;
  const labels = host.split('.');
  if (labels.some((label) => !label || label.length > 63
    || !/^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(label))) return null;
  return labels.length > 1 ? host : null;
}

const builtinMode = (
  id: string,
  name: string,
  writingStyle: WritingStyle | null,
  overrides: Partial<MurmurMode> = {},
): MurmurMode => ({
  id, name, builtIn: true, enabled: true, writingStyle,
  cleanupEnabled: null, smartFormattingEnabled: null, cliFormattingEnabled: null,
  vocabularyPolicy: 'inherit', contextPolicy: 'none', modelId: null,
  language: null, autoPaste: null, ...overrides,
});

export const BUILTIN_MODES: readonly MurmurMode[] = [
  builtinMode('builtin.everyday', 'Everyday', null),
  builtinMode('builtin.messages', 'Messages', 'conversational'),
  builtinMode('builtin.email', 'Email', 'polished'),
  builtinMode('builtin.notes', 'Notes', 'notes'),
  builtinMode('builtin.technical', 'Technical', 'code_technical', { vocabularyPolicy: 'technical' }),
  builtinMode('builtin.terminal', 'Terminal', 'code_technical', { vocabularyPolicy: 'technical', cliFormattingEnabled: true }),
  builtinMode('builtin.verbatim', 'Verbatim', 'verbatim'),
] as const;



/**
 * A user-defined voice command. When `phrase` is spoken it is replaced by
 * `replacement` (case-insensitive, word-boundary). Applied after the built-in
 * commands, so users extend rather than override the defaults.
 */
export interface VoiceCommand {
  phrase: string;
  replacement: string;
}

export type VocabularyScope =
  | { kind: 'global' }
  | { kind: 'app'; bundleId: string }
  | { kind: 'project'; bundleId: string; root: string };

/** One canonical written term plus exact spoken variants recognized locally. */
export interface VocabularyEntry {
  id: string;
  written: string;
  aliases: string[];
  enabled: boolean;
  scope: VocabularyScope;
}

export const MAX_VOCABULARY_ENTRIES = 500;
export const MAX_VOCABULARY_ALIASES = 16;
export const MAX_VOCABULARY_VALUE_CHARS = 256;

export function vocabularyPrompt(entries: VocabularyEntry[]): string {
  return entries
    .filter((entry) => entry.enabled && entry.scope.kind === 'global')
    .map((entry) => entry.written.trim())
    .filter(Boolean)
    .join(', ');
}

/**
 * Result of a code-vocabulary scan. Shape matches the Rust `scan_code_vocab`
 * command return value exactly (serde camelCase). Persisted so the settings
 * panel can show the last completed scan when reopened.
 */
/** One ranked term actually kept by the scan. `rank` is the array index + 1. */
export interface RankedTerm {
  term: string;
  freq: number;
}

export interface VocabScanSummary {
  files: number;
  skipped: number;
  terms: number;
  bytes: number;
  capped: boolean;
  ms: number;
  /** Top ~12 written forms surfaced as sample chips. */
  sampleTerms: string[];
  /**
   * Full ranked list of terms actually kept (<=500), ordered by frequency.
   * rank = array index + 1. Powers the View-all pop-out. The top
   * `whisperCount` of these also feed Whisper's token-bound prompt; the rest
   * are Smart-Correction-only.
   */
  rankedTerms: RankedTerm[];
  /** How many of `rankedTerms` feed the Whisper prompt (= min(96, len)). */
  whisperCount: number;
  /** False when a newer scan or settings change superseded this walk. */
  adopted: boolean;
}

/** Hard ceiling on the persisted ranked list, mirroring the backend cap. */

/** Hard ceiling on the persisted sample-chip list (backend sends ~12). */

export interface Settings {
  /** Durable ordering for native quick-control fields; not capture configuration. */
  ownedFieldsRevision?: number;
  model: ModelOption;
  doubleTapKey: DictationKey;
  language: string;
  autoPaste: boolean;
  /** Keep delivery associated with the recent text field across supported media controls. */
  smartDestinationEnabled: boolean;
  autoPasteDelayMs: number;
  /** Global Paste Last chord. `null` disables it. */
  pasteLastShortcut: PasteLastShortcut | null;
  recordingMode: RecordingMode;
  hotkeyMissFeedback: boolean;
  /** Play local output-only feedback for dictation lifecycle transitions. */
  soundCuesEnabled: boolean;
  /** Cue output volume as an integer percentage. */
  soundCueVolume: number;
  /** Opt-in because recurring meeting cues can be disruptive. */
  meetingSoundCuesEnabled: boolean;
  /**
   * Trailing silence (ms) after which a hands-free double-tap recording stops
   * itself. `0` disables it. Only ever applied in Double-Tap mode — in
   * Hold Down (and the hold half of Both) the key release owns the stop.
   */
  autoStopSilenceMs: number;
  /**
   * Smart Auto-Stop (#834): at each pause, ask the local Smart Turn model
   * whether the speaker is done and stop early when it is. Rides on
   * `autoStopSilenceMs`, which stays the ceiling. Off by default.
   */
  smartAutoStopEnabled: boolean;
  microphone: string;
  /** True once `microphone` is proven to be a backend ID or System Default. */
  microphoneIdMigrationComplete: boolean;
  /** Opt-in cached-inventory routing for the next dictation recording. */
  smartAutoMicrophoneEnabled: boolean;
  /** Separate consent for bounded idle-time signal checks of approved inputs. */
  smartAutoProbeEnabled: boolean;
  /** Stable Core Audio UIDs the user has explicitly allowed Smart Auto to use. */
  smartAutoApprovedDeviceIds: string[];
  /** Optional ordered stable-ID preferences, evaluated before the macOS default. */
  smartAutoPreferredDeviceIds: string[];
  /** Continuity Capture remains excluded unless the user explicitly allows it. */
  smartAutoAllowContinuity: boolean;
  launchAtLogin: boolean;
  /** Confirmed vertical fine-tuning for the notch overlay, in logical points. */
  overlayVerticalOffset: number;
  overlayPlacement: 'notch' | 'floating_pill';
  overlayLiveWords: boolean;
  vadSensitivity: number;
  idleTimeoutMinutes: number;
  captureStandbyEnabled: boolean;
  captureStandbySeconds: number;
  capturePreRollMs: number;
  /** @deprecated Migration-only mirror; structured entries are authoritative. */
  customVocabulary: string;
  vocabularyEntries: VocabularyEntry[];
  disabled: boolean;
  smartPunctuation: boolean;
  /** Persist completed microphone and file transcripts in local history. */
  retainHistory: boolean;
  /** Keep meeting chunk WAV files after their durable transcript commits. */
  meetingRetainAudio: boolean;
  /** Opt in to local, session-scoped labels for remote meeting speakers. */
  meetingDiarization: boolean;
  /** Opt in to helper-side acoustic echo cancellation for Meeting Capture. */
  meetingEchoCancellationEnabled: boolean;
  /** Opt in to bounded Calendar checks that can suggest starting Notetaker. */
  meetingSuggestionsEnabled: boolean;
  /** Delete completed meetings older than this many days; 0 keeps them by age. */
  meetingRetentionDays: number;
  /** Maximum completed/interrupted meeting sessions retained in SQLite. */
  meetingMaxSessions: number;
  saveTranscript: boolean;
  saveAudio: boolean;
  /** Mirror each final transcript to a local file NotchPill can show in the notch. */
  mirrorToNotchPill: boolean;
  outputDir: string;
  /** Destination for saved Performance Lab benchmark reports. Empty = default
   * `Documents/Murmur`. Kept separate from `outputDir` so benchmark JSON doesn't
   * mix with saved dictation transcripts/audio. */
  benchmarkOutputDir: string;
  /** Write each benchmark report to `benchmarkOutputDir` automatically as it
   * completes, so reports survive the 10-slot localStorage cap. */
  benchmarkAutoSave: boolean;
  appProfiles: AppProfile[];
  /** User-defined reusable Modes. Built-ins are code-owned and not duplicated here. */
  modes: MurmurMode[];
  /** Last Mode explicitly selected outside an automatic app binding. */
  activeModeId: string;
  /** Explicit opt-in to frontmost-browser exact-host Mode activation. */
  siteModeLookupEnabled: boolean;
  /** Exact browser + host rules. URL paths and live site identity are never persisted. */
  browserSiteRules: BrowserSiteRule[];
  voiceCommandsEnabled: boolean;
  /** User-defined voice commands applied after the built-in set. */
  voiceCommands: VoiceCommand[];
  cleanupEnabled: boolean;
  /** Deterministic live prose formatting and bounded same-utterance correction. */
  smartFormattingEnabled: boolean;
  /** When cleanup is on, remove filler tokens ("um", "uh"). */
  cleanupRemoveFiller: boolean;
  /** When cleanup is on, capitalize sentence starts. */
  cleanupCapitalize: boolean;
  /**
   * Bias transcription toward code identifiers. When enabled, a built-in
   * dev-term dictionary is always used; a project folder (optional) layers the
   * user's own identifiers on top.
   */
  codeVocabEnabled: boolean;
  /** Optional absolute path to a project folder scanned for code identifiers. */
  codeVocabFolder: string;
  /**
   * Last completed code-vocab scan summary, persisted so the settings panel
   * shows the done-state on reopen. `null` until the folder has been scanned.
   */
  codeVocabLastScan: VocabScanSummary | null;
  /**
   * Post-model correction: apply the vocabulary to the transcript *output* of every
   * backend (Tier 1 exact map + Tier 2 sounds-like). On by default — it's what makes
   * vocab work on non-Whisper engines, which ignore Whisper's prompt.
   */
  correctionEnabled: boolean;
  /** Tier 2 phonetic "sounds-like" matching. Gated under correctionEnabled. */
  correctionFuzzy: boolean;
  /** Experimental Apple on-device homophone context tier; opt-in. */
  homophoneContextEnabled: boolean;
}

export type ModelOption =
  | 'parakeet-tdt-0.6b-v3-coreml'
  | 'tiny.en'
  | 'base.en'
  | 'small.en'
  | 'medium.en'
  | 'large-v3-turbo'
  // --- Parakeet backend (removable): delete this member to remove. ---
  | 'parakeet-tdt-0.6b-v2-fp16';

export type TranscriptionBackend = 'whisper' | 'parakeet' | 'coreml';

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string; backend: TranscriptionBackend }[] = [
  { value: 'parakeet-tdt-0.6b-v3-coreml', label: 'Parakeet Core ML', size: '~470 MB', backend: 'coreml' },
  { value: 'tiny.en', label: 'Whisper Tiny (English)', size: '~75 MB', backend: 'whisper' },
  { value: 'base.en', label: 'Whisper Base (English)', size: '~150 MB', backend: 'whisper' },
  { value: 'small.en', label: 'Whisper Small (English)', size: '~500 MB', backend: 'whisper' },
  { value: 'medium.en', label: 'Whisper Medium (English)', size: '~1.5 GB', backend: 'whisper' },
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~1.6 GB', backend: 'whisper' },
  // --- Parakeet backend (removable): delete this entry to remove. ---
  { value: 'parakeet-tdt-0.6b-v2-fp16', label: 'Parakeet TDT 0.6B (English, fast)', size: '~1.2 GB', backend: 'parakeet' },
];

export const AVAILABLE_MODEL_OPTIONS = MODEL_OPTIONS;

export interface DictationKeyOption {
  value: DictationKey;
  label: string;
}

export const DICTATION_KEY_OPTIONS: DictationKeyOption[] =
  DICTATION_KEY_IDS.map((value) => ({ value, label: dictationKeyLabel(value) }));

const DICTATION_KEY_ID_SET: ReadonlySet<string> = new Set(DICTATION_KEY_IDS);

export function isDictationKey(value: unknown): value is DictationKey {
  return typeof value === 'string' && DICTATION_KEY_ID_SET.has(value);
}

export function dictationKeyLabel(key: DictationKey): string {
  if (isFunctionDictationKey(key)) return key.toUpperCase();
  switch (key) {
    case 'shift_l': return '⇧ Left Shift';
    case 'alt_l': return '⌥ Left Option';
    case 'ctrl_r': return '⌃ Right Control';
    default: {
      const exhaustive: never = key;
      return exhaustive;
    }
  }
}

export function isFunctionDictationKey(key: DictationKey): key is FunctionDictationKey {
  return key.startsWith('f');
}

export function recordingShortcutHint(mode: RecordingMode, key: DictationKey): string {
  const label = dictationKeyLabel(key);
  if (mode === 'double_tap') return `Double-tap ${label}`;
  if (mode === 'both') return `Hold or double-tap ${label}`;
  return `Hold ${label}`;
}

export const PASTE_LAST_SHORTCUT_OPTIONS: {
  value: PasteLastShortcut | 'disabled';
  label: string;
}[] = [
  { value: 'disabled', label: 'Disabled' },
  { value: 'command_shift_v', label: '⌘⇧V' },
  { value: 'command_option_v', label: '⌘⌥V' },
  { value: 'command_control_v', label: '⌘⌃V' },
];

export function pasteLastShortcutLabel(shortcut: PasteLastShortcut | null): string | undefined {
  if (shortcut === null) return undefined;
  return PASTE_LAST_SHORTCUT_OPTIONS.find((option) => option.value === shortcut)?.label;
}

export function pasteLastShortcutConflict(
  shortcut: string | null,
  settings: Pick<Settings, 'doubleTapKey'>,
): string | null {
  if (shortcut === null) return null;
  if (shortcut === settings.doubleTapKey) {
    return 'That shortcut is already assigned to Dictation. Choose another Paste Last shortcut.';
  }

  if (shortcut === 'command_shift_p') {
    return 'That shortcut opens Murmur\'s command palette. Choose another Paste Last shortcut.';
  }
  if (shortcut === 'command_comma') {
    return 'That shortcut opens Murmur Settings. Choose another Paste Last shortcut.';
  }
  return null;
}

export const RECORDING_MODE_OPTIONS: { value: RecordingMode; label: string }[] = [
  { value: 'hold_down', label: 'Hold down' },
  { value: 'double_tap', label: 'Double-tap' },
  { value: 'both', label: 'Both' },
];

/** Allow-list for `autoStopSilenceMs`. Anything else coerces back to Off. */
export const AUTO_STOP_SILENCE_OPTIONS: { value: number; label: string }[] = [
  { value: 0, label: 'Off' },
  { value: 1500, label: '1.5s' },
  { value: 2500, label: '2.5s' },
  { value: 4000, label: '4s' },
];

/** Keeps the model loaded on adapter or desktop power; 5 minutes on battery. */
export const IDLE_TIMEOUT_NEVER_ON_POWER_ADAPTER = -1;

export const IDLE_TIMEOUT_OPTIONS: { value: number; label: string }[] = [
  { value: 5, label: '5 minutes' },
  { value: 15, label: '15 minutes' },
  { value: IDLE_TIMEOUT_NEVER_ON_POWER_ADAPTER, label: 'Never on power adapter' },
  { value: 0, label: 'Never' },
];

export const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  { value: 'auto', label: 'Auto-detect' },
  { value: 'en', label: 'English' },
  { value: 'es', label: 'Spanish' },
  { value: 'fr', label: 'French' },
  { value: 'de', label: 'German' },
  { value: 'it', label: 'Italian' },
  { value: 'pt', label: 'Portuguese' },
  { value: 'nl', label: 'Dutch' },
  { value: 'ja', label: 'Japanese' },
  { value: 'ko', label: 'Korean' },
  { value: 'zh', label: 'Chinese' },
  { value: 'ru', label: 'Russian' },
  { value: 'pl', label: 'Polish' },
  { value: 'tr', label: 'Turkish' },
  { value: 'hi', label: 'Hindi' },
  { value: 'ar', label: 'Arabic' },
];

export const DEFAULT_SETTINGS = rustSettingsDefaults as Settings;

export const STORAGE_KEY = 'dictation-settings';
export const ONBOARDING_KEY = 'murmur_onboarding_complete';
export const LEGACY_OVERLAY_OFFSET_KEY = 'murmur-overlay-vertical-offset';
export const OVERLAY_VERTICAL_OFFSET_MIN = -12;
export const OVERLAY_VERTICAL_OFFSET_MAX = 12;
export const SETTINGS_VERSION = 7;

export interface SmartAutoMicrophoneRequest {
  approvedDeviceIds: string[];
  preferredDeviceIds: string[];
  allowContinuity: boolean;
  requireRecentSignal: boolean;
}

export function smartAutoMicrophoneRequest(
  settings: Pick<Settings,
    'smartAutoMicrophoneEnabled'
    | 'smartAutoProbeEnabled'
    | 'smartAutoApprovedDeviceIds'
    | 'smartAutoPreferredDeviceIds'
    | 'smartAutoAllowContinuity'>,
): SmartAutoMicrophoneRequest | null {
  if (!settings.smartAutoMicrophoneEnabled) return null;
  return {
    approvedDeviceIds: settings.smartAutoApprovedDeviceIds,
    preferredDeviceIds: settings.smartAutoPreferredDeviceIds,
    allowContinuity: settings.smartAutoAllowContinuity,
    requireRecentSignal: settings.smartAutoProbeEnabled,
  };
}

/**
 * Normalize the persisted microphone setting into the `deviceName` argument the
 * capture commands expect. `DEFAULT_SETTINGS.microphone` is the sentinel string
 * `'system_default'`, not a CoreAudio UID — forwarding it verbatim makes the
 * Rust device lookup fail with `NoInputDevice`. It (and an empty value) must
 * become `null`, which means "let the backend pick the system default".
 */
export function microphoneDeviceNameArg(microphone: string | null | undefined): string | null {
  return microphone && microphone !== DEFAULT_SETTINGS.microphone ? microphone : null;
}

export function clampOverlayVerticalOffset(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return DEFAULT_SETTINGS.overlayVerticalOffset;
  }
  return Math.max(
    OVERLAY_VERTICAL_OFFSET_MIN,
    Math.min(OVERLAY_VERTICAL_OFFSET_MAX, Math.trunc(value)),
  );
}

/**
 * Mirror a serialized blob to the Rust-owned `settings.json`. Fire-and-forget:
 * localStorage is the synchronous contract every caller depends on, and disk is
 * the durable copy read back at the next window boot — a failed or unavailable
 * bridge (plain browser, tests) must never surface as a settings-save failure.
 */
function mirrorToDisk(blob: string, ownedEdits?: OwnedEdits): void {
  persistSourceState(STORAGE_KEY, blob, ownedEdits);
}

function writePersistedSettings(settings: Settings, ownedEdits?: OwnedEdits): void {
  let launchOnboardingComplete: boolean | undefined;
  try { launchOnboardingComplete = presentationStorage.getItem(ONBOARDING_KEY) === 'true'; }
  catch { /* Rust preserves the existing gate when this optional cache is unreadable. */ }
  const blob = JSON.stringify({
    ...settings,
    settingsVersion: SETTINGS_VERSION,
    ...(launchOnboardingComplete !== undefined ? { launchOnboardingComplete } : {}),
  });
  mirrorToDisk(blob, ownedEdits);
  try { presentationStorage.setItem(STORAGE_KEY, blob); } catch { /* Rust intent remains queued. */ }
}

/** Mirror only the onboarding gate; preserve the exact current settings blob.
 * Native setup uses it on later launches. Capture still waits for configure.
 */
export function persistOnboardingComplete(complete: boolean): void {
  persistSourceState(ONBOARDING_KEY, JSON.stringify(complete));
  try {
    const cached = presentationStorage.getItem(STORAGE_KEY);
    if (cached === null) return;
    const value: unknown = JSON.parse(cached);
    if (!value || typeof value !== 'object' || Array.isArray(value)) return;
    presentationStorage.setItem(STORAGE_KEY, JSON.stringify({ ...value, launchOnboardingComplete: complete }));
  } catch { /* Cache cannot suppress the queued native patch. */ }
}

// Only the main window rewrites a migrated snapshot or seeds a fresh install.
// A companion window can hold a stale pre-migration cache, and persisting its
// migration would overwrite choices the main window has since saved.
let ownsSettingsMigrations = false;

/** Called once by the main window entry before it hydrates settings. */
export function ownSettingsMigrations(owns = true): void {
  ownsSettingsMigrations = owns;
}

/**
 * Seed localStorage from the durable `settings.json` before any window renders.
 * Disk wins: localStorage is a write-through cache that a reinstall or a WebKit
 * eviction can silently drop. Idempotent, so every window entry can await it
 * regardless of creation order.
 */
export async function hydrateSettingsFromDisk(): Promise<void> {
  try {
    if (!isTauri()) return;
    const blob = await invoke<string | null>('load_settings_blob');
    if (typeof blob === 'string') {
      // `loadSettings()` re-runs the full validation gauntlet over whatever is
      // in here, so no schema work belongs on this path.
      presentationStorage.setItem(STORAGE_KEY, blob);
      // The durable gate survives WebKit eviction with the same settings.
      // Legacy snapshots retain the existing local flag and get mirrored by
      // App's onboarding decision; Rust falls back for that first launch.
      const gate = (JSON.parse(blob) as { launchOnboardingComplete?: unknown }).launchOnboardingComplete;
      if (ownsSettingsMigrations && typeof gate === 'boolean') {
        if (gate) presentationStorage.setItem(ONBOARDING_KEY, 'true');
        else presentationStorage.removeItem(ONBOARDING_KEY);
      }
      return;
    }
    // No durable copy: either a first run, or an existing install whose
    // settings only ever lived in presentationStorage. Repair it once.
    const cached = presentationStorage.getItem(STORAGE_KEY);
    if (cached !== null && ownsSettingsMigrations) {
      await invoke('save_settings_blob', { blob: cached, ...nextSourceWriteVersion() });
    } else if (ownsSettingsMigrations) {
      // A fresh install. Commit the defaults so companions that fail closed
      // on a missing document (the live-words preview) see default-on.
      const defaults = JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: SETTINGS_VERSION });
      await invoke('save_settings_blob', { blob: defaults, ...nextSourceWriteVersion() });
      try { presentationStorage.setItem(STORAGE_KEY, defaults); } catch { /* Native creation succeeded. */ }
    }
  } catch (e) {
    // Boot must never block on the settings store; localStorage stays the
    // fallback for this session.
    console.error('Failed to hydrate settings from disk:', e);
  }
}

/** Decode only a Rust-validated snapshot. No migrations or durable writes. */
export function parseSettingsBlob(blob: string | null): Settings {
  try {
    const value: unknown = JSON.parse(blob ?? 'null');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return DEFAULT_SETTINGS;
    const { settingsVersion: _version, launchOnboardingComplete: _gate, ...settings } = value as Record<string, unknown>;
    return { ...DEFAULT_SETTINGS, ...settings } as Settings;
  } catch { return DEFAULT_SETTINGS; }
}
export function loadSettings(): Settings {
  try { return parseSettingsBlob(presentationStorage.getItem(STORAGE_KEY)); }
  catch { return DEFAULT_SETTINGS; }
}

/** Refresh a webview cache after the host has already committed settings. */
export function cacheSettings(settings: Settings): void {
  try {
    presentationStorage.setItem(STORAGE_KEY, JSON.stringify({ ...settings, settingsVersion: SETTINGS_VERSION }));
  } catch (e) {
    console.error('Failed to cache settings:', e);
  }
}

export function saveSettings(settings: Settings, ownedEdits?: OwnedEdits): void {
  try {
    writePersistedSettings(settings, ownedEdits);
  } catch (e) {
    console.error('Failed to save settings:', e);
  }
}
