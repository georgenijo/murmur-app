import { afterEach, describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, loadSettings, saveSettings, STORAGE_KEY } from './settings';

afterEach(() => localStorage.clear());
describe('voice island settings', () => {
  it('defaults to the notch, Flow, and explicit opt-in for live words', () => {
    localStorage.clear();
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayProcessingStyle, s.overlayLiveWords]).toEqual(['notch', 'flow', false]);
  });
  it('does not resurrect the retired preview flag or accept malformed persisted values', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS,
      liveTranscriptPreview: true, overlayPlacement: 'menu_bar', overlayProcessingStyle: 'spin', overlayLiveWords: 'true' }));
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayProcessingStyle, s.overlayLiveWords]).toEqual(['notch', 'flow', false]);
    expect('liveTranscriptPreview' in s).toBe(false);
  });
  it('round-trips each supported style and preserves calibration without storing preview content', () => {
    for (const style of ['sweep', 'flow', 'pulse'] as const) {
      saveSettings({ ...DEFAULT_SETTINGS, overlayPlacement: 'below_menu_bar', overlayLiveWords: true,
        overlayProcessingStyle: style, overlayVerticalOffset: -4 });
      const s = loadSettings();
      expect([s.overlayPlacement, s.overlayProcessingStyle, s.overlayLiveWords, s.overlayVerticalOffset])
        .toEqual(['below_menu_bar', style, true, -4]);
      expect(Object.keys(s).filter(key => /partial|stableText|tentativeText/i.test(key))).toEqual([]);
    }
  });
});
