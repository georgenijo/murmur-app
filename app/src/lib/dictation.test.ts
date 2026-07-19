import { describe, expect, it } from 'vitest';
import { buildConfigureOptions } from './dictation';
import { DEFAULT_SETTINGS } from './settings';

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
        },
      ],
    });

    expect(options.smartFormattingEnabled).toBe(true);
    expect(options.smartPunctuation).toBe(false);
    expect(options.appProfiles?.[0].smartFormattingOverride).toBe(false);
  });
});
