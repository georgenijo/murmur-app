import { describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { buildConfigureOptions, cancelRecording } from './dictation';
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
          queryContextExcluded: true,
        },
      ],
    });

    expect(options.smartFormattingEnabled).toBe(true);
    expect(options.smartDestinationEnabled).toBe(true);
    expect(options.smartPunctuation).toBe(false);
    expect(options.appProfiles?.[0].smartFormattingOverride).toBe(false);
    expect(options.appProfiles?.[0].writingStyle).toBe('code_technical');
    expect(options.appProfiles?.[0].queryContextExcluded).toBe(true);
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
