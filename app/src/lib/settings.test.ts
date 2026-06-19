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

  it('migrates moonshine model to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'moonshine-tiny',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe(DEFAULT_SETTINGS.model);
  });

  it('migrates moonshine-base model to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'moonshine-base',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe(DEFAULT_SETTINGS.model);
  });

  it('resets unknown model to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'nonexistent-model',
    }));
    const settings = loadSettings();
    expect(settings.model).toBe(DEFAULT_SETTINGS.model);
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

  it('defaults cleanupEnabled when absent from stored settings', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.cleanupEnabled).toBe(DEFAULT_SETTINGS.cleanupEnabled);
  });

  it('coerces non-boolean cleanupEnabled to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      cleanupEnabled: 'yes',
    }));
    const settings = loadSettings();
    expect(settings.cleanupEnabled).toBe(DEFAULT_SETTINGS.cleanupEnabled);
  });

  it('preserves an explicit cleanupEnabled value', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      cleanupEnabled: true,
    }));
    const settings = loadSettings();
    expect(settings.cleanupEnabled).toBe(true);
  });

  it('defaults language to auto when absent from stored settings', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.language).toBe(DEFAULT_SETTINGS.language);
    expect(settings.language).toBe('auto');
  });

  it('coerces an unknown language code to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      language: 'klingon',
    }));
    const settings = loadSettings();
    expect(settings.language).toBe(DEFAULT_SETTINGS.language);
  });

  it('coerces a non-string language to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      language: 42,
    }));
    const settings = loadSettings();
    expect(settings.language).toBe(DEFAULT_SETTINGS.language);
  });

  it('preserves a valid non-default language code', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      language: 'nl',
    }));
    const settings = loadSettings();
    expect(settings.language).toBe('nl');
  });

  it('defaults codeVocabEnabled and codeVocabFolder when absent', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.codeVocabEnabled).toBe(DEFAULT_SETTINGS.codeVocabEnabled);
    expect(settings.codeVocabFolder).toBe(DEFAULT_SETTINGS.codeVocabFolder);
  });

  it('coerces non-boolean codeVocabEnabled to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabEnabled: 'yes',
    }));
    const settings = loadSettings();
    expect(settings.codeVocabEnabled).toBe(DEFAULT_SETTINGS.codeVocabEnabled);
  });

  it('coerces non-string codeVocabFolder to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabFolder: 123,
    }));
    const settings = loadSettings();
    expect(settings.codeVocabFolder).toBe(DEFAULT_SETTINGS.codeVocabFolder);
  });

  it('preserves explicit codeVocab settings', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabEnabled: true,
      codeVocabFolder: '/Users/me/project',
    }));
    const settings = loadSettings();
    expect(settings.codeVocabEnabled).toBe(true);
    expect(settings.codeVocabFolder).toBe('/Users/me/project');
  });

  it('defaults livePreviewEnabled to false (off) when absent', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.livePreviewEnabled).toBe(DEFAULT_SETTINGS.livePreviewEnabled);
    expect(settings.livePreviewEnabled).toBe(false);
  });

  it('coerces non-boolean livePreviewEnabled to default', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      livePreviewEnabled: 'yes',
    }));
    const settings = loadSettings();
    expect(settings.livePreviewEnabled).toBe(DEFAULT_SETTINGS.livePreviewEnabled);
  });

  it('preserves an explicit livePreviewEnabled value', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      livePreviewEnabled: true,
    }));
    const settings = loadSettings();
    expect(settings.livePreviewEnabled).toBe(true);
  });
});
