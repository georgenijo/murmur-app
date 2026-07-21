import { describe, it, expect, beforeEach } from 'vitest';
import {
  loadSettings,
  saveSettings,
  DEFAULT_SETTINGS,
  defaultModelForPlatform,
  modelOptionsForPlatform,
} from './settings';

beforeEach(() => {
  localStorage.clear();
});

describe('loadSettings', () => {
  it('round-trips every stored field and value through the unified Settings UI schema', () => {
    const stored = {
      ...DEFAULT_SETTINGS,
      model: 'tiny.en' as const,
      doubleTapKey: 'alt_l' as const,
      transformHoldKey: 'alt_r' as const,
      language: 'es',
      autoPaste: true,
      autoPasteDelayMs: 230,
      recordingMode: 'both' as const,
      hotkeyMissFeedback: true,
      microphone: 'Studio Mic',
      launchAtLogin: true,
      vadSensitivity: 75,
      idleTimeoutMinutes: 15,
      customVocabulary: 'Murmur',
      vocabularyEntries: [{ id: 'murmur', written: 'Murmur', aliases: ['murmur app'], enabled: true, scope: { kind: 'global' as const } }],
      disabled: true,
      smartPunctuation: false,
      saveTranscript: true,
      saveAudio: true,
      outputDir: '/tmp/murmur-output',
      appProfiles: [{
        bundleId: 'com.apple.Terminal',
        label: 'Terminal',
        autoPasteOverride: false,
        cleanupOverride: true,
        smartFormattingOverride: false,
        cliFormattingOverride: true,
        writingStyle: 'code_technical' as const,
        ideContextEnabled: true,
        ideProjectRoots: ['/tmp/project'],
      }],
      voiceCommandsEnabled: true,
      voiceCommands: [{ phrase: 'standup', replacement: 'Yesterday:\nToday:' }],
      cleanupEnabled: true,
      smartFormattingEnabled: true,
      cleanupRemoveFiller: false,
      cleanupCapitalize: false,
      codeVocabEnabled: true,
      codeVocabFolder: '/tmp/project',
      codeVocabLastScan: {
        files: 2, skipped: 1, terms: 3, bytes: 44, capped: false, ms: 5,
        sampleTerms: ['useEffect'], rankedTerms: [{ term: 'useEffect', freq: 2 }],
        whisperCount: 1, adopted: true,
      },
      correctionEnabled: false,
      correctionFuzzy: false,
    };

    saveSettings(stored);
    const loaded = loadSettings();

    expect(Object.keys(loaded).sort()).toEqual(Object.keys(DEFAULT_SETTINGS).sort());
    expect(loaded).toEqual(stored);
  });

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

  it('migrates and validates per-app smart and CLI formatting overrides', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      appProfiles: [
        {
          bundleId: 'com.apple.Terminal',
          label: 'Terminal',
          autoPasteOverride: null,
          cleanupOverride: false,
          smartFormattingOverride: true,
          cliFormattingOverride: true,
          writingStyle: 'polished',
        },
        {
          bundleId: 'com.apple.mail',
          label: 'Mail',
          autoPasteOverride: null,
          cleanupOverride: null,
          smartFormattingOverride: 'yes',
          cliFormattingOverride: 'yes',
          writingStyle: 'automatic',
        },
        {
          bundleId: 'com.apple.TextEdit',
          label: 'Legacy profile',
          autoPasteOverride: false,
          cleanupOverride: null,
        },
      ],
    }));

    const [terminal, mail, legacy] = loadSettings().appProfiles;
    expect(terminal.smartFormattingOverride).toBe(true);
    expect(terminal.cliFormattingOverride).toBe(true);
    expect(terminal.writingStyle).toBe('polished');
    expect(mail.smartFormattingOverride).toBeNull();
    expect(mail.cliFormattingOverride).toBeNull();
    expect(mail.writingStyle).toBeNull();
    expect(legacy.smartFormattingOverride).toBeNull();
    expect(legacy.cliFormattingOverride).toBeNull();
    expect(legacy.writingStyle).toBeNull();
  });

  it('migrates IDE context as explicit opt-in with bounded persisted roots only', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      appProfiles: [
        {
          bundleId: 'com.example.Editor',
          label: 'Editor',
          ideContextEnabled: true,
          ideProjectRoots: [
            ' /project/one ',
            '/project/one',
            `/${'x'.repeat(4096)}`,
            '/project/two',
            '/project/three',
            '/project/four',
            '/project/five',
            42,
          ],
          persistedSymbols: ['must-not-survive'],
          scanResults: { filename: 'must-not-survive.rs' },
        },
        {
          bundleId: 'com.example.Legacy',
          label: 'Legacy',
        },
        {
          bundleId: 'com.example.Malformed',
          label: 'Malformed',
          ideContextEnabled: 'yes',
          ideProjectRoots: '/project/not-an-array',
        },
      ],
    }));

    const [editor, legacy, malformed] = loadSettings().appProfiles;
    expect(editor.ideContextEnabled).toBe(true);
    expect(editor.ideProjectRoots).toEqual([
      '/project/one',
      '/project/two',
      '/project/three',
      '/project/four',
    ]);
    expect(editor).not.toHaveProperty('persistedSymbols');
    expect(editor).not.toHaveProperty('scanResults');
    expect(legacy.ideContextEnabled).toBe(false);
    expect(legacy.ideProjectRoots).toEqual([]);
    expect(malformed.ideContextEnabled).toBe(false);
    expect(malformed.ideProjectRoots).toEqual([]);
  });

  it('keeps smart formatting opt-in across settings migrations', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      smartFormattingEnabled: true,
    }));
    expect(loadSettings().smartFormattingEnabled).toBe(true);

    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      smartFormattingEnabled: 'yes',
    }));
    expect(loadSettings().smartFormattingEnabled).toBe(false);

    const legacy = { ...DEFAULT_SETTINGS } as Record<string, unknown>;
    delete legacy.smartFormattingEnabled;
    localStorage.setItem('dictation-settings', JSON.stringify(legacy));
    expect(loadSettings().smartFormattingEnabled).toBe(false);
  });

  it('defaults transformHoldKey to disabled (null) when absent from a pre-feature blob', () => {
    const legacy = { ...DEFAULT_SETTINGS } as Record<string, unknown>;
    delete legacy.transformHoldKey;
    localStorage.setItem('dictation-settings', JSON.stringify(legacy));
    expect(loadSettings().transformHoldKey).toBeNull();
  });

  it('preserves a valid stored transformHoldKey', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      transformHoldKey: 'ctrl_l',
    }));
    expect(loadSettings().transformHoldKey).toBe('ctrl_l');
  });

  it('coerces an unrecognised or malformed transformHoldKey back to disabled', () => {
    for (const bad of ['shift_l', 'not_a_key', 42, {}, true]) {
      localStorage.setItem('dictation-settings', JSON.stringify({
        ...DEFAULT_SETTINGS,
        transformHoldKey: bad,
      }));
      expect(loadSettings().transformHoldKey).toBeNull();
    }
  });

  it('explicit null transformHoldKey stays disabled', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      transformHoldKey: null,
    }));
    expect(loadSettings().transformHoldKey).toBeNull();
  });

  it('preserves legacy custom Voice Command pairs for one-time Rust migration', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      voiceCommandsEnabled: true,
      voiceCommands: [
        { phrase: ' insert standup ', replacement: 'Yesterday:\n- done\nToday:\n- ship' },
        { phrase: 'remove phrase', replacement: '' },
        { phrase: '', replacement: 'ignored' },
      ],
    }));
    const settings = loadSettings();
    expect(settings.voiceCommandsEnabled).toBe(true);
    expect(settings.voiceCommands).toEqual([
      { phrase: 'insert standup', replacement: 'Yesterday:\n- done\nToday:\n- ship' },
      { phrase: 'remove phrase', replacement: '' },
    ]);
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
    expect(settings.hotkeyMissFeedback).toBe(false);
  });

  it('validates the opt-in hotkey timing feedback setting', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      hotkeyMissFeedback: true,
    }));
    expect(loadSettings().hotkeyMissFeedback).toBe(true);

    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      hotkeyMissFeedback: 'yes',
    }));
    expect(loadSettings().hotkeyMissFeedback).toBe(false);
  });

  it('removes the retired live transcript preview setting', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      liveTranscriptPreview: true,
    }));
    expect(loadSettings()).not.toHaveProperty('liveTranscriptPreview');
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

  it('migrates legacy custom vocabulary into enabled global entries', () => {
    const legacy = { ...DEFAULT_SETTINGS } as Record<string, unknown>;
    delete legacy.vocabularyEntries;
    legacy.customVocabulary = 'Tauri, API Gateway\nMünchen';
    localStorage.setItem('dictation-settings', JSON.stringify(legacy));

    const settings = loadSettings();
    expect(settings.vocabularyEntries).toEqual([
      { id: 'legacy-0', written: 'Tauri', aliases: [], enabled: true, scope: { kind: 'global' } },
      { id: 'legacy-1', written: 'API Gateway', aliases: [], enabled: true, scope: { kind: 'global' } },
      { id: 'legacy-2', written: 'München', aliases: [], enabled: true, scope: { kind: 'global' } },
    ]);
    expect(settings.customVocabulary).toBe('Tauri, API Gateway, München');
  });

  it('sanitizes structured vocabulary and derives the legacy prompt mirror', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      customVocabulary: 'stale value',
      vocabularyEntries: [
        {
          id: 'tauri',
          written: ' Tauri ',
          aliases: [' Tori ', 'tori', ' Tory '],
          enabled: true,
          scope: { kind: 'global' },
        },
        {
          id: 'disabled',
          written: 'Hidden',
          aliases: ['heard'],
          enabled: false,
          scope: { kind: 'global' },
        },
        { id: 'bad', written: '', aliases: [], enabled: true },
      ],
    }));

    const settings = loadSettings();
    expect(settings.vocabularyEntries).toEqual([
      { id: 'tauri', written: 'Tauri', aliases: ['Tori', 'Tory'], enabled: true, scope: { kind: 'global' } },
      { id: 'disabled', written: 'Hidden', aliases: ['heard'], enabled: false, scope: { kind: 'global' } },
    ]);
    expect(settings.customVocabulary).toBe('Tauri');
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
