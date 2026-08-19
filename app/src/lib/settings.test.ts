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
  loadSettings,
  saveSettings,
  AUTO_STOP_SILENCE_OPTIONS,
  AVAILABLE_MODEL_OPTIONS,
  BUILTIN_MODES,
  DEFAULT_SETTINGS,
  LEGACY_OVERLAY_OFFSET_KEY,
  MODEL_OPTIONS,
  STORAGE_KEY,
} from './settings';

beforeEach(() => {
  localStorage.clear();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue(undefined);
  // Default to "not running under Tauri" so the existing localStorage-only
  // expectations below are unaffected by the durable store.
  mocks.isTauri.mockReturnValue(false);
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
      overlayVerticalOffset: 7,
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
        queryContextExcluded: true,
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

  it('marks an old System Default selection migration-complete without inventory proof', () => {
    const old = { ...DEFAULT_SETTINGS, settingsVersion: 2 } as Record<string, unknown>;
    delete old.microphoneIdMigrationComplete;
    localStorage.setItem('dictation-settings', JSON.stringify(old));
    expect(loadSettings().microphoneIdMigrationComplete).toBe(true);
  });

  it('normalizes System Default migration proof to complete even when stored false', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      microphone: 'system_default',
      microphoneIdMigrationComplete: false,
      settingsVersion: 3,
    }));
    expect(loadSettings().microphoneIdMigrationComplete).toBe(true);
  });

  it('leaves an old opaque microphone value pending live inventory proof', () => {
    const old = {
      ...DEFAULT_SETTINGS,
      settingsVersion: 3,
      microphone: 'UID or legacy display name',
    } as Record<string, unknown>;
    delete old.microphoneIdMigrationComplete;
    localStorage.setItem('dictation-settings', JSON.stringify(old));
    expect(loadSettings().microphoneIdMigrationComplete).toBe(false);
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

  it('allows zero-delay paste and bounds malformed persisted delays', () => {
    for (const [stored, expected] of [
      [0, 0],
      [23.9, 23],
      [900, 500],
      [-10, 0],
      ['slow', DEFAULT_SETTINGS.autoPasteDelayMs],
    ] as const) {
      localStorage.setItem('dictation-settings', JSON.stringify({
        ...DEFAULT_SETTINGS,
        autoPasteDelayMs: stored,
      }));
      expect(loadSettings().autoPasteDelayMs).toBe(expected);
    }
  });

  it('keeps the Paste Last shortcut opt-in and rejects malformed persistence', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS,
      pasteLastShortcutEnabled: 'yes',
    }));
    expect(loadSettings().pasteLastShortcutEnabled).toBe(false);
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS,
      pasteLastShortcutEnabled: true,
    }));
    expect(loadSettings().pasteLastShortcutEnabled).toBe(true);
  });

  it('defaults and bounds persisted sound cue settings', () => {
    for (const [stored, expected] of [
      [undefined, DEFAULT_SETTINGS.soundCueVolume],
      ['loud', DEFAULT_SETTINGS.soundCueVolume],
      [42.6, 43],
      [-5, 0],
      [500, 100],
    ] as const) {
      const persisted: Record<string, unknown> = {
        ...DEFAULT_SETTINGS,
        soundCueVolume: stored,
        soundCuesEnabled: 'yes',
        meetingSoundCuesEnabled: 1,
      };
      if (stored === undefined) delete persisted.soundCueVolume;
      localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted));
      const loaded = loadSettings();
      expect(loaded.soundCueVolume).toBe(expected);
      expect(loaded.soundCuesEnabled).toBe(DEFAULT_SETTINGS.soundCuesEnabled);
      expect(loaded.meetingSoundCuesEnabled).toBe(DEFAULT_SETTINGS.meetingSoundCuesEnabled);
    }
  });

  it('migrates the exact legacy 50 ms paste delay to zero once', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      autoPasteDelayMs: 50,
    }));

    expect(loadSettings().autoPasteDelayMs).toBe(0);
    expect(JSON.parse(localStorage.getItem('dictation-settings') ?? '{}')).toMatchObject({
      autoPasteDelayMs: 0,
      settingsVersion: 3,
    });

    saveSettings({ ...loadSettings(), autoPasteDelayMs: 50 });
    expect(loadSettings().autoPasteDelayMs).toBe(50);
  });

  it('preserves a deliberate 50 ms paste delay after the migration version', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      autoPasteDelayMs: 50,
      settingsVersion: 3,
    }));

    expect(loadSettings().autoPasteDelayMs).toBe(50);
  });

  it('versions legacy settings even when their paste delay does not change', () => {
    localStorage.setItem('dictation-settings', JSON.stringify({
      ...DEFAULT_SETTINGS,
      autoPasteDelayMs: 23,
    }));

    expect(loadSettings().autoPasteDelayMs).toBe(23);
    expect(JSON.parse(localStorage.getItem('dictation-settings') ?? '{}')).toMatchObject({
      autoPasteDelayMs: 23,
      settingsVersion: 3,
    });
  });

  it('resets offsets from the broken calibration flow once and removes its standalone key', () => {
    localStorage.setItem(LEGACY_OVERLAY_OFFSET_KEY, '48');
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS,
      overlayVerticalOffset: 12,
      settingsVersion: 1,
    }));

    expect(loadSettings().overlayVerticalOffset).toBe(0);
    expect(localStorage.getItem(LEGACY_OVERLAY_OFFSET_KEY)).toBeNull();
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}')).toMatchObject({
      overlayVerticalOffset: 0,
      settingsVersion: 3,
    });
  });

  it('preserves and bounds confirmed overlay offsets after migration', () => {
    for (const [stored, expected] of [
      [7, 7],
      [99, 12],
      [-99, -12],
      [4.9, 4],
      ['down', 0],
    ] as const) {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({
        ...DEFAULT_SETTINGS,
        overlayVerticalOffset: stored,
        settingsVersion: 2,
      }));
      expect(loadSettings().overlayVerticalOffset).toBe(expected);
    }
  });

  it('removes a legacy overlay offset even when no settings document exists', () => {
    localStorage.setItem(LEGACY_OVERLAY_OFFSET_KEY, '48');
    expect(loadSettings()).toEqual(DEFAULT_SETTINGS);
    expect(localStorage.getItem(LEGACY_OVERLAY_OFFSET_KEY)).toBeNull();
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
    expect(legacy.modeId).toBeUndefined();
  });

  it('exposes the seven immutable built-in Mode identities', () => {
    expect(BUILTIN_MODES.map((mode) => [mode.id, mode.name])).toEqual([
      ['builtin.everyday', 'Everyday'], ['builtin.messages', 'Messages'],
      ['builtin.email', 'Email'], ['builtin.notes', 'Notes'],
      ['builtin.technical', 'Technical'], ['builtin.terminal', 'Terminal'],
      ['builtin.verbatim', 'Verbatim'],
    ]);
  });

  it('sanitizes custom Modes and preserves unknown bindings for fail-closed resolution', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS,
      modes: [
        {
          id: 'mode.focus', name: ' Focus ', builtIn: false, writingStyle: 'notes',
          cleanupEnabled: true, smartFormattingEnabled: null, cliFormattingEnabled: null,
          vocabularyPolicy: 'general', contextPolicy: 'project', modelId: 'small.en',
          language: 'en', autoPaste: false,
        },
        { id: 'mode.invalid', name: 'Invalid', builtIn: false, modelId: 'missing-model' },
        { id: 'builtin.email', name: 'Spoofed', builtIn: false },
      ],
      appProfiles: [{ bundleId: 'com.example.App', label: 'App', modeId: 'missing.mode' }],
    }));
    const settings = loadSettings();
    expect(settings.modes).toEqual([expect.objectContaining({
      id: 'mode.focus', name: 'Focus', builtIn: false, writingStyle: 'notes',
      cleanupEnabled: true, smartFormattingEnabled: null,
      vocabularyPolicy: 'general', contextPolicy: 'project', modelId: 'small.en', language: 'en',
    })]);
    expect(settings.appProfiles[0].modeId).toBe('missing.mode');
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

  it('accepts only allow-listed stop-on-silence durations', () => {
    for (const value of AUTO_STOP_SILENCE_OPTIONS.map((option) => option.value)) {
      localStorage.setItem('dictation-settings', JSON.stringify({ ...DEFAULT_SETTINGS, autoStopSilenceMs: value }));
      expect(loadSettings().autoStopSilenceMs).toBe(value);
    }
  });

  it('coerces an unknown, tampered or absent stop-on-silence value back to Off', () => {
    for (const value of [900, -2500, 'soon', null, Number.NaN, 2500.5]) {
      localStorage.setItem('dictation-settings', JSON.stringify({ ...DEFAULT_SETTINGS, autoStopSilenceMs: value }));
      expect(loadSettings().autoStopSilenceMs).toBe(0);
    }
    localStorage.setItem('dictation-settings', JSON.stringify({
      model: 'base.en', doubleTapKey: 'shift_l', language: 'en', recordingMode: 'double_tap',
    }));
    expect(loadSettings().autoStopSilenceMs).toBe(0);
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

  it('uses Core ML for new installs and exposes the complete macOS catalog', () => {
    expect(DEFAULT_SETTINGS.model).toBe('parakeet-tdt-0.6b-v3-coreml');
    expect(AVAILABLE_MODEL_OPTIONS).toBe(MODEL_OPTIONS);
    expect(AVAILABLE_MODEL_OPTIONS.some((model) => model.backend === 'coreml')).toBe(true);
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

describe('durable settings store', () => {
  const STORAGE_KEY = 'dictation-settings';

  beforeEach(() => {
    mocks.isTauri.mockReturnValue(true);
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

  it('writes nothing on a first run with no cached settings', async () => {
    mocks.invoke.mockResolvedValue(null);

    await hydrateSettingsFromDisk();

    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(localStorage.getItem(STORAGE_KEY)).toBeNull();
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

  it('writes localStorage synchronously and mirrors the same blob to disk', () => {
    saveSettings({ ...DEFAULT_SETTINGS, language: 'ko' });

    const written = localStorage.getItem(STORAGE_KEY);
    expect(written).not.toBeNull();
    expect(JSON.parse(written ?? '{}')).toMatchObject({ language: 'ko', settingsVersion: 3 });
    expect(mocks.invoke).toHaveBeenCalledWith('save_settings_blob', { blob: written });
  });

  it('still persists to localStorage when the disk mirror fails', () => {
    mocks.invoke.mockRejectedValue(new Error('disk full'));

    saveSettings({ ...DEFAULT_SETTINGS, language: 'ru' });

    expect(loadSettings().language).toBe('ru');
  });
});

describe('Voice Query settings', () => {
  it('is fully opt-in with no default executable', () => {
    expect(DEFAULT_SETTINGS.queryHotkey).toBeNull();
    expect(DEFAULT_SETTINGS.queryProvider).toBe('custom');
    expect(DEFAULT_SETTINGS.queryExecutable).toBe('');
    expect(DEFAULT_SETTINGS.queryArguments).toEqual([]);
    expect(DEFAULT_SETTINGS.queryTimeoutSeconds).toBe(60);
    expect(DEFAULT_SETTINGS.queryContextLevel).toBe('none');
    expect(DEFAULT_SETTINGS.queryAutomaticallyCopyAnswers).toBe(true);
    expect(DEFAULT_SETTINGS.retainQueryHistory).toBe(false);
  });

  it('keeps auto-copy on for old or malformed documents and preserves an explicit opt-out', () => {
    for (const [stored, expected] of [
      [undefined, true],
      ['yes', true],
      [null, true],
      [false, false],
      [true, true],
    ] as const) {
      const settings = { ...DEFAULT_SETTINGS } as Record<string, unknown>;
      if (stored === undefined) {
        delete settings.queryAutomaticallyCopyAnswers;
      } else {
        settings.queryAutomaticallyCopyAnswers = stored;
      }
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
      expect(loadSettings().queryAutomaticallyCopyAnswers).toBe(expected);
    }
  });

  it('fails closed when a persisted query key conflicts with transform', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      transformHoldKey: 'alt_r',
      queryHotkey: 'alt_r',
      queryExecutable: '/usr/bin/printf',
    }));

    const settings = loadSettings();

    expect(settings.transformHoldKey).toBe('alt_r');
    expect(settings.queryHotkey).toBeNull();
  });

  it('bounds malformed CLI configuration before IPC', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      queryHotkey: 'unexpected_key',
      queryProvider: 'untrusted-provider',
      queryExecutable: 42,
      queryArguments: [...Array.from({ length: 40 }, (_, index) => `arg-${index}`), 7],
      queryTimeoutSeconds: 999,
      queryContextLevel: 'desktop_screenshot',
      queryAutomaticallyCopyAnswers: 'sometimes',
      retainQueryHistory: 'yes',
    }));

    const settings = loadSettings();

    expect(settings.queryHotkey).toBeNull();
    expect(settings.queryProvider).toBe('custom');
    expect(settings.queryExecutable).toBe('');
    expect(settings.queryArguments).toHaveLength(32);
    expect(settings.queryArguments.every((argument) => typeof argument === 'string')).toBe(true);
    expect(settings.queryTimeoutSeconds).toBe(60);
    expect(settings.queryContextLevel).toBe('none');
    expect(settings.queryAutomaticallyCopyAnswers).toBe(true);
    expect(settings.retainQueryHistory).toBe(false);
  });

  it('keeps valid context levels and fails closed for per-app exclusions', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      ...DEFAULT_SETTINGS,
      queryContextLevel: 'selection',
      appProfiles: [
        { bundleId: 'com.example.Private', queryContextExcluded: true },
        { bundleId: 'com.example.Legacy' },
        { bundleId: 'com.example.Tampered', queryContextExcluded: 'yes' },
      ],
    }));

    const settings = loadSettings();
    expect(settings.queryContextLevel).toBe('selection');
    expect(settings.appProfiles.map((profile) => profile.queryContextExcluded)).toEqual([
      true,
      false,
      false,
    ]);
  });
});
