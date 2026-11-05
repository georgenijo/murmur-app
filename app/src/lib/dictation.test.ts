import { describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { buildConfigureOptions, cancelRecording, startRecording, stopRecording } from './dictation';
import { DEFAULT_SETTINGS } from './settings';

describe('buildConfigureOptions', () => {
  it('sends the opt-in homophone tier to the recording configuration', () => {
    expect(buildConfigureOptions(DEFAULT_SETTINGS).homophoneContextEnabled).toBe(false);
    expect(buildConfigureOptions({ ...DEFAULT_SETTINGS, homophoneContextEnabled: true }).homophoneContextEnabled).toBe(true);
  });

  it('sends smart formatting and its independent per-app override to Rust', () => {
    const options = buildConfigureOptions({
      ...DEFAULT_SETTINGS,
      smartPunctuation: false,
      smartFormattingEnabled: true,
      smartDestinationEnabled: true,
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
    expect(options.smartDestinationEnabled).toBe(true);
    expect(options.smartPunctuation).toBe(false);
    expect(options.appProfiles?.[0].smartFormattingOverride).toBe(false);
    expect(options.appProfiles?.[0].writingStyle).toBe('code_technical');
  });
});

describe('cancelRecording', () => {
  it('scopes cancellation to an expected generation only when one is given', async () => {
    await cancelRecording();
    await cancelRecording(9);
    expect(invoke).toHaveBeenNthCalledWith(1, 'cancel_native_recording');
    expect(invoke).toHaveBeenNthCalledWith(2, 'cancel_native_recording', { expectedRecordingId: 9 });
  });
});

describe('startRecording gesture correlation', () => {
  it('carries only the originating keyboard token; a later toolbar call omits it', async () => {
    invoke.mockClear();
    await startRecording(undefined, 'toggle', null, '18446744073709551614');
    await startRecording();
    expect(invoke).toHaveBeenNthCalledWith(1, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'toggle', gestureToken: '18446744073709551614',
    });
    expect(invoke).toHaveBeenNthCalledWith(2, 'start_native_recording', {
      deviceName: null, smartAuto: null, origin: 'toggle',
    });
  });
});

describe('stopRecording', () => {
  it('sends the stop trigger explicitly and keeps legacy callers conservative', async () => {
    await stopRecording('toggle');
    expect(invoke).toHaveBeenLastCalledWith('stop_native_recording', { stopTrigger: 'toggle' });
    await stopRecording();
    expect(invoke).toHaveBeenLastCalledWith('stop_native_recording', { stopTrigger: 'other' });
  });
});
