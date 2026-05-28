import { describe, it, expect, beforeEach } from 'vitest';
import { loadSettings, DEFAULT_SETTINGS, isLikelyBluetoothMicrophone } from './settings';

beforeEach(() => {
  const store = new Map<string, string>();
  const localStorageMock = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (key: string) => store.get(key) ?? null,
    key: (index: number) => Array.from(store.keys())[index] ?? null,
    removeItem: (key: string) => { store.delete(key); },
    setItem: (key: string, value: string) => { store.set(key, value); },
  } satisfies Storage;
  Object.defineProperty(globalThis, 'localStorage', { value: localStorageMock, configurable: true });
  Object.defineProperty(window, 'localStorage', { value: localStorageMock, configurable: true });
});

describe('loadSettings', () => {
  it('returns defaults when localStorage is empty', () => {
    const settings = loadSettings();
    expect(settings).toEqual(DEFAULT_SETTINGS);
  });

  it('returns defaults when localStorage has invalid JSON', () => {
    localStorage.setItem('dictation-settings', 'not json{{{');
    const settings = loadSettings();
    expect(settings).toEqual(DEFAULT_SETTINGS);
  });

  it('preserves valid stored settings', () => {
    const stored = { ...DEFAULT_SETTINGS, language: 'es', autoPaste: true };
    localStorage.setItem('dictation-settings', JSON.stringify(stored));
    const settings = loadSettings();
    expect(settings.language).toBe('es');
    expect(settings.autoPaste).toBe(true);
  });

  it('fills missing fields from defaults', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'double_tap',
    }));
    const settings = loadSettings();
    expect(settings.autoPaste).toBe(DEFAULT_SETTINGS.autoPaste);
    expect(settings.recordingMode).toBe('double_tap');
  });

  it('migrates legacy "hotkey" recordingMode to "hold_down"', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hotkey',
      hotkey: 'ctrl+shift+space',
    }));
    const settings = loadSettings();
    expect(settings.recordingMode).toBe('hold_down');
    expect((settings as unknown as Record<string, unknown>).hotkey).toBeUndefined();
  });

  it('migrates missing recordingMode to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'tiny.en',
      doubleTapKey: 'alt_l',
      language: 'en',
    }));
    const settings = loadSettings();
    expect(settings.recordingMode).toBe(DEFAULT_SETTINGS.recordingMode);
  });

  it('migrates moonshine model to base.en', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'moonshine-tiny',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe('base.en');
  });

  it('migrates moonshine-base model to base.en', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'moonshine-base',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe('base.en');
  });

  it('resets unknown model to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'nonexistent-model',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe('base.en');
  });

  it('preserves valid recordingMode values', () => {
    for (const mode of ['hold_down', 'double_tap'] as const) {
      localStorage.setItem('dictation-settings', JSON.stringify({
        ...DEFAULT_SETTINGS,
        recordingMode: mode,
      }));
      const settings = loadSettings();
      expect(settings.recordingMode).toBe(mode);
    }
  });
});

describe('isLikelyBluetoothMicrophone', () => {
  it('flags common Bluetooth headset microphones', () => {
    expect(isLikelyBluetoothMicrophone("George's AirPods Pro")).toBe(true);
    expect(isLikelyBluetoothMicrophone('Bose QC Headset')).toBe(true);
    expect(isLikelyBluetoothMicrophone('Jabra Hands-Free Audio')).toBe(true);
  });

  it('does not flag built-in or USB-style microphones', () => {
    expect(isLikelyBluetoothMicrophone('MacBook Pro Microphone')).toBe(false);
    expect(isLikelyBluetoothMicrophone('USB Audio Device')).toBe(false);
  });
});
