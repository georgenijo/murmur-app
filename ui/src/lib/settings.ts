export type RecordingMode = 'hold_down' | 'double_tap';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

export interface Settings {
  model: ModelOption;
  doubleTapKey: DoubleTapKey;
  language: string;
  autoPaste: boolean;
  recordingMode: RecordingMode;
}

export type ModelOption =
  | 'moonshine-tiny'
  | 'moonshine-base'
  | 'tiny.en'
  | 'base.en'
  | 'small.en'
  | 'medium.en'
  | 'large-v3-turbo';

export type TranscriptionBackend = 'whisper' | 'moonshine';

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string; backend: TranscriptionBackend }[] = [
  { value: 'moonshine-tiny', label: 'Moonshine Tiny (Fastest)', size: '~124 MB', backend: 'moonshine' },
  { value: 'moonshine-base', label: 'Moonshine Base', size: '~286 MB', backend: 'moonshine' },
  { value: 'tiny.en', label: 'Whisper Tiny (English)', size: '~75 MB', backend: 'whisper' },
  { value: 'base.en', label: 'Whisper Base (English)', size: '~150 MB', backend: 'whisper' },
  { value: 'small.en', label: 'Whisper Small (English)', size: '~500 MB', backend: 'whisper' },
  { value: 'medium.en', label: 'Whisper Medium (English)', size: '~1.5 GB', backend: 'whisper' },
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~3 GB', backend: 'whisper' },
];

export const MOONSHINE_MODELS = MODEL_OPTIONS.filter(m => m.backend === 'moonshine');
export const WHISPER_MODELS = MODEL_OPTIONS.filter(m => m.backend === 'whisper');

export const DOUBLE_TAP_KEY_OPTIONS: { value: DoubleTapKey; label: string }[] = [
  { value: 'shift_l', label: 'Shift' },
  { value: 'alt_l', label: 'Option' },
  { value: 'ctrl_r', label: 'Control' },
];

export const RECORDING_MODE_OPTIONS: { value: RecordingMode; label: string }[] = [
  { value: 'hold_down', label: 'Hold Down' },
  { value: 'double_tap', label: 'Double-Tap' },
];

export const DEFAULT_SETTINGS: Settings = {
  model: 'moonshine-tiny',
  doubleTapKey: 'shift_l',
  language: 'en',
  autoPaste: false,
  recordingMode: 'hold_down',
};

const STORAGE_KEY = 'dictation-settings';

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<Settings> & { hotkey?: string; recordingMode?: string };

      // Migrate: 'hotkey' mode no longer exists → default to 'hold_down'
      const validModes: RecordingMode[] = ['hold_down', 'double_tap'];
      if (!parsed.recordingMode || !validModes.includes(parsed.recordingMode as RecordingMode)) {
        parsed.recordingMode = DEFAULT_SETTINGS.recordingMode;
      }

      // Remove legacy hotkey field if present
      delete parsed.hotkey;

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
