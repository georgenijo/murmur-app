import { flushSourceState } from './webkitLifecycle';
import { describe, it, expect, beforeEach, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => false),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
  isTauri: () => mocks.isTauri(),
}));

import {
  hydrateSettingsFromDisk,
  ONBOARDING_KEY,
  persistOnboardingComplete,
  parseSettingsBlob,
  loadSettings,
  saveSettings,
  DEFAULT_SETTINGS,
  DICTATION_KEY_OPTIONS,
  dictationKeyLabel,
  LEGACY_OVERLAY_OFFSET_KEY,
  microphoneDeviceNameArg,
  ownSettingsMigrations,
  STORAGE_KEY,
  recordingShortcutHint,
  WRITING_STYLE_OPTIONS,
  WRITING_STYLE_VALUES,
} from './settings';

beforeEach(() => {
  localStorage.clear();
  ownSettingsMigrations();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  // Default to "not running under Tauri" so the existing localStorage-only
  // expectations below are unaffected by the durable store.
  mocks.isTauri.mockReturnValue(false);
});

describe('WRITING_STYLE_VALUES', () => {
  it('matches the concrete writing-style values, excluding the inherit sentinel', () => {
    expect(WRITING_STYLE_VALUES).toEqual([
      'conversational',
      'polished',
      'code_technical',
      'verbatim',
      'notes',
    ]);
  });

  it('stays in sync with every non-inherit option in WRITING_STYLE_OPTIONS', () => {
    const expected = WRITING_STYLE_OPTIONS
      .map(option => option.value)
      .filter(value => value !== 'inherit');
    expect(WRITING_STYLE_VALUES).toEqual(expected);
  });
});

describe('dictation keys', () => {
  it('keeps the three existing modifiers and F1 through F20 in one catalog', () => {
    expect(DICTATION_KEY_OPTIONS.map((option) => option.value)).toEqual([
      'shift_l', 'alt_l', 'ctrl_r',
      ...Array.from({ length: 20 }, (_, index) => `f${index + 1}`),
    ]);
    expect(DICTATION_KEY_OPTIONS.map((option) => option.label)).toEqual([
      '⇧ Left Shift', '⌥ Left Option', '⌃ Right Control',
      ...Array.from({ length: 20 }, (_, index) => `F${index + 1}`),
    ]);
  });

  it('reloads function keys with the same labels and recording hints', () => {
    saveSettings({ ...DEFAULT_SETTINGS, doubleTapKey: 'f20', recordingMode: 'both' });
    const reloaded = loadSettings();

    expect(reloaded.doubleTapKey).toBe('f20');
    expect(dictationKeyLabel(reloaded.doubleTapKey)).toBe('F20');
    expect(recordingShortcutHint(reloaded.recordingMode, reloaded.doubleTapKey))
      .toBe('Hold or double-tap F20');
  });


});

describe('Rust-owned settings snapshots', () => {
  it('reads canonical state without publishing migrations', () => {
    const blob = JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7, language: 'es' });
    localStorage.setItem(STORAGE_KEY, blob);
    expect(loadSettings().language).toBe('es');
    expect(localStorage.getItem(STORAGE_KEY)).toBe(blob);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
  it('returns defaults for missing or unreadable presentation snapshots', () => {
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
    localStorage.setItem(STORAGE_KEY, '{bad');
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
  });
});

describe('durable settings store', () => {
  const STORAGE_KEY = 'dictation-settings';

  beforeEach(() => {
    mocks.isTauri.mockReturnValue(true);
  });

  it('hydrates the authoritative onboarding gate before startup can continue', async () => {
    let release!: (blob: string) => void;
    mocks.invoke.mockImplementation(() => new Promise<string>((resolve) => { release = resolve; }));
    localStorage.setItem(ONBOARDING_KEY, 'true');
    let rendered = false;
    const startup = hydrateSettingsFromDisk().then(() => { rendered = true; });
    expect(rendered).toBe(false);
    release(JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7, launchOnboardingComplete: false }));
    await startup;
    expect(rendered).toBe(true);
    expect(localStorage.getItem(ONBOARDING_KEY)).toBeNull();
    expect(loadSettings()).not.toHaveProperty('launchOnboardingComplete');
  });

  it('restores a completed onboarding gate after cache eviction and keeps companions read-only', async () => {
    mocks.invoke.mockResolvedValue(JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7, launchOnboardingComplete: true }));
    await hydrateSettingsFromDisk();
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
    ownSettingsMigrations(false);
    mocks.invoke.mockResolvedValue(JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7, launchOnboardingComplete: false }));
    await hydrateSettingsFromDisk();
    expect(localStorage.getItem(ONBOARDING_KEY)).toBe('true');
  });

  it('mirrors completion and reset without replacing capture settings with defaults', async () => {
    const original = { ...DEFAULT_SETTINGS, language: 'ja', settingsVersion: 7 };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(original));
    persistOnboardingComplete(true);
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({ ...original, launchOnboardingComplete: true });
    persistOnboardingComplete(true);
    await flushSourceState();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    persistOnboardingComplete(false);
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toEqual({ ...original, launchOnboardingComplete: false });
    await flushSourceState();
    expect(mocks.invoke).toHaveBeenLastCalledWith('save_settings_blob', {
      blob: JSON.stringify({ ...original, launchOnboardingComplete: false }),
    });
  });

  it('seeds localStorage from the blob the backend returns', async () => {
    const blob = JSON.stringify({ ...DEFAULT_SETTINGS, language: 'es', settingsVersion: 1 });
    mocks.invoke.mockResolvedValue(blob);

    await hydrateSettingsFromDisk();

    expect(mocks.invoke).toHaveBeenCalledWith('load_settings_blob');
    expect(localStorage.getItem(STORAGE_KEY)).toBe(blob);
    expect(loadSettings().language).toBe('es');
  });

  it('overwrites a stale localStorage cache — disk is authoritative', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, language: 'fr' }));
    mocks.invoke.mockResolvedValue(JSON.stringify({ ...DEFAULT_SETTINGS, language: 'de' }));

    await hydrateSettingsFromDisk();

    expect(loadSettings().language).toBe('de');
  });

  it('migrates an existing localStorage blob to disk when nothing is stored yet', async () => {
    const cached = JSON.stringify({ ...DEFAULT_SETTINGS, language: 'it', settingsVersion: 1 });
    localStorage.setItem(STORAGE_KEY, cached);
    mocks.invoke.mockImplementation(async (command: string) =>
      command === 'load_settings_blob' ? null : undefined);

    await hydrateSettingsFromDisk();

    expect(mocks.invoke).toHaveBeenCalledWith('save_settings_blob', { blob: cached });
    expect(localStorage.getItem(STORAGE_KEY)).toBe(cached);
  });

  it('lets only the main window seed the defaults on a first run', async () => {
    mocks.invoke.mockResolvedValue(null);
    ownSettingsMigrations(false);

    await hydrateSettingsFromDisk();

    await flushSourceState();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();

    ownSettingsMigrations();
    await hydrateSettingsFromDisk();

    const seeded = localStorage.getItem(STORAGE_KEY)!;
    expect(mocks.invoke).toHaveBeenCalledWith('save_settings_blob', { blob: seeded });
    expect(JSON.parse(seeded)).toMatchObject({ settingsVersion: 7, overlayLiveWords: true });
  });

  it('leaves localStorage untouched when the backend rejects', async () => {
    const cached = JSON.stringify({ ...DEFAULT_SETTINGS, language: 'ja' });
    localStorage.setItem(STORAGE_KEY, cached);
    mocks.invoke.mockRejectedValue(new Error('store unavailable'));

    await expect(hydrateSettingsFromDisk()).resolves.toBeUndefined();

    expect(localStorage.getItem(STORAGE_KEY)).toBe(cached);
  });

  it('does not touch the backend outside Tauri', async () => {
    mocks.isTauri.mockReturnValue(false);

    await hydrateSettingsFromDisk();
    saveSettings(DEFAULT_SETTINGS);

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(localStorage.getItem(STORAGE_KEY)).not.toBeNull();
  });

  it('writes localStorage synchronously and mirrors the same blob to disk', async () => {
    saveSettings({ ...DEFAULT_SETTINGS, language: 'ko' });

    const written = localStorage.getItem(STORAGE_KEY);
    expect(written).not.toBeNull();
    expect(JSON.parse(written ?? '{}')).toMatchObject({ language: 'ko', settingsVersion: 7 });
    await flushSourceState();
    expect(mocks.invoke).toHaveBeenCalledWith('save_settings_blob', { blob: written });
  });

  it('still persists to localStorage when the disk mirror fails', () => {
    mocks.invoke.mockRejectedValue(new Error('disk full'));

    saveSettings({ ...DEFAULT_SETTINGS, language: 'ru' });

    expect(loadSettings().language).toBe('ru');
  });
});

describe('microphoneDeviceNameArg', () => {
  it('drops the system-default sentinel so Rust picks the default input device', () => {
    // `DEFAULT_SETTINGS.microphone` is a sentinel, not a CoreAudio UID.
    // Forwarding it verbatim fails the device lookup with NoInputDevice.
    expect(DEFAULT_SETTINGS.microphone).toBe('system_default');
    expect(microphoneDeviceNameArg(DEFAULT_SETTINGS.microphone)).toBeNull();
    expect(microphoneDeviceNameArg('system_default')).toBeNull();
  });

  it('drops empty and absent values', () => {
    expect(microphoneDeviceNameArg('')).toBeNull();
    expect(microphoneDeviceNameArg(null)).toBeNull();
    expect(microphoneDeviceNameArg(undefined)).toBeNull();
  });

  it('passes an explicitly chosen device identifier through unchanged', () => {
    expect(microphoneDeviceNameArg('AppleUSBAudioEngine:Shure:MV7:1234:1'))
      .toBe('AppleUSBAudioEngine:Shure:MV7:1234:1');
  });
});

describe('read-only durable settings snapshots', () => {
  it('decodes a Rust-canonical snapshot without overwriting cache or disk', () => {
    mocks.isTauri.mockReturnValue(true);
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, overlayLiveWords: false }));
    localStorage.setItem(LEGACY_OVERLAY_OFFSET_KEY, '48');
    const cached = localStorage.getItem(STORAGE_KEY);
    const parsed = parseSettingsBlob(JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7,
      overlayVerticalOffset: 0, overlayPlacement: 'floating_pill', overlayLiveWords: true }));
    expect(parsed.overlayVerticalOffset).toBe(0);
    expect(parsed.overlayPlacement).toBe('floating_pill');
    expect('overlayProcessingStyle' in parsed).toBe(false);
    expect(parsed.overlayLiveWords).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBe(cached);
    expect(localStorage.getItem(LEGACY_OVERLAY_OFFSET_KEY)).toBe('48');
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
