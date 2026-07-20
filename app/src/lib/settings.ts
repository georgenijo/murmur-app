export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

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
  language: string;
  autoPaste: boolean;
  autoPasteDelayMs: number;
  recordingMode: RecordingMode;
  hotkeyMissFeedback: boolean;
  microphone: string;
  launchAtLogin: boolean;
  vadSensitivity: number;
  idleTimeoutMinutes: number;
  /** @deprecated Migration-only mirror; structured entries are authoritative. */
  customVocabulary: string;
  vocabularyEntries: VocabularyEntry[];
  disabled: boolean;
  smartPunctuation: boolean;
  saveTranscript: boolean;
  saveAudio: boolean;
  outputDir: string;
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

export function isMacOSPlatform(platform: string): boolean {
  return platform.startsWith('Mac');
}

export function modelOptionsForPlatform(platform: string): typeof MODEL_OPTIONS {
  return isMacOSPlatform(platform)
    ? MODEL_OPTIONS
    : MODEL_OPTIONS.filter((model) => model.backend !== 'coreml');
}

export function defaultModelForPlatform(platform: string): ModelOption {
  return isMacOSPlatform(platform)
    ? 'parakeet-tdt-0.6b-v3-coreml'
    : 'parakeet-tdt-0.6b-v2-fp16';
}

const runtimePlatform = typeof navigator === 'undefined' ? '' : navigator.platform;
export const AVAILABLE_MODEL_OPTIONS = modelOptionsForPlatform(runtimePlatform);

export const DOUBLE_TAP_KEY_OPTIONS: { value: DoubleTapKey; label: string }[] = [
  { value: 'shift_l', label: 'Shift' },
  { value: 'alt_l', label: 'Option' },
  { value: 'ctrl_r', label: 'Control' },
];

export const RECORDING_MODE_OPTIONS: { value: RecordingMode; label: string }[] = [
  { value: 'hold_down', label: 'Hold Down' },
  { value: 'double_tap', label: 'Double-Tap' },
  { value: 'both', label: 'Both' },
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
  model: defaultModelForPlatform(runtimePlatform),
  doubleTapKey: 'shift_l',
  // 'auto' lets Whisper auto-detect the spoken language ("just works"); the
  // non-Whisper models may auto-detect or ignore this value.
  language: 'auto',
  autoPaste: false,
  autoPasteDelayMs: 50,
  recordingMode: 'hold_down',
  hotkeyMissFeedback: false,
  microphone: 'system_default',
  launchAtLogin: false,
  vadSensitivity: 50,
  idleTimeoutMinutes: 5,
  customVocabulary: '',
  vocabularyEntries: [],
  disabled: false,
  smartPunctuation: true,
  saveTranscript: false,
  saveAudio: false,
  outputDir: '',
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
};

export const STORAGE_KEY = 'dictation-settings';

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

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<Settings> & {
        hotkey?: string;
        liveTranscriptPreview?: unknown;
        recordingMode?: string;
      };

      // Migrate: 'hotkey' mode no longer exists → default to 'hold_down'
      const validModes: RecordingMode[] = ['hold_down', 'double_tap', 'both'];
      if (!parsed.recordingMode || !validModes.includes(parsed.recordingMode as RecordingMode)) {
        parsed.recordingMode = DEFAULT_SETTINGS.recordingMode;
      }

      // Remove legacy hotkey field if present
      delete parsed.hotkey;
      // The removed live-preview feature must not remain in persisted settings.
      delete parsed.liveTranscriptPreview;

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

      // outputDir feeds a filesystem path on the Rust side — coerce anything
      // non-string back to the default (empty = app-chosen Documents/Murmur).
      if (typeof parsed.outputDir !== 'string') {
        parsed.outputDir = DEFAULT_SETTINGS.outputDir;
      }

      parsed.vocabularyEntries = sanitizeVocabularyEntries(
        parsed.vocabularyEntries,
        parsed.customVocabulary,
      );
      // Keep the legacy field as a derived compatibility mirror. It is never an
      // independently editable source after migration.
      parsed.customVocabulary = vocabularyPrompt(parsed.vocabularyEntries);

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

      return { ...DEFAULT_SETTINGS, ...parsed } as Settings;
    }
  } catch (e) {
    console.error('Failed to load settings:', e);
  }
  return DEFAULT_SETTINGS;
}

export function saveSettings(settings: Settings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch (e) {
    console.error('Failed to save settings:', e);
  }
}
