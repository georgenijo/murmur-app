import { afterEach, describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, loadSettings, saveSettings } from './settings';

afterEach(() => { localStorage.clear(); });
describe('voice island settings', () => {
  it('defaults to the notch with live words on', () => {
    localStorage.clear();
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayLiveWords]).toEqual(['notch', true]);
  });
  it('round-trips placement and an explicit live-words opt-out without storing preview content', () => {
    for (const placement of ['notch', 'floating_pill'] as const) {
      saveSettings({ ...DEFAULT_SETTINGS, overlayPlacement: placement, overlayLiveWords: false, overlayVerticalOffset: -4 });
      const s = loadSettings();
      expect([s.overlayPlacement, s.overlayLiveWords, s.overlayVerticalOffset]).toEqual([placement, false, -4]);
      expect(Object.keys(s).filter(key => /partial|stableText|tentativeText/i.test(key))).toEqual([]);
    }
  });
});
