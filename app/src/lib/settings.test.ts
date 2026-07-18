import { describe, it, expect, beforeEach } from 'vitest';
import {
  loadSettings,
  DEFAULT_SETTINGS,
  defaultModelForPlatform,
  modelOptionsForPlatform,
} from './settings';

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

  it('uses Core ML for new macOS installs and CPU Parakeet elsewhere', () => {
    expect(defaultModelForPlatform('MacIntel')).toBe('parakeet-tdt-0.6b-v3-coreml');
    expect(defaultModelForPlatform('Linux x86_64')).toBe('parakeet-tdt-0.6b-v2-fp16');
    expect(defaultModelForPlatform('Win32')).toBe('parakeet-tdt-0.6b-v2-fp16');
  });

  it('hides the Core ML option outside macOS', () => {
    expect(modelOptionsForPlatform('MacIntel').some((model) => model.backend === 'coreml')).toBe(true);
    expect(modelOptionsForPlatform('Linux x86_64').some((model) => model.backend === 'coreml')).toBe(false);
  });

  it('preserves an existing CPU Parakeet selection', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      model: 'parakeet-tdt-0.6b-v2-fp16',
    }));
    expect(loadSettings().model).toBe('parakeet-tdt-0.6b-v2-fp16');
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

  it('opts pre-feature settings into correction (correctionEnabled defaults on)', () => {
    // An older blob predating the correction feature should migrate to ON.
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.correctionEnabled).toBe(true);
    expect(settings.correctionFuzzy).toBe(true);
  });

  it('coerces non-boolean correction toggles to defaults', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      correctionEnabled: 'yes',
      correctionFuzzy: 1,
    }));
    const settings = loadSettings();
    expect(settings.correctionEnabled).toBe(DEFAULT_SETTINGS.correctionEnabled);
    expect(settings.correctionFuzzy).toBe(DEFAULT_SETTINGS.correctionFuzzy);
  });

  it('preserves explicit correction settings', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      correctionEnabled: false,
      correctionFuzzy: false,
    }));
    const settings = loadSettings();
    expect(settings.correctionEnabled).toBe(false);
    expect(settings.correctionFuzzy).toBe(false);
  });

  it('defaults codeVocabLastScan to null when absent', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en',
      doubleTapKey: 'shift_l',
      language: 'en',
      recordingMode: 'hold_down',
    }));
    const settings = loadSettings();
    expect(settings.codeVocabLastScan).toBeNull();
  });

  it('sanitizes a valid codeVocabLastScan with ranked terms', () => {
    const scan = {
      files: 87,
      skipped: 6,
      terms: 268,
      bytes: 2_400_000,
      capped: false,
      ms: 610,
      sampleTerms: ['useRecordingState', 'TranscriptionBackend'],
      rankedTerms: [
        { term: 'useRecordingState', freq: 42 },
        { term: 'TranscriptionBackend', freq: 31 },
      ],
      whisperCount: 2,
      adopted: true,
    };
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabLastScan: scan,
    }));
    const settings = loadSettings();
    expect(settings.codeVocabLastScan).toEqual(scan);
  });

  it('defaults rankedTerms/whisperCount on a pre-feature scan blob', () => {
    // A scan summary persisted before this feature lacks rankedTerms/whisperCount.
    const legacyScan = {
      files: 10,
      skipped: 1,
      terms: 5,
      bytes: 1000,
      capped: false,
      ms: 100,
      sampleTerms: ['foo', 'bar'],
    };
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabLastScan: legacyScan,
    }));
    const settings = loadSettings();
    expect(settings.codeVocabLastScan).not.toBeNull();
    expect(settings.codeVocabLastScan!.rankedTerms).toEqual([]);
    expect(settings.codeVocabLastScan!.whisperCount).toBe(0);
    expect(settings.codeVocabLastScan!.sampleTerms).toEqual(['foo', 'bar']);
    expect(settings.codeVocabLastScan!.adopted).toBe(true);
  });

  it('drops malformed ranked-term entries and clamps the list to 500', () => {
    const ranked = [
      { term: 'good', freq: 9 },
      { term: 'noFreq' }, // missing freq -> dropped
      { freq: 3 }, // missing term -> dropped
      { term: '', freq: 1 }, // empty term -> dropped
      { term: 'nanFreq', freq: Number.NaN }, // non-finite -> dropped
      ...Array.from({ length: 600 }, (_, i) => ({ term: `t${i}`, freq: 1 })),
    ];
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabLastScan: {
        ...DEFAULT_SETTINGS,
        files: 1,
        skipped: 0,
        terms: 601,
        bytes: 1,
        capped: true,
        ms: 1,
        sampleTerms: ['good'],
        rankedTerms: ranked,
        whisperCount: 96,
      },
    }));
    const settings = loadSettings();
    const kept = settings.codeVocabLastScan!.rankedTerms;
    expect(kept.length).toBe(500);
    expect(kept[0]).toEqual({ term: 'good', freq: 9 });
    // whisperCount stays valid since 96 <= 500 kept entries.
    expect(settings.codeVocabLastScan!.whisperCount).toBe(96);
  });

  it('clamps whisperCount to the number of ranked terms kept', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      codeVocabLastScan: {
        files: 1,
        skipped: 0,
        terms: 2,
        bytes: 1,
        capped: false,
        ms: 1,
        sampleTerms: ['a'],
        rankedTerms: [{ term: 'a', freq: 2 }],
        whisperCount: 96, // more than the single kept term
      },
    }));
    const settings = loadSettings();
    expect(settings.codeVocabLastScan!.whisperCount).toBe(1);
  });
});
