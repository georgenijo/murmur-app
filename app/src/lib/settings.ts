import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  applyLegacyModes,
  legacyGlobalModelId,
  readLegacyModeState,
  stripLegacyModeKeys,
} from './legacyModes';

export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

const MODIFIER_DICTATION_KEY_IDS = ['shift_l', 'alt_l', 'ctrl_r'] as const;
const FUNCTION_DICTATION_KEY_IDS = [
  'f1', 'f2', 'f3', 'f4', 'f5', 'f6', 'f7', 'f8', 'f9', 'f10',
  'f11', 'f12', 'f13', 'f14', 'f15', 'f16', 'f17', 'f18', 'f19', 'f20',
] as const;
/** Fn/Globe: the only trigger whose press opens the microphone before the
 * gesture is confirmed (unconfirmed audio is discarded natively). */
export const GLOBE_DICTATION_KEY = 'fn';
const DICTATION_KEY_IDS = [
  ...MODIFIER_DICTATION_KEY_IDS,
  GLOBE_DICTATION_KEY,
  ...FUNCTION_DICTATION_KEY_IDS,
] as const;

export type DictationKey = (typeof DICTATION_KEY_IDS)[number];
const FUNCTION_DICTATION_KEY_SET: ReadonlySet<string> = new Set(FUNCTION_DICTATION_KEY_IDS);
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
}

const MAX_IDE_PROJECT_ROOT_BYTES = 4096;

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

function truncateVocabularyValue(value: string): string {
  return Array.from(value).slice(0, MAX_VOCABULARY_VALUE_CHARS).join('');
}

export function vocabularyPrompt(entries: VocabularyEntry[]): string {
  return entries
    .filter((entry) => entry.enabled && entry.scope.kind === 'global')
    .map((entry) => entry.written.trim())
    .filter(Boolean)
    .join(', ');
}

function legacyVocabularyEntries(value: unknown): VocabularyEntry[] {
  if (typeof value !== 'string') return [];
  return value
    .split(/[,\r\n]/)
    .map((written) => written.trim())
    .filter(Boolean)
    .slice(0, MAX_VOCABULARY_ENTRIES)
    .map((written, index) => ({
      id: `legacy-${index}`,
      written: truncateVocabularyValue(written),
      aliases: [],
      enabled: true,
      scope: { kind: 'global' },
    }));
}

function sanitizeVocabularyEntries(raw: unknown, legacy: unknown): VocabularyEntry[] {
  if (!Array.isArray(raw)) return legacyVocabularyEntries(legacy);
  return raw
    .filter((entry): entry is Record<string, unknown> => !!entry && typeof entry === 'object')
    .map((entry, index): VocabularyEntry | null => {
      if (typeof entry.written !== 'string' || !entry.written.trim()) return null;
      const scopeValue = entry.scope && typeof entry.scope === 'object'
        ? entry.scope as Record<string, unknown>
        : { kind: 'global' };
      let scope: VocabularyScope = { kind: 'global' };
      if (scopeValue.kind === 'app' && typeof scopeValue.bundleId === 'string' && scopeValue.bundleId.trim()) {
        scope = { kind: 'app', bundleId: scopeValue.bundleId.trim() };
      } else if (
        scopeValue.kind === 'project'
        && typeof scopeValue.bundleId === 'string'
        && scopeValue.bundleId.trim()
        && typeof scopeValue.root === 'string'
        && scopeValue.root.trim()
      ) {
        scope = {
          kind: 'project',
          bundleId: scopeValue.bundleId.trim(),
          root: scopeValue.root.trim(),
        };
      }
      const aliases = Array.isArray(entry.aliases)
        ? entry.aliases
            .filter((alias): alias is string => typeof alias === 'string')
            .map((alias) => truncateVocabularyValue(alias.trim()))
            .filter(Boolean)
            .filter((alias, aliasIndex, values) =>
              values.findIndex((value) => value.toLowerCase() === alias.toLowerCase()) === aliasIndex)
            .slice(0, MAX_VOCABULARY_ALIASES)
        : [];
      return {
        id: typeof entry.id === 'string' && entry.id.trim() ? entry.id : `vocabulary-${index}`,
        written: truncateVocabularyValue(entry.written.trim()),
        aliases,
        enabled: typeof entry.enabled === 'boolean' ? entry.enabled : true,
        scope,
      };
    })
    .filter((entry): entry is VocabularyEntry => entry !== null)
    .slice(0, MAX_VOCABULARY_ENTRIES);
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
const MAX_RANKED_TERMS = 500;

/** Hard ceiling on the persisted sample-chip list (backend sends ~12). */
const MAX_SAMPLE_TERMS = 50;

export interface Settings {
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
    case 'fn': return '🌐 Fn / Globe';
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
  return FUNCTION_DICTATION_KEY_SET.has(key);
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

export const DEFAULT_SETTINGS: Settings = {
  // FluidAudio runs Parakeet v3 on the Apple Neural Engine. Existing persisted
  // Whisper and sherpa selections remain valid and are never force-migrated.
  model: 'parakeet-tdt-0.6b-v3-coreml',
  doubleTapKey: 'shift_l',
  // Disabled by default — no settings UI to configure it yet (Phase D).
  // 'auto' lets Whisper auto-detect the spoken language ("just works"); the
  // non-Whisper models may auto-detect or ignore this value.
  language: 'auto',
  autoPaste: false,
  // Opt-in: destination continuity relies on native observation and must never
  // be enabled implicitly for existing installs or malformed settings blobs.
  smartDestinationEnabled: false,
  // Native CGEvents can paste immediately in the common case. Apps that move
  // focus asynchronously can still opt into a settling delay in Settings.
  autoPasteDelayMs: 0,
  pasteLastShortcut: null,
  recordingMode: 'hold_down',
  hotkeyMissFeedback: false,
  soundCuesEnabled: true,
  soundCueVolume: 45,
  meetingSoundCuesEnabled: false,
  // Opt-in: a recording that ends itself is a surprise until you ask for it.
  autoStopSilenceMs: 0,
  smartAutoStopEnabled: false,
  microphone: 'system_default',
  microphoneIdMigrationComplete: true,
  smartAutoMicrophoneEnabled: false,
  smartAutoProbeEnabled: false,
  smartAutoApprovedDeviceIds: [],
  smartAutoPreferredDeviceIds: [],
  smartAutoAllowContinuity: false,
  launchAtLogin: false,
  overlayVerticalOffset: 0,
  overlayPlacement: 'notch',
  overlayLiveWords: true,
  vadSensitivity: 50,
  idleTimeoutMinutes: 5,
  captureStandbyEnabled: false,
  captureStandbySeconds: 300,
  capturePreRollMs: 450,
  customVocabulary: '',
  vocabularyEntries: [],
  disabled: false,
  smartPunctuation: true,
  retainHistory: true,
  meetingRetainAudio: false,
  meetingDiarization: false,
  meetingEchoCancellationEnabled: false,
  meetingSuggestionsEnabled: false,
  meetingRetentionDays: 0,
  meetingMaxSessions: 100,
  saveTranscript: false,
  saveAudio: false,
  mirrorToNotchPill: false,
  outputDir: '',
  benchmarkOutputDir: '',
  benchmarkAutoSave: false,
  appProfiles: [],
  voiceCommandsEnabled: false,
  voiceCommands: [],
  cleanupEnabled: false,
  smartFormattingEnabled: false,
  cleanupRemoveFiller: true,
  cleanupCapitalize: true,
  codeVocabEnabled: false,
  codeVocabFolder: '',
  codeVocabLastScan: null,
  // Correction on by default: it's the fix that makes vocab actually apply on the
  // non-Whisper engines. A no-op when there's no vocabulary configured.
  correctionEnabled: true,
  correctionFuzzy: true,
  homophoneContextEnabled: false,
};

export const STORAGE_KEY = 'dictation-settings';
export const ONBOARDING_KEY = 'murmur_onboarding_complete';
export const LEGACY_OVERLAY_OFFSET_KEY = 'murmur-overlay-vertical-offset';
export const OVERLAY_VERTICAL_OFFSET_MIN = -12;
export const OVERLAY_VERTICAL_OFFSET_MAX = 12;
const SETTINGS_VERSION = 7;
const ZERO_DELAY_MIGRATION_VERSION = 1;
const OVERLAY_CALIBRATION_MIGRATION_VERSION = 2;
const LIVE_WORDS_DEFAULT_ON_MIGRATION_VERSION = 7;
const MAX_SMART_AUTO_DEVICES = 32;

export interface SmartAutoMicrophoneRequest {
  approvedDeviceIds: string[];
  preferredDeviceIds: string[];
  allowContinuity: boolean;
  requireRecentSignal: boolean;
}

function sanitizeSmartAutoDeviceIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const result: string[] = [];
  const seen = new Set<string>();
  for (const candidate of value) {
    if (typeof candidate !== 'string') continue;
    const id = candidate.trim();
    if (!id || id.includes('\0') || new TextEncoder().encode(id).length > 4096 || seen.has(id)) continue;
    seen.add(id);
    result.push(id);
    if (result.length === MAX_SMART_AUTO_DEVICES) break;
  }
  return result;
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

type PersistedSettings = Omit<Partial<Settings>, 'overlayPlacement'> & {
  settingsVersion?: unknown;
  launchOnboardingComplete?: unknown;
  /** v6 and earlier also stored `below_menu_bar`, migrated to `floating_pill`. */
  overlayPlacement?: string;
  /** @deprecated v6 and earlier. Belonged to the removed dot field. */
  overlayProcessingStyle?: unknown;
  hotkey?: string;
  liveTranscriptPreview?: unknown;
  recordingMode?: string;
  /** @deprecated v3 and earlier. Migrated to `pasteLastShortcut`. */
  pasteLastShortcutEnabled?: unknown;
};

/**
 * Mirror a serialized blob to the Rust-owned `settings.json`. Fire-and-forget:
 * localStorage is the synchronous contract every caller depends on, and disk is
 * the durable copy read back at the next window boot — a failed or unavailable
 * bridge (plain browser, tests) must never surface as a settings-save failure.
 */
function mirrorToDisk(blob: string): void {
  try {
    if (!isTauri()) return;
    void invoke('save_settings_blob', { blob }).catch(console.error);
  } catch (e) {
    console.error('Failed to persist settings to disk:', e);
  }
}

function writePersistedSettings(settings: Settings): void {
  const blob = JSON.stringify({
    ...settings,
    settingsVersion: SETTINGS_VERSION,
    launchOnboardingComplete: localStorage.getItem(ONBOARDING_KEY) === 'true',
  });
  localStorage.setItem(STORAGE_KEY, blob);
  mirrorToDisk(blob);
}

/** Mirror only the onboarding gate; preserve the exact current settings blob.
 * Native setup uses it on later launches. Capture still waits for configure.
 */
export function persistOnboardingComplete(complete: boolean): void {
  try {
    const cached = localStorage.getItem(STORAGE_KEY);
    if (cached === null) return;
    const value: unknown = JSON.parse(cached);
    if (!value || typeof value !== 'object' || Array.isArray(value)) return;
    if ((value as { launchOnboardingComplete?: unknown }).launchOnboardingComplete === complete) return;
    const blob = JSON.stringify({ ...value, launchOnboardingComplete: complete });
    localStorage.setItem(STORAGE_KEY, blob);
    mirrorToDisk(blob);
  } catch { /* An unavailable mirror falls back to frontend-driven startup. */ }
}

// Installed model ids, fetched once at boot by the main window so the Modes
// migration can adopt a global mode's model only when it is installed. Unknown
// (`null`) means the model is not adopted.
let installedModelIdsForLegacyMigration: ReadonlySet<string> | null = null;

async function prepareLegacyModeMigration(blob: string): Promise<void> {
  if (!ownsSettingsMigrations) return;
  installedModelIdsForLegacyMigration = null;
  try {
    const document = JSON.parse(blob) as unknown;
    if (!document || typeof document !== 'object' || Array.isArray(document)) return;
    const legacy = readLegacyModeState(document as Record<string, unknown>);
    if (!legacy.present || legacyGlobalModelId(legacy) === null) return;
    const catalog = await invoke<Array<{ modelName: string; installState: string }>>('get_model_runtime_catalog');
    installedModelIdsForLegacyMigration = new Set(
      catalog.filter((model) => model.installState === 'installed').map((model) => model.modelName),
    );
  } catch (e) {
    console.error('Failed to read installed models for the Modes migration:', e);
  }
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
      localStorage.setItem(STORAGE_KEY, blob);
      await prepareLegacyModeMigration(blob);
      // The durable gate survives WebKit eviction with the same settings.
      // Legacy snapshots retain the existing local flag and get mirrored by
      // App's onboarding decision; Rust falls back for that first launch.
      const gate = (JSON.parse(blob) as { launchOnboardingComplete?: unknown }).launchOnboardingComplete;
      if (ownsSettingsMigrations && typeof gate === 'boolean') {
        if (gate) localStorage.setItem(ONBOARDING_KEY, 'true');
        else localStorage.removeItem(ONBOARDING_KEY);
      }
      return;
    }
    // No durable copy: either a first run, or an existing install whose
    // settings only ever lived in localStorage. Repair it once.
    const cached = localStorage.getItem(STORAGE_KEY);
    if (cached !== null) {
      await prepareLegacyModeMigration(cached);
      await invoke('save_settings_blob', { blob: cached });
    } else if (ownsSettingsMigrations) {
      // A fresh install. Commit the defaults so companions that fail closed
      // on a missing document (the live-words preview) see default-on.
      const defaults = JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: SETTINGS_VERSION });
      localStorage.setItem(STORAGE_KEY, defaults);
      await invoke('save_settings_blob', { blob: defaults });
    }
  } catch (e) {
    // Boot must never block on the settings store; localStorage stays the
    // fallback for this session.
    console.error('Failed to hydrate settings from disk:', e);
  }
}

/**
 * Validate a persisted code-vocab scan summary. Returns a clean
 * `VocabScanSummary` only when every field has the expected type; otherwise
 * `null` (treated as "never scanned"). Keeps a malformed/partial blob from
 * rendering NaN counts or a non-array chip list in the done-state.
 */
function sanitizeVocabScan(raw: unknown): VocabScanSummary | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const nums = ['files', 'skipped', 'terms', 'bytes', 'ms'] as const;
  for (const k of nums) {
    if (typeof r[k] !== 'number' || !Number.isFinite(r[k] as number)) return null;
  }
  if (typeof r.capped !== 'boolean') return null;
  if (!Array.isArray(r.sampleTerms)) return null;

  // rankedTerms is additive (absent on pre-feature blobs). Drop malformed
  // entries, keep only well-formed { term:string, freq:finite-number } rows,
  // and clamp the length to the backend cap so a bad blob can't bloat the modal.
  const rankedTerms: RankedTerm[] = Array.isArray(r.rankedTerms)
    ? (r.rankedTerms as unknown[])
        .filter((t): t is RankedTerm => {
          if (!t || typeof t !== 'object') return false;
          const e = t as Record<string, unknown>;
          return (
            typeof e.term === 'string' &&
            e.term.length > 0 &&
            typeof e.freq === 'number' &&
            Number.isFinite(e.freq)
          );
        })
        .slice(0, MAX_RANKED_TERMS)
        .map((t) => ({ term: t.term, freq: Math.max(0, Math.trunc(t.freq)) }))
    : [];

  // whisperCount is additive too; coerce anything non-finite to 0 and never let
  // it exceed how many ranked terms we actually have.
  const rawWhisper = r.whisperCount;
  const whisperCount =
    typeof rawWhisper === 'number' && Number.isFinite(rawWhisper)
      ? Math.max(0, Math.min(Math.trunc(rawWhisper), rankedTerms.length))
      : 0;

  // Counts passed the finite check above; coerce to non-negative integers so a
  // tampered blob can't surface negative/fractional stats (NaN already rejected).
  const count = (v: unknown) => Math.max(0, Math.trunc(v as number));
  return {
    files: count(r.files),
    skipped: count(r.skipped),
    terms: count(r.terms),
    bytes: count(r.bytes),
    ms: count(r.ms),
    capped: r.capped as boolean,
    // Bound the persisted sample list so a tampered blob can't bloat the chip row.
    sampleTerms: (r.sampleTerms as unknown[])
      .filter((t): t is string => typeof t === 'string')
      .slice(0, MAX_SAMPLE_TERMS),
    rankedTerms,
    whisperCount,
    // Added after persisted summaries first shipped; old successful summaries
    // predate the field and are therefore treated as adopted.
    adopted: typeof r.adopted === 'boolean' ? r.adopted : true,
  };
}

/** Decode a durable snapshot without rewriting a webview's cache or disk. */
export function parseSettingsBlob(blob: string | null): Settings {
  return settingsFromBlob(blob, false);
}

export function loadSettings(): Settings {
  try { return settingsFromBlob(localStorage.getItem(STORAGE_KEY), true); }
  catch (e) { console.error('Failed to load settings:', e); return DEFAULT_SETTINGS; }
}

function settingsFromBlob(stored: string | null, repairCache: boolean): Settings {
  try {
    if (stored) {
      const parsed = JSON.parse(stored) as PersistedSettings;
      const storedSettingsVersion =
        typeof parsed.settingsVersion === 'number'
        && Number.isInteger(parsed.settingsVersion)
        && parsed.settingsVersion >= 0
          ? parsed.settingsVersion
          : 0;
      delete parsed.settingsVersion;
      for (const key of ['queryHotkey', 'queryProvider', 'queryExecutable', 'queryArguments',
        'queryTimeoutSeconds', 'queryContextLevel', 'queryAutomaticallyCopyAnswers', 'retainQueryHistory',
        'assistantConnection']) delete (parsed as unknown as Record<string, unknown>)[key];
      delete parsed.launchOnboardingComplete; // Launch metadata is not capture configuration.

      // v2: the original overlay calibration preview moved only DOM content,
      // committed on pointer-up, and could persist a +48pt launch displacement.
      // None of those offsets are trustworthy. Reset them once, then persist
      // future confirmed offsets inside the durable Settings document.
      if (storedSettingsVersion < OVERLAY_CALIBRATION_MIGRATION_VERSION) {
        parsed.overlayVerticalOffset = DEFAULT_SETTINGS.overlayVerticalOffset;
      } else {
        parsed.overlayVerticalOffset = clampOverlayVerticalOffset(parsed.overlayVerticalOffset);
      }
      if (repairCache) localStorage.removeItem(LEGACY_OVERLAY_OFFSET_KEY);
      // v7: the below-menu-bar placement became the bottom floating pill.
      if (parsed.overlayPlacement === 'below_menu_bar') parsed.overlayPlacement = 'floating_pill';
      if (parsed.overlayPlacement !== 'notch' && parsed.overlayPlacement !== 'floating_pill') {
        parsed.overlayPlacement = DEFAULT_SETTINGS.overlayPlacement;
      }
      // v7: live words shipped off by default during the field redesign, so
      // stored `false` values are the old default rather than a user choice.
      if (
        typeof parsed.overlayLiveWords !== 'boolean'
        || storedSettingsVersion < LIVE_WORDS_DEFAULT_ON_MIGRATION_VERSION
      ) {
        parsed.overlayLiveWords = DEFAULT_SETTINGS.overlayLiveWords;
      }
      // The processing-style picker belonged to the removed dot field.
      delete parsed.overlayProcessingStyle;

      // v1: installs that persisted the former 50 ms default should adopt the
      // zero-delay fast path once. A user can set 50 ms again after migration,
      // and saveSettings will retain it with the current settings version.
      if (
        storedSettingsVersion < ZERO_DELAY_MIGRATION_VERSION
        && parsed.autoPasteDelayMs === 50
      ) {
        parsed.autoPasteDelayMs = DEFAULT_SETTINGS.autoPasteDelayMs;
      }

      // Migrate: 'hotkey' mode no longer exists → default to 'hold_down'
      const validModes: RecordingMode[] = ['hold_down', 'double_tap', 'both'];
      if (!parsed.recordingMode || !validModes.includes(parsed.recordingMode as RecordingMode)) {
        parsed.recordingMode = DEFAULT_SETTINGS.recordingMode;
      }

      // The trigger crosses localStorage and the durable settings file before
      // reaching the native listener. Only install keys represented by the
      // shared picker catalog; malformed or future values return to the safe
      // default instead of silently disabling keyboard recording.
      if (!isDictationKey(parsed.doubleTapKey)) {
        parsed.doubleTapKey = DEFAULT_SETTINGS.doubleTapKey;
      }

      // Remove legacy hotkey field if present
      delete parsed.hotkey;
      // The removed live-preview feature must not remain in persisted settings.
      delete parsed.liveTranscriptPreview;

      // Older blobs stored either an opaque CoreAudio UID or a display name in
      // the same string field. Only the default sentinel is self-authenticating;
      // all other old values remain pending until checked against a live list.
      if (parsed.microphone === 'system_default') {
        parsed.microphoneIdMigrationComplete = true;
      } else if (typeof parsed.microphoneIdMigrationComplete !== 'boolean') {
        parsed.microphoneIdMigrationComplete = false;
      }

      parsed.captureStandbyEnabled = parsed.captureStandbyEnabled === true;
      parsed.captureStandbySeconds = [60, 300, 900].includes(parsed.captureStandbySeconds as number) ? parsed.captureStandbySeconds : 300;
      parsed.capturePreRollMs = [300, 450].includes(parsed.capturePreRollMs as number) ? parsed.capturePreRollMs : 450;
      parsed.smartAutoMicrophoneEnabled = parsed.smartAutoMicrophoneEnabled === true;
      // Background microphone access is a separate consent. Existing Smart Auto
      // users and malformed persisted values remain off after upgrade.
      parsed.smartAutoProbeEnabled = parsed.smartAutoProbeEnabled === true;
      const smartAutoApprovedDeviceIds = sanitizeSmartAutoDeviceIds(parsed.smartAutoApprovedDeviceIds);
      parsed.smartAutoApprovedDeviceIds = smartAutoApprovedDeviceIds;
      parsed.smartAutoPreferredDeviceIds = sanitizeSmartAutoDeviceIds(parsed.smartAutoPreferredDeviceIds)
        .filter((id) => smartAutoApprovedDeviceIds.includes(id));
      parsed.smartAutoAllowContinuity = parsed.smartAutoAllowContinuity === true;

      // Validate model against current allow-list (includes Moonshine migration)
      const validModels = new Set<string>(AVAILABLE_MODEL_OPTIONS.map((m) => m.value));
      if (typeof parsed.model !== 'string' || !validModels.has(parsed.model)) {
        parsed.model = DEFAULT_SETTINGS.model;
      }

      // Validate language against current allow-list
      const validLanguages = new Set<string>(LANGUAGE_OPTIONS.map((o) => o.value));
      if (typeof parsed.language !== 'string' || !validLanguages.has(parsed.language)) {
        parsed.language = DEFAULT_SETTINGS.language;
      }

      // Native paste has a zero-delay fast path. Keep persisted values bounded
      // to the same range accepted by the Rust command.
      if (typeof parsed.autoPasteDelayMs !== 'number' || !Number.isFinite(parsed.autoPasteDelayMs)) {
        parsed.autoPasteDelayMs = DEFAULT_SETTINGS.autoPasteDelayMs;
      } else {
        parsed.autoPasteDelayMs = Math.max(0, Math.min(500, Math.trunc(parsed.autoPasteDelayMs)));
      }
      // Smart destination is an explicit opt-in. Missing or malformed values
      // fail closed rather than enabling native destination observation.
      if (typeof parsed.smartDestinationEnabled !== 'boolean') {
        parsed.smartDestinationEnabled = DEFAULT_SETTINGS.smartDestinationEnabled;
      }
      // v4 replaces the opt-in boolean with an explicit, allow-listed chord.
      // Preserve the old working ⌘⇧V binding for enabled upgrades. Any
      // malformed new value fails closed instead of arming an unexpected key.
      if (!Object.prototype.hasOwnProperty.call(parsed, 'pasteLastShortcut')) {
        parsed.pasteLastShortcut = parsed.pasteLastShortcutEnabled === true
          ? 'command_shift_v'
          : null;
      }
      delete parsed.pasteLastShortcutEnabled;
      {
        const validPasteLastShortcuts = new Set<string>(
          PASTE_LAST_SHORTCUT_OPTIONS
            .map((option) => option.value)
            .filter((value) => value !== 'disabled'),
        );
        if (
          parsed.pasteLastShortcut !== null
          && (
            typeof parsed.pasteLastShortcut !== 'string'
            || !validPasteLastShortcuts.has(parsed.pasteLastShortcut)
            || pasteLastShortcutConflict(parsed.pasteLastShortcut, {
              doubleTapKey: parsed.doubleTapKey ?? DEFAULT_SETTINGS.doubleTapKey,
            }) !== null
          )
        ) {
          parsed.pasteLastShortcut = null;
        }
      }

      // outputDir feeds a filesystem path on the Rust side — coerce anything
      // non-string back to the default (empty = app-chosen Documents/Murmur).
      if (typeof parsed.outputDir !== 'string') {
        parsed.outputDir = DEFAULT_SETTINGS.outputDir;
      }

      // benchmarkOutputDir also feeds a filesystem path on the Rust side.
      if (typeof parsed.benchmarkOutputDir !== 'string') {
        parsed.benchmarkOutputDir = DEFAULT_SETTINGS.benchmarkOutputDir;
      }
      if (typeof parsed.benchmarkAutoSave !== 'boolean') {
        parsed.benchmarkAutoSave = DEFAULT_SETTINGS.benchmarkAutoSave;
      }

      parsed.vocabularyEntries = sanitizeVocabularyEntries(
        parsed.vocabularyEntries,
        parsed.customVocabulary,
      );
      // Keep the legacy field as a derived compatibility mirror. It is never an
      // independently editable source after migration.
      parsed.customVocabulary = vocabularyPrompt(parsed.vocabularyEntries);

      // Modes were retired. Capture what they persisted before anything is
      // dropped, then fold it into profiles and globals below.
      const legacyModes = readLegacyModeState(parsed as unknown as Record<string, unknown>);

      // appProfiles drives per-app delivery and transformation overrides. Drop
      // malformed entries and coerce a non-array back to the empty default so
      // the Rust side and UI never see a bad shape.
      if (!Array.isArray(parsed.appProfiles)) {
        parsed.appProfiles = DEFAULT_SETTINGS.appProfiles;
      } else {
        parsed.appProfiles = parsed.appProfiles
          .filter((p): p is AppProfile & { modeId?: unknown } =>
            !!p && typeof (p as AppProfile).bundleId === 'string' && (p as AppProfile).bundleId.trim() !== '')
          .map((p): AppProfile & { modeId?: string } => ({
            bundleId: p.bundleId.trim(),
            label: typeof p.label === 'string' ? p.label : '',
            autoPasteOverride:
              typeof p.autoPasteOverride === 'boolean' ? p.autoPasteOverride : null,
            cleanupOverride:
              typeof p.cleanupOverride === 'boolean' ? p.cleanupOverride : null,
            smartFormattingOverride:
              typeof p.smartFormattingOverride === 'boolean' ? p.smartFormattingOverride : null,
            cliFormattingOverride:
              typeof p.cliFormattingOverride === 'boolean' ? p.cliFormattingOverride : null,
            writingStyle:
              typeof p.writingStyle === 'string' &&
              (WRITING_STYLE_VALUES as readonly string[]).includes(p.writingStyle)
                ? p.writingStyle as WritingStyle
                : null,
            ideContextEnabled: typeof p.ideContextEnabled === 'boolean' ? p.ideContextEnabled : false,
            ideProjectRoots: Array.isArray(p.ideProjectRoots)
              ? p.ideProjectRoots
                  .filter((root): root is string => typeof root === 'string' && root.trim().length > 0)
                  .map((root) => root.trim())
                  .filter((root) => root.length <= MAX_IDE_PROJECT_ROOT_BYTES)
                  .filter((root, index, roots) => roots.indexOf(root) === index)
                  .slice(0, 4)
              : [],
            // Carried only so the legacy Mode migration can read the binding.
            ...(typeof p.modeId === 'string' && p.modeId.trim()
              ? { modeId: p.modeId.trim() }
              : {}),
          }));
      }

      // voiceCommands: array of { phrase, replacement }. Drop malformed entries
      // and coerce a non-array (or absent on older blobs) back to the default.
      if (!Array.isArray(parsed.voiceCommands)) {
        parsed.voiceCommands = DEFAULT_SETTINGS.voiceCommands;
      } else {
        parsed.voiceCommands = parsed.voiceCommands
          .filter((c): c is VoiceCommand =>
            !!c && typeof (c as VoiceCommand).phrase === 'string' && (c as VoiceCommand).phrase.trim() !== '')
          .map((c) => ({
            phrase: c.phrase.trim(),
            replacement: typeof c.replacement === 'string' ? c.replacement : '',
          }));
      }

      // cleanup sub-toggles default to on; coerce non-booleans back to the default.
      if (typeof parsed.cleanupRemoveFiller !== 'boolean') {
        parsed.cleanupRemoveFiller = DEFAULT_SETTINGS.cleanupRemoveFiller;
      }
      if (typeof parsed.cleanupCapitalize !== 'boolean') {
        parsed.cleanupCapitalize = DEFAULT_SETTINGS.cleanupCapitalize;
      }

      // Voice commands gate the Rust transform — coerce non-booleans (or a
      // missing field on pre-feature stored settings) back to the default.
      if (typeof parsed.voiceCommandsEnabled !== 'boolean') {
        parsed.voiceCommandsEnabled = DEFAULT_SETTINGS.voiceCommandsEnabled;
      }

      // cleanupEnabled is a boolean toggle — coerce anything non-boolean
      // (including absent on older settings blobs) back to the default.
      if (typeof parsed.cleanupEnabled !== 'boolean') {
        parsed.cleanupEnabled = DEFAULT_SETTINGS.cleanupEnabled;
      }

      // Smart formatting is an explicit opt-in. Older or malformed settings
      // stay off rather than silently enabling broad prose transformations.
      if (typeof parsed.smartFormattingEnabled !== 'boolean') {
        parsed.smartFormattingEnabled = DEFAULT_SETTINGS.smartFormattingEnabled;
      }

      if (typeof parsed.hotkeyMissFeedback !== 'boolean') {
        parsed.hotkeyMissFeedback = DEFAULT_SETTINGS.hotkeyMissFeedback;
      }

      if (typeof parsed.soundCuesEnabled !== 'boolean') {
        parsed.soundCuesEnabled = DEFAULT_SETTINGS.soundCuesEnabled;
      }
      parsed.soundCueVolume = typeof parsed.soundCueVolume === 'number'
        && Number.isFinite(parsed.soundCueVolume)
        ? Math.max(0, Math.min(100, Math.round(parsed.soundCueVolume)))
        : DEFAULT_SETTINGS.soundCueVolume;
      if (typeof parsed.meetingSoundCuesEnabled !== 'boolean') {
        parsed.meetingSoundCuesEnabled = DEFAULT_SETTINGS.meetingSoundCuesEnabled;
      }

      if (typeof parsed.retainHistory !== 'boolean') {
        parsed.retainHistory = DEFAULT_SETTINGS.retainHistory;
      }
      if (typeof parsed.meetingRetainAudio !== 'boolean') {
        parsed.meetingRetainAudio = DEFAULT_SETTINGS.meetingRetainAudio;
      }
      if (typeof parsed.meetingDiarization !== 'boolean') {
        parsed.meetingDiarization = DEFAULT_SETTINGS.meetingDiarization;
      }
      if (typeof parsed.meetingEchoCancellationEnabled !== 'boolean') {
        parsed.meetingEchoCancellationEnabled = DEFAULT_SETTINGS.meetingEchoCancellationEnabled;
      }
      if (typeof parsed.meetingSuggestionsEnabled !== 'boolean') {
        parsed.meetingSuggestionsEnabled = DEFAULT_SETTINGS.meetingSuggestionsEnabled;
      }
      if (
        typeof parsed.meetingRetentionDays !== 'number'
        || !Number.isFinite(parsed.meetingRetentionDays)
      ) {
        parsed.meetingRetentionDays = DEFAULT_SETTINGS.meetingRetentionDays;
      } else {
        parsed.meetingRetentionDays = Math.max(
          0,
          Math.min(3650, Math.trunc(parsed.meetingRetentionDays)),
        );
      }
      if (
        typeof parsed.meetingMaxSessions !== 'number'
        || !Number.isFinite(parsed.meetingMaxSessions)
      ) {
        parsed.meetingMaxSessions = DEFAULT_SETTINGS.meetingMaxSessions;
      } else {
        parsed.meetingMaxSessions = Math.max(
          1,
          Math.min(10_000, Math.trunc(parsed.meetingMaxSessions)),
        );
      }

      // autoStopSilenceMs ends a recording on its own, so an unrecognised or
      // tampered value must fall back to Off rather than to some arbitrary
      // duration. Absent on pre-feature blobs — also Off.
      {
        const validDurations = new Set<number>(AUTO_STOP_SILENCE_OPTIONS.map((o) => o.value));
        if (
          typeof parsed.autoStopSilenceMs !== 'number'
          || !validDurations.has(parsed.autoStopSilenceMs)
        ) {
          parsed.autoStopSilenceMs = DEFAULT_SETTINGS.autoStopSilenceMs;
        }
      }
      // Smart Auto-Stop can also end a recording on its own: only an explicit
      // true turns it on.
      parsed.smartAutoStopEnabled = parsed.smartAutoStopEnabled === true;
      // codeVocabEnabled gates the Rust scan — coerce non-booleans (or a missing
      // field on pre-feature stored settings) back to the default.
      if (typeof parsed.codeVocabEnabled !== 'boolean') {
        parsed.codeVocabEnabled = DEFAULT_SETTINGS.codeVocabEnabled;
      }

      // codeVocabFolder feeds a filesystem path on the Rust side — coerce
      // anything non-string back to the empty default.
      if (typeof parsed.codeVocabFolder !== 'string') {
        parsed.codeVocabFolder = DEFAULT_SETTINGS.codeVocabFolder;
      }

      // codeVocabLastScan is a persisted scan summary (or null). Validate the
      // whole shape — a partial/malformed blob would render bad numbers in the
      // done-state, so coerce anything that doesn't match back to null.
      parsed.codeVocabLastScan = sanitizeVocabScan(parsed.codeVocabLastScan);

      // Correction toggles — coerce non-booleans (or absent on pre-feature blobs)
      // back to defaults. correctionEnabled defaults ON, so an older settings blob
      // that predates this field opts into correction (the intended migration).
      if (typeof parsed.correctionEnabled !== 'boolean') {
        parsed.correctionEnabled = DEFAULT_SETTINGS.correctionEnabled;
      }
      if (typeof parsed.correctionFuzzy !== 'boolean') {
        parsed.correctionFuzzy = DEFAULT_SETTINGS.correctionFuzzy;
      }

      if (typeof parsed.homophoneContextEnabled !== 'boolean') {
        parsed.homophoneContextEnabled = false;
      }

      let settings = { ...DEFAULT_SETTINGS, ...parsed } as Settings;
      if (legacyModes.present) {
        const migrated = applyLegacyModes(settings, legacyModes, {
          installedModelIds: installedModelIdsForLegacyMigration,
        });
        settings = {
          ...settings,
          model: migrated.model as ModelOption,
          language: migrated.language,
          autoPaste: migrated.autoPaste,
          cleanupEnabled: migrated.cleanupEnabled,
          smartFormattingEnabled: migrated.smartFormattingEnabled,
          appProfiles: migrated.appProfiles,
        };
        stripLegacyModeKeys(settings as unknown as Record<string, unknown>);
      }
      // The migrated document replaces the stored blob whole, so the legacy
      // keys leave storage only when that save succeeds. A failed save keeps
      // them, and the next load derives the same result again.
      if (
        repairCache && ownsSettingsMigrations
        && (storedSettingsVersion < SETTINGS_VERSION || legacyModes.present)
      ) {
        try {
          writePersistedSettings(settings);
        } catch (e) {
          console.error('Failed to persist settings migration:', e);
        }
      }
      return settings;
    }
    if (repairCache) localStorage.removeItem(LEGACY_OVERLAY_OFFSET_KEY);
  } catch (e) {
    console.error('Failed to load settings:', e);
  }
  return DEFAULT_SETTINGS;
}

/** Refresh a webview cache after the host has already committed settings. */
export function cacheSettings(settings: Settings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...settings, settingsVersion: SETTINGS_VERSION }));
  } catch (e) {
    console.error('Failed to cache settings:', e);
  }
}

export function saveSettings(settings: Settings): void {
  try {
    writePersistedSettings(settings);
  } catch (e) {
    console.error('Failed to save settings:', e);
  }
}
