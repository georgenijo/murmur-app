export type RecordingMode = 'hotkey' | 'double_tap';

export type DoubleTapKey = 'shift_l' | 'alt_l' | 'ctrl_r';

export interface Settings {
  model: ModelOption;
  hotkey: string;
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

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string }[] = [
  { value: 'moonshine-tiny', label: 'Moonshine Tiny (Fastest)', size: '~124 MB' },
  { value: 'moonshine-base', label: 'Moonshine Base', size: '~286 MB' },
  { value: 'tiny.en', label: 'Whisper Tiny (English)', size: '~75 MB' },
  { value: 'base.en', label: 'Whisper Base (English)', size: '~150 MB' },
  { value: 'small.en', label: 'Whisper Small (English)', size: '~500 MB' },
  { value: 'medium.en', label: 'Whisper Medium (English)', size: '~1.5 GB' },
  { value: 'large-v3-turbo', label: 'Whisper Large Turbo', size: '~3 GB' },
];

export const DOUBLE_TAP_KEY_OPTIONS: { value: DoubleTapKey; label: string }[] = [
  { value: 'shift_l', label: 'Shift' },
  { value: 'alt_l', label: 'Option' },
  { value: 'ctrl_r', label: 'Control' },
];

export const RECORDING_MODE_OPTIONS: { value: RecordingMode; label: string }[] = [
  { value: 'hotkey', label: 'Key Combo' },
  { value: 'double_tap', label: 'Double-Tap' },
];

export const DEFAULT_SETTINGS: Settings = {
  model: 'large-v3-turbo',
  hotkey: 'Shift+Space',
  doubleTapKey: 'shift_l',
  language: 'en',
  autoPaste: false,
  recordingMode: 'hotkey',
};

const STORAGE_KEY = 'dictation-settings';

// Legacy rdev key names → Tauri shortcut format
const LEGACY_HOTKEY_MAP: Record<string, string> = {
  'shift_l': 'Shift+Space',
  'shift_r': 'Shift+Space',
  'alt_l': 'Alt+Space',
  'alt_r': 'Alt+Space',
  'ctrl_r': 'Ctrl+Space',
};

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<Settings>;

      const validModes: RecordingMode[] = ['hotkey', 'double_tap'];
      if (parsed.recordingMode && !validModes.includes(parsed.recordingMode)) {
        parsed.recordingMode = DEFAULT_SETTINGS.recordingMode;
      }

      // Migrate legacy rdev key names to Tauri shortcut format
      if (parsed.hotkey && LEGACY_HOTKEY_MAP[parsed.hotkey]) {
        parsed.hotkey = LEGACY_HOTKEY_MAP[parsed.hotkey];
      }

      return { ...DEFAULT_SETTINGS, ...parsed };
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
