import { describe, expect, it, vi } from 'vitest';
import { buildConfigureOptions, startRecording } from './dictation';
import { DEFAULT_SETTINGS } from './settings';

const invoke = vi.hoisted(() => vi.fn(async () => ({ type: 'recording_starting' })));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('buildConfigureOptions', () => {
  it('sends smart formatting and its independent per-app override to Rust', () => {
    const options = buildConfigureOptions({
      ...DEFAULT_SETTINGS,
      smartPunctuation: false,
      smartFormattingEnabled: true,
      appProfiles: [
        {
          bundleId: 'com.apple.Terminal',
          label: 'Terminal',
          autoPasteOverride: null,
          cleanupOverride: null,
          smartFormattingOverride: false,
          cliFormattingOverride: true,
          writingStyle: 'code_technical',
          ideContextEnabled: false,
          ideProjectRoots: [],
        },
      ],
    });

    expect(options.smartFormattingEnabled).toBe(true);
    expect(options.smartPunctuation).toBe(false);
    expect(options.appProfiles?.[0].smartFormattingOverride).toBe(false);
    expect(options.appProfiles?.[0].writingStyle).toBe('code_technical');
  });
});

describe('startRecording microphone policy', () => {
  it('sends preferred-device fallback only for an explicit microphone', async () => {
    await startRecording('USB-A', 'hold', true);
    expect(invoke).toHaveBeenLastCalledWith('start_native_recording', {
      deviceName: 'USB-A',
      fallbackToDefault: true,
      origin: 'hold',
    });

    await startRecording('system_default', 'toggle', true);
    expect(invoke).toHaveBeenLastCalledWith('start_native_recording', {
      deviceName: null,
      fallbackToDefault: false,
      origin: 'toggle',
    });
  });
});
