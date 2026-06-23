export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

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
}

/**
 * A user-defined voice command. When `phrase` is spoken it is replaced by
 * `replacement` (case-insensitive, word-boundary). Applied after the built-in
 * commands, so users extend rather than override the defaults.
 */
export interface VoiceCommand {
  phrase: string;
  replacement: string;
}

export interface Settings {
  model: ModelOption;
  doubleTapKey: DoubleTapKey;
  language: string;
  autoPaste: boolean;
  autoPasteDelayMs: number;
  recordingMode: RecordingMode;
  microphone: string;
  launchAtLogin: boolean;
  vadSensitivity: number;
  idleTimeoutMinutes: number;
  customVocabulary: string;
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
   * Post-model correction: apply the vocabulary to the transcript *output* of every
   * backend (Tier 1 exact map + Tier 2 sounds-like). On by default — it's what makes
   * vocab work on the default Parakeet engine, which ignores Whisper's prompt.
   */
  correctionEnabled: boolean;
  /** Tier 2 phonetic "sounds-like" matching. Gated under correctionEnabled. */
  correctionFuzzy: boolean;
  /** Tier 3 local-LLM cleanup pass for context mishears. Opt-in, default off. */
  correctionModelEnabled: boolean;
  /** Tier 3 "fast mode": the smaller, faster local model variant. */
  correctionModelFast: boolean;
}

export type ModelOption =
  | 'tiny.en'
  | 'base.en'
  | 'small.en'
  | 'medium.en'
  | 'large-v3-turbo'
  // --- Parakeet backend (removable): delete this member to remove. ---
  | 'parakeet-tdt-0.6b-v2-fp16';

export type TranscriptionBackend = 'whisper' | 'parakeet';

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string; backend: TranscriptionBackend }[] = [
  { value: 'tiny.en', label: 'Whisper Tiny (English)', size: '~75 MB', backend: 'whisper' },
  { value: 'base.en', label: 'Whisper Base (English)', size: '~150 MB', backend: 'whisper' },
  { value: 'small.en', label: 'Whisper Small (English)', size: '~500 MB', backend: 'whisper' },
  { value: 'medium.en', label: 'Whisper Medium (English)', size: '~1.5 GB', backend: 'whisper' },
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~3 GB', backend: 'whisper' },
  // --- Parakeet backend (removable): delete this entry to remove. ---
  { value: 'parakeet-tdt-0.6b-v2-fp16', label: 'Parakeet TDT 0.6B (English, fast)', size: '~1.2 GB', backend: 'parakeet' },
];

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
  // Parakeet fp16 is the default transcription model (faster; English-only).
  model: 'parakeet-tdt-0.6b-v2-fp16',
  doubleTapKey: 'shift_l',
  // 'auto' lets Whisper auto-detect the spoken language ("just works"); the
  // default Parakeet model is English-only and ignores this value.
  language: 'auto',
  autoPaste: false,
  autoPasteDelayMs: 50,
  recordingMode: 'hold_down',
  microphone: 'system_default',
  launchAtLogin: false,
  vadSensitivity: 50,
  idleTimeoutMinutes: 5,
  customVocabulary: '',
  disabled: false,
  smartPunctuation: true,
  saveTranscript: false,
  saveAudio: false,
  outputDir: '',
  appProfiles: [],
  voiceCommandsEnabled: false,
  voiceCommands: [],
  cleanupEnabled: false,
  cleanupRemoveFiller: true,
  cleanupCapitalize: true,
  codeVocabEnabled: false,
  codeVocabFolder: '',
  // Correction on by default: it's the fix that makes vocab actually apply on the
  // default Parakeet engine. A no-op when there's no vocabulary configured.
  correctionEnabled: true,
  correctionFuzzy: true,
  correctionModelEnabled: false,
  correctionModelFast: false,
};

export const STORAGE_KEY = 'dictation-settings';

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<Settings> & { hotkey?: string; recordingMode?: string };

      // Migrate: 'hotkey' mode no longer exists → default to 'hold_down'
      const validModes: RecordingMode[] = ['hold_down', 'double_tap', 'both'];
      if (!parsed.recordingMode || !validModes.includes(parsed.recordingMode as RecordingMode)) {
        parsed.recordingMode = DEFAULT_SETTINGS.recordingMode;
      }

      // Remove legacy hotkey field if present
      delete parsed.hotkey;

      // Validate model against current allow-list (includes Moonshine migration)
      const validModels = new Set<string>(MODEL_OPTIONS.map((m) => m.value));
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

      // appProfiles drives per-app auto-paste overrides. Drop malformed entries
      // and coerce a non-array back to the empty default so the Rust side and UI
      // never see a bad shape.
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

      // Correction toggles — coerce non-booleans (or absent on pre-feature blobs)
      // back to defaults. correctionEnabled defaults ON, so an older settings blob
      // that predates this field opts into correction (the intended migration).
      if (typeof parsed.correctionEnabled !== 'boolean') {
        parsed.correctionEnabled = DEFAULT_SETTINGS.correctionEnabled;
      }
      if (typeof parsed.correctionFuzzy !== 'boolean') {
        parsed.correctionFuzzy = DEFAULT_SETTINGS.correctionFuzzy;
      }
      if (typeof parsed.correctionModelEnabled !== 'boolean') {
        parsed.correctionModelEnabled = DEFAULT_SETTINGS.correctionModelEnabled;
      }
      if (typeof parsed.correctionModelFast !== 'boolean') {
        parsed.correctionModelFast = DEFAULT_SETTINGS.correctionModelFast;
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
