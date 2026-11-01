import { afterEach, describe, expect, it } from 'vitest';
import { DEFAULT_SETTINGS, loadSettings, ownSettingsMigrations, saveSettings, STORAGE_KEY } from './settings';

afterEach(() => { localStorage.clear(); ownSettingsMigrations(false); });
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
  it('never lets a companion window persist a stale migration over a newer opt-out', () => {
    // A companion webview still holds a v6 cache after main saved a v7 opt-out.
    const stale = JSON.stringify({ ...DEFAULT_SETTINGS, settingsVersion: 6, overlayLiveWords: false });
    localStorage.setItem(STORAGE_KEY, stale);
    expect(loadSettings().overlayLiveWords).toBe(true);
    expect(localStorage.getItem(STORAGE_KEY)).toBe(stale);
    // The main window owns migrations and writes them back once.
    ownSettingsMigrations();
    loadSettings();
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY)!)).toMatchObject({ settingsVersion: 7, overlayLiveWords: true });
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
