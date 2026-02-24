import { describe, it, expect, beforeEach } from 'vitest';
import { loadSettings, DEFAULT_SETTINGS } from './settings';

beforeEach(() => {
  localStorage.clear();
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
