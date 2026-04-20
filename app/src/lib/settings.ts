export type RecordingMode = 'hold_down' | 'double_tap' | 'both';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

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
  showTimingMissFeedback: boolean;
}

export type ModelOption =
  | 'tiny.en'
  | 'base.en'
  | 'small.en'
  | 'medium.en'
  | 'large-v3-turbo';

export type TranscriptionBackend = 'whisper';

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string; backend: TranscriptionBackend }[] = [
  { value: 'tiny.en', label: 'Whisper Tiny (English)', size: '~75 MB', backend: 'whisper' },
  { value: 'base.en', label: 'Whisper Base (English)', size: '~150 MB', backend: 'whisper' },
  { value: 'small.en', label: 'Whisper Small (English)', size: '~500 MB', backend: 'whisper' },
  { value: 'medium.en', label: 'Whisper Medium (English)', size: '~1.5 GB', backend: 'whisper' },
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~3 GB', backend: 'whisper' },
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

export const DEFAULT_SETTINGS: Settings = {
  model: 'base.en',
  doubleTapKey: 'shift_l',
  language: 'en',
  autoPaste: false,
  autoPasteDelayMs: 50,
  recordingMode: 'hold_down',
  microphone: 'system_default',
  launchAtLogin: false,
  vadSensitivity: 50,
  idleTimeoutMinutes: 5,
  customVocabulary: '',
  showTimingMissFeedback: false,
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
