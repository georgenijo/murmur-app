import { invoke, isTauri } from '@tauri-apps/api/core';

export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

/**
 * Independent hotkey for the AX-selection transform shortcut (issue #312).
 * Deliberately a distinct id set from `DoubleTapKey` (same `<modifier>_<side>`
 * naming style) rather than reusing it verbatim: the transform key is meant
 * to coexist with whichever dictation hotkey is configured, so the default
 * options live on the opposite side of the keyboard.
 */
export type TransformKey = 'alt_r' | 'ctrl_l' | 'shift_r';
export type QueryKey = TransformKey;
export type QueryProviderId = 'claude' | 'codex' | 'grok' | 'cursor' | 'custom';
export type QueryContextLevel = 'none' | 'application' | 'selection';

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
  /** Privacy deny override; can never enable Voice Query context by itself. */
  queryContextExcluded: boolean;
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

const MAX_VOCABULARY_ENTRIES = 500;
const MAX_VOCABULARY_ALIASES = 16;
const MAX_VOCABULARY_VALUE_CHARS = 256;

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
  doubleTapKey: DoubleTapKey;
  /** Independent transform-shortcut hotkey (issue #312). `null` = disabled;
   * no settings UI exposes this yet. */
  transformHoldKey: TransformKey | null;
  /** Independent double-tap voice-query shortcut. `null` keeps the integration off. */
  queryHotkey: QueryKey | null;
  /** Provider preset metadata; `custom` preserves the original generic bridge. */
  queryProvider: QueryProviderId;
  /** Absolute path to the exact user-selected CLI executable. */
  queryExecutable: string;
  /** Fixed argv elements placed before the one-element spoken question. */
  queryArguments: string[];
  queryTimeoutSeconds: number;
  /** Optional context appended inside the one literal query argv element. */
  queryContextLevel: QueryContextLevel;
  /** Copy successful final answers to the clipboard; snapshotted per query pass. */
  queryAutomaticallyCopyAnswers: boolean;
  /** Opt-in Rust-owned local question/answer history; false keeps content ephemeral. */
  retainQueryHistory: boolean;
  language: string;
  autoPaste: boolean;
  autoPasteDelayMs: number;
  /** Opt-in global Command-Shift-V shortcut for Paste Last. */
  pasteLastShortcutEnabled: boolean;
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
  microphone: string;
  /** True once `microphone` is proven to be a backend ID or System Default. */
  microphoneIdMigrationComplete: boolean;
  launchAtLogin: boolean;
  /** Confirmed vertical fine-tuning for the notch overlay, in logical points. */
  overlayVerticalOffset: number;
  vadSensitivity: number;
  idleTimeoutMinutes: number;
  /** @deprecated Migration-only mirror; structured entries are authoritative. */
  customVocabulary: string;
  vocabularyEntries: VocabularyEntry[];
  disabled: boolean;
  smartPunctuation: boolean;
  /** Persist completed microphone and file transcripts in local history. */
  retainHistory: boolean;
  /** Keep meeting chunk WAV files after their durable transcript commits. */
  meetingRetainAudio: boolean;
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
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~3 GB', backend: 'whisper' },
  // --- Parakeet backend (removable): delete this entry to remove. ---
  { value: 'parakeet-tdt-0.6b-v2-fp16', label: 'Parakeet TDT 0.6B (English, fast)', size: '~1.2 GB', backend: 'parakeet' },
];

export const AVAILABLE_MODEL_OPTIONS = MODEL_OPTIONS;

export const DOUBLE_TAP_KEY_OPTIONS: { value: DoubleTapKey; label: string }[] = [
  { value: 'shift_l', label: 'Shift' },
  { value: 'alt_l', label: 'Option' },
  { value: 'ctrl_r', label: 'Control' },
];

/** Allow-list of transform hold-key options, shared by the Settings Transform
 * section's picker (issue #312 D1) and migration/validation, so both draw from
 * a single source of truth. Kept alongside `DOUBLE_TAP_KEY_OPTIONS`. */
export const TRANSFORM_KEY_OPTIONS: { value: TransformKey; label: string }[] = [
  { value: 'alt_r', label: 'Right Option' },
  { value: 'ctrl_l', label: 'Left Control' },
  { value: 'shift_r', label: 'Right Shift' },
];

export const QUERY_KEY_OPTIONS: { value: QueryKey; label: string }[] = TRANSFORM_KEY_OPTIONS;

export const QUERY_CONTEXT_LEVEL_OPTIONS: { value: QueryContextLevel; label: string }[] = [
  { value: 'none', label: 'None' },
  { value: 'application', label: 'App & window' },
  { value: 'selection', label: 'App, window & selection' },
];

export const RECORDING_MODE_OPTIONS: { value: RecordingMode; label: string }[] = [
  { value: 'hold_down', label: 'Hold Down' },
  { value: 'double_tap', label: 'Double-Tap' },
  { value: 'both', label: 'Both' },
];

/** Allow-list for `autoStopSilenceMs`. Anything else coerces back to Off. */
export const AUTO_STOP_SILENCE_OPTIONS: { value: number; label: string }[] = [
  { value: 0, label: 'Off' },
  { value: 1500, label: '1.5s' },
  { value: 2500, label: '2.5s' },
  { value: 4000, label: '4s' },
];

export const IDLE_TIMEOUT_OPTIONS: { value: number; label: string }[] = [
  { value: 5, label: '5 minutes' },
  { value: 15, label: '15 minutes' },
  { value: 0, label: 'Never' },
];

export const LANGUAGE_OPTIONS: { value: string; label: string }[] = [
  { value: 'auto', label: 'Auto Detect' },
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
  transformHoldKey: null,
  queryHotkey: null,
  queryProvider: 'custom',
  queryExecutable: '',
  queryArguments: [],
  queryTimeoutSeconds: 60,
  queryContextLevel: 'none',
  queryAutomaticallyCopyAnswers: true,
  retainQueryHistory: false,
  // 'auto' lets Whisper auto-detect the spoken language ("just works"); the
  // non-Whisper models may auto-detect or ignore this value.
  language: 'auto',
  autoPaste: false,
  // Native CGEvents can paste immediately in the common case. Apps that move
  // focus asynchronously can still opt into a settling delay in Settings.
  autoPasteDelayMs: 0,
  pasteLastShortcutEnabled: false,
  recordingMode: 'hold_down',
  hotkeyMissFeedback: false,
  soundCuesEnabled: true,
  soundCueVolume: 45,
  meetingSoundCuesEnabled: false,
  // Opt-in: a recording that ends itself is a surprise until you ask for it.
  autoStopSilenceMs: 0,
  microphone: 'system_default',
  microphoneIdMigrationComplete: true,
  launchAtLogin: false,
  overlayVerticalOffset: 0,
  vadSensitivity: 50,
  idleTimeoutMinutes: 5,
  customVocabulary: '',
  vocabularyEntries: [],
  disabled: false,
  smartPunctuation: true,
  retainHistory: true,
  meetingRetainAudio: false,
  meetingRetentionDays: 0,
  meetingMaxSessions: 100,
  saveTranscript: false,
  saveAudio: false,
  mirrorToNotchPill: false,
  outputDir: '',
  benchmarkOutputDir: '',
  benchmarkAutoSave: false,
  appProfiles: [],
  modes: [],
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
};

export const STORAGE_KEY = 'dictation-settings';
export const LEGACY_OVERLAY_OFFSET_KEY = 'murmur-overlay-vertical-offset';
export const OVERLAY_VERTICAL_OFFSET_MIN = -12;
export const OVERLAY_VERTICAL_OFFSET_MAX = 12;
const SETTINGS_VERSION = 3;
const ZERO_DELAY_MIGRATION_VERSION = 1;
const OVERLAY_CALIBRATION_MIGRATION_VERSION = 2;

export function clampOverlayVerticalOffset(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return DEFAULT_SETTINGS.overlayVerticalOffset;
  }
  return Math.max(
    OVERLAY_VERTICAL_OFFSET_MIN,
    Math.min(OVERLAY_VERTICAL_OFFSET_MAX, Math.trunc(value)),
  );
}

type PersistedSettings = Partial<Settings> & {
  settingsVersion?: unknown;
  hotkey?: string;
  liveTranscriptPreview?: unknown;
  recordingMode?: string;
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
  });
  localStorage.setItem(STORAGE_KEY, blob);
  mirrorToDisk(blob);
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
      return;
    }
    // No durable copy: either a first run, or an existing install whose
    // settings only ever lived in localStorage. Repair it once.
    const cached = localStorage.getItem(STORAGE_KEY);
    if (cached !== null) {
      await invoke('save_settings_blob', { blob: cached });
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

function sanitizeModes(raw: unknown): MurmurMode[] {
  if (!Array.isArray(raw)) return [];
  const models = new Set(AVAILABLE_MODEL_OPTIONS.map((option) => option.value));
  const languages = new Set(LANGUAGE_OPTIONS.map((option) => option.value));
  const styles = new Set<WritingStyle>(['conversational', 'polished', 'code_technical', 'verbatim', 'notes']);
  const seen = new Set(BUILTIN_MODES.map((mode) => mode.id));
  const modes: MurmurMode[] = [];
  for (const value of raw.slice(0, 100)) {
    if (!value || typeof value !== 'object') continue;
    const mode = value as Partial<MurmurMode>;
    const id = typeof mode.id === 'string' ? mode.id.trim() : '';
    const name = typeof mode.name === 'string' ? mode.name.trim() : '';
    if (!id || !name || id.length > 128 || name.length > 128 || seen.has(id)) continue;
    if (mode.builtIn === true) continue;
    if (mode.modelId != null && (typeof mode.modelId !== 'string' || !models.has(mode.modelId))) continue;
    if (mode.language != null && (typeof mode.language !== 'string' || !languages.has(mode.language))) continue;
    if (mode.writingStyle != null && !styles.has(mode.writingStyle as WritingStyle)) continue;
    if ([mode.enabled, mode.cleanupEnabled, mode.smartFormattingEnabled, mode.cliFormattingEnabled, mode.autoPaste]
      .some((field) => field != null && typeof field !== 'boolean')) continue;
    if (mode.vocabularyPolicy != null && !['inherit', 'general', 'technical'].includes(mode.vocabularyPolicy)) continue;
    if (mode.contextPolicy != null && !['none', 'project'].includes(mode.contextPolicy)) continue;
    const writingStyle = mode.writingStyle === null || styles.has(mode.writingStyle as WritingStyle)
      ? mode.writingStyle as WritingStyle | null
      : null;
    const nullableBoolean = (input: unknown) => typeof input === 'boolean' ? input : null;
    modes.push({
      id, name, builtIn: false, enabled: mode.enabled !== false, writingStyle,
      cleanupEnabled: nullableBoolean(mode.cleanupEnabled),
      smartFormattingEnabled: nullableBoolean(mode.smartFormattingEnabled),
      cliFormattingEnabled: nullableBoolean(mode.cliFormattingEnabled),
      vocabularyPolicy: mode.vocabularyPolicy === 'technical' || mode.vocabularyPolicy === 'general'
        ? mode.vocabularyPolicy : 'inherit',
      contextPolicy: mode.contextPolicy === 'project' ? 'project' : 'none',
      modelId: typeof mode.modelId === 'string' && models.has(mode.modelId) ? mode.modelId as ModelOption : null,
      language: typeof mode.language === 'string' && languages.has(mode.language) ? mode.language : null,
      autoPaste: nullableBoolean(mode.autoPaste),
    });
    seen.add(id);
  }
  return modes;
}

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as PersistedSettings;
      const storedSettingsVersion =
        typeof parsed.settingsVersion === 'number'
        && Number.isInteger(parsed.settingsVersion)
        && parsed.settingsVersion >= 0
          ? parsed.settingsVersion
          : 0;
      delete parsed.settingsVersion;

      // v2: the original overlay calibration preview moved only DOM content,
      // committed on pointer-up, and could persist a +48pt launch displacement.
      // None of those offsets are trustworthy. Reset them once, then persist
      // future confirmed offsets inside the durable Settings document.
      if (storedSettingsVersion < OVERLAY_CALIBRATION_MIGRATION_VERSION) {
        parsed.overlayVerticalOffset = DEFAULT_SETTINGS.overlayVerticalOffset;
      } else {
        parsed.overlayVerticalOffset = clampOverlayVerticalOffset(parsed.overlayVerticalOffset);
      }
      localStorage.removeItem(LEGACY_OVERLAY_OFFSET_KEY);

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
      if (typeof parsed.pasteLastShortcutEnabled !== 'boolean') {
        parsed.pasteLastShortcutEnabled = DEFAULT_SETTINGS.pasteLastShortcutEnabled;
      }

      // transformHoldKey: `null` (disabled) or one of TRANSFORM_KEY_OPTIONS.
      // Anything else — including an absent field on pre-feature blobs, or a
      // tampered/unrecognised id — coerces back to disabled rather than
      // silently arming an unexpected shortcut.
      {
        const validTransformKeys = new Set<string>(TRANSFORM_KEY_OPTIONS.map((o) => o.value));
        if (
          parsed.transformHoldKey !== null
          && (typeof parsed.transformHoldKey !== 'string' || !validTransformKeys.has(parsed.transformHoldKey))
        ) {
          parsed.transformHoldKey = DEFAULT_SETTINGS.transformHoldKey;
        }
      }

      // Voice Query is disabled by default. Persisted/tampered values must
      // stay inside the same explicit opposite-side modifier allow-list as
      // Transform, and command configuration is bounded before it reaches IPC.
      {
        const validQueryKeys = new Set<string>(QUERY_KEY_OPTIONS.map((o) => o.value));
        if (
          parsed.queryHotkey !== null
          && (typeof parsed.queryHotkey !== 'string' || !validQueryKeys.has(parsed.queryHotkey))
        ) {
          parsed.queryHotkey = DEFAULT_SETTINGS.queryHotkey;
        }
      }
      if (typeof parsed.queryExecutable !== 'string') {
        parsed.queryExecutable = DEFAULT_SETTINGS.queryExecutable;
      } else {
        parsed.queryExecutable = parsed.queryExecutable.slice(0, 4096);
      }
      if (
        typeof parsed.queryProvider !== 'string'
        || !['claude', 'codex', 'grok', 'cursor', 'custom'].includes(parsed.queryProvider)
      ) {
        parsed.queryProvider = DEFAULT_SETTINGS.queryProvider;
      }
      if (!Array.isArray(parsed.queryArguments)) {
        parsed.queryArguments = DEFAULT_SETTINGS.queryArguments;
      } else {
        parsed.queryArguments = parsed.queryArguments
          .filter((argument): argument is string => typeof argument === 'string')
          .map((argument) => argument.slice(0, 4096))
          .slice(0, 32);
      }
      if (
        typeof parsed.queryTimeoutSeconds !== 'number'
        || !Number.isInteger(parsed.queryTimeoutSeconds)
        || parsed.queryTimeoutSeconds < 5
        || parsed.queryTimeoutSeconds > 300
      ) {
        parsed.queryTimeoutSeconds = DEFAULT_SETTINGS.queryTimeoutSeconds;
      }
      if (
        typeof parsed.queryContextLevel !== 'string'
        || !QUERY_CONTEXT_LEVEL_OPTIONS.some((option) => option.value === parsed.queryContextLevel)
      ) {
        parsed.queryContextLevel = DEFAULT_SETTINGS.queryContextLevel;
      }
      // Auto-copy shipped before it became configurable, so pre-feature and
      // malformed documents retain that behavior. An explicit false is kept.
      if (typeof parsed.queryAutomaticallyCopyAnswers !== 'boolean') {
        parsed.queryAutomaticallyCopyAnswers = DEFAULT_SETTINGS.queryAutomaticallyCopyAnswers;
      }
      if (typeof parsed.retainQueryHistory !== 'boolean') {
        parsed.retainQueryHistory = DEFAULT_SETTINGS.retainQueryHistory;
      }
      if (
        parsed.queryHotkey !== null
        && parsed.queryHotkey === parsed.transformHoldKey
      ) {
        parsed.queryHotkey = null;
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

      parsed.modes = sanitizeModes(parsed.modes);

      // appProfiles drives per-app delivery and transformation overrides. Drop
      // malformed entries and coerce a non-array back to the empty default so
      // the Rust side and UI never see a bad shape.
      if (!Array.isArray(parsed.appProfiles)) {
        parsed.appProfiles = DEFAULT_SETTINGS.appProfiles;
      } else {
        parsed.appProfiles = parsed.appProfiles
          .filter((p): p is AppProfile =>
            !!p && typeof (p as AppProfile).bundleId === 'string' && (p as AppProfile).bundleId.trim() !== '')
          .map((p) => ({
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
              ['conversational', 'polished', 'code_technical', 'verbatim', 'notes'].includes(p.writingStyle)
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
            queryContextExcluded:
              typeof p.queryContextExcluded === 'boolean' ? p.queryContextExcluded : false,
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

      const settings = { ...DEFAULT_SETTINGS, ...parsed } as Settings;
      if (storedSettingsVersion < SETTINGS_VERSION) {
        try {
          writePersistedSettings(settings);
        } catch (e) {
          console.error('Failed to persist settings migration:', e);
        }
      }
      return settings;
    }
    localStorage.removeItem(LEGACY_OVERLAY_OFFSET_KEY);
  } catch (e) {
    console.error('Failed to load settings:', e);
  }
  return DEFAULT_SETTINGS;
}

export function saveSettings(settings: Settings): void {
  try {
    writePersistedSettings(settings);
  } catch (e) {
    console.error('Failed to save settings:', e);
  }
}
