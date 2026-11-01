import { afterEach, describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, loadSettings, saveSettings, STORAGE_KEY } from './settings';

afterEach(() => localStorage.clear());
describe('voice island settings', () => {
  it('defaults to the notch with live words on', () => {
    localStorage.clear();
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayLiveWords]).toEqual(['notch', true]);
  });
  it('does not resurrect retired fields or accept malformed persisted values', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 7,
      liveTranscriptPreview: true, overlayPlacement: 'menu_bar', overlayProcessingStyle: 'spin', overlayLiveWords: 'false' }));
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayLiveWords]).toEqual(['notch', true]);
    expect('liveTranscriptPreview' in s).toBe(false);
    expect('overlayProcessingStyle' in s).toBe(false);
  });
  it('migrates the below-menu-bar placement to the floating pill and turns live words on once', () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 6,
      overlayPlacement: 'below_menu_bar', overlayLiveWords: false, overlayProcessingStyle: 'flow' }));
    const s = loadSettings();
    expect([s.overlayPlacement, s.overlayLiveWords]).toEqual(['floating_pill', true]);
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
