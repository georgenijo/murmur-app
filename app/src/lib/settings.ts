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
  { value: 'both', label: 'Both' },
];

import type { SelectGroup } from '../components/ui/Select';

export const LANGUAGE_OPTIONS: SelectGroup[] = [
  {
    label: 'Common',
    options: [
      { value: 'auto', label: 'Auto-detect' },
      { value: 'en', label: 'English' },
      { value: 'zh', label: 'Chinese' },
      { value: 'es', label: 'Spanish' },
      { value: 'hi', label: 'Hindi' },
      { value: 'ar', label: 'Arabic' },
      { value: 'fr', label: 'French' },
      { value: 'de', label: 'German' },
      { value: 'ja', label: 'Japanese' },
      { value: 'ko', label: 'Korean' },
      { value: 'pt', label: 'Portuguese' },
      { value: 'ru', label: 'Russian' },
    ],
  },
  {
    label: 'All Languages',
    options: [
      { value: 'af', label: 'Afrikaans' },
      { value: 'am', label: 'Amharic' },
      { value: 'az', label: 'Azerbaijani' },
      { value: 'ba', label: 'Bashkir' },
      { value: 'be', label: 'Belarusian' },
      { value: 'bn', label: 'Bengali' },
      { value: 'bo', label: 'Tibetan' },
      { value: 'br', label: 'Breton' },
      { value: 'bs', label: 'Bosnian' },
      { value: 'ca', label: 'Catalan' },
      { value: 'cs', label: 'Czech' },
      { value: 'cy', label: 'Welsh' },
      { value: 'da', label: 'Danish' },
      { value: 'el', label: 'Greek' },
      { value: 'et', label: 'Estonian' },
      { value: 'eu', label: 'Basque' },
      { value: 'fa', label: 'Persian' },
      { value: 'fi', label: 'Finnish' },
      { value: 'fo', label: 'Faroese' },
      { value: 'gl', label: 'Galician' },
      { value: 'gu', label: 'Gujarati' },
      { value: 'ha', label: 'Hausa' },
      { value: 'haw', label: 'Hawaiian' },
      { value: 'he', label: 'Hebrew' },
      { value: 'hr', label: 'Croatian' },
      { value: 'ht', label: 'Haitian Creole' },
      { value: 'hu', label: 'Hungarian' },
      { value: 'hy', label: 'Armenian' },
      { value: 'id', label: 'Indonesian' },
      { value: 'is', label: 'Icelandic' },
      { value: 'it', label: 'Italian' },
      { value: 'jw', label: 'Javanese' },
      { value: 'ka', label: 'Georgian' },
      { value: 'kk', label: 'Kazakh' },
      { value: 'km', label: 'Khmer' },
      { value: 'kn', label: 'Kannada' },
      { value: 'la', label: 'Latin' },
      { value: 'lb', label: 'Luxembourgish' },
      { value: 'ln', label: 'Lingala' },
      { value: 'lo', label: 'Lao' },
      { value: 'lt', label: 'Lithuanian' },
      { value: 'lv', label: 'Latvian' },
      { value: 'mg', label: 'Malagasy' },
      { value: 'mi', label: 'Maori' },
      { value: 'mk', label: 'Macedonian' },
      { value: 'ml', label: 'Malayalam' },
      { value: 'mn', label: 'Mongolian' },
      { value: 'mr', label: 'Marathi' },
      { value: 'ms', label: 'Malay' },
      { value: 'mt', label: 'Maltese' },
      { value: 'my', label: 'Myanmar' },
      { value: 'ne', label: 'Nepali' },
      { value: 'nl', label: 'Dutch' },
      { value: 'nn', label: 'Nynorsk' },
      { value: 'no', label: 'Norwegian' },
      { value: 'oc', label: 'Occitan' },
      { value: 'pa', label: 'Punjabi' },
      { value: 'pl', label: 'Polish' },
      { value: 'ps', label: 'Pashto' },
      { value: 'ro', label: 'Romanian' },
      { value: 'sa', label: 'Sanskrit' },
      { value: 'sd', label: 'Sindhi' },
      { value: 'si', label: 'Sinhala' },
      { value: 'sk', label: 'Slovak' },
      { value: 'sl', label: 'Slovenian' },
      { value: 'sn', label: 'Shona' },
      { value: 'so', label: 'Somali' },
      { value: 'sq', label: 'Albanian' },
      { value: 'sr', label: 'Serbian' },
      { value: 'su', label: 'Sundanese' },
      { value: 'sv', label: 'Swedish' },
      { value: 'sw', label: 'Swahili' },
      { value: 'ta', label: 'Tamil' },
      { value: 'te', label: 'Telugu' },
      { value: 'tg', label: 'Tajik' },
      { value: 'th', label: 'Thai' },
      { value: 'tk', label: 'Turkmen' },
      { value: 'tl', label: 'Tagalog' },
      { value: 'tr', label: 'Turkish' },
      { value: 'tt', label: 'Tatar' },
      { value: 'uk', label: 'Ukrainian' },
      { value: 'ur', label: 'Urdu' },
      { value: 'uz', label: 'Uzbek' },
      { value: 'vi', label: 'Vietnamese' },
      { value: 'yi', label: 'Yiddish' },
      { value: 'yo', label: 'Yoruba' },
    ],
  },
];

export function isEnglishOnlyModel(model: ModelOption): boolean {
  return model.endsWith('.en') || model.startsWith('moonshine-');
}

export function isMultilingualLanguage(language: string): boolean {
  return language !== 'en';
}

export const DEFAULT_SETTINGS: Settings = {
  model: 'moonshine-tiny',
  doubleTapKey: 'shift_l',
  language: 'en',
  autoPaste: false,
  autoPasteDelayMs: 50,
  recordingMode: 'hold_down',
  microphone: 'system_default',
  launchAtLogin: false,
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
