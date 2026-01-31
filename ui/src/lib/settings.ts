export interface Settings {
  model: ModelOption;
  hotkey: HotkeyOption;
  language: string;
  autoPaste: boolean;
}

export type ModelOption =
  | 'tiny.en'
  | 'base.en'
  | 'small.en'
  | 'medium.en'
  | 'large-v3-turbo';

export type HotkeyOption = 'shift_l' | 'alt_l' | 'ctrl_r';

export const MODEL_OPTIONS: { value: ModelOption; label: string; size: string }[] = [
  { value: 'tiny.en', label: 'Tiny (English)', size: '~75 MB' },
  { value: 'base.en', label: 'Base (English)', size: '~150 MB' },
  { value: 'small.en', label: 'Small (English)', size: '~500 MB' },
  { value: 'medium.en', label: 'Medium (English)', size: '~1.5 GB' },
  { value: 'large-v3-turbo', label: 'Large Turbo (Recommended)', size: '~3 GB' },
];

export const HOTKEY_OPTIONS: { value: HotkeyOption; label: string }[] = [
  { value: 'shift_l', label: 'Shift + Space' },
  { value: 'alt_l', label: 'Option + Space' },
  { value: 'ctrl_r', label: 'Control + Space' },
];

export const DEFAULT_SETTINGS: Settings = {
  model: 'large-v3-turbo',
  hotkey: 'shift_l',
  language: 'en',
  autoPaste: false,
};

const STORAGE_KEY = 'dictation-settings';

export function loadSettings(): Settings {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return { ...DEFAULT_SETTINGS, ...JSON.parse(stored) };
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
