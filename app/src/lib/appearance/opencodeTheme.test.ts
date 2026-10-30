import { describe, expect, it } from 'vitest';

import {
  BUILTIN_THEME_LIBRARY_ENTRIES,
  MURMUR_TOKEN_NAMES,
  OPENCODE_DARK,
  OPENCODE_LIGHT,
  OPENCODE_THEME,
  OPENCODE_THEME_ID,
  THEME_LIBRARY_STORAGE_KEY,
  applyResolvedTheme,
  availableThemeId,
  createAppearanceDocument,
  effectiveAppearanceSelection,
  emptyThemeLibrary,
  exportThemeLibraryEntryText,
  findThemeLibraryEntry,
  installThemeLibraryEntries,
  isBuiltinThemeId,
  loadThemeLibrary,
  makeLocalThemeEntry,
  nonTextContrastFailures,
  previewAppearanceImport,
  previewThemeLibraryPairSelection,
  previewThemeLibrarySelection,
  removeThemeLibraryEntries,
  resolveTheme,
  semanticContrastFailures,
  type MurmurTokens,
  type ThemeLibraryStorageLike,
} from '.';

class MemoryStorage implements ThemeLibraryStorageLike {
  value: string | null = null;

  getItem(key: string) {
    expect(key).toBe(THEME_LIBRARY_STORAGE_KEY);
    return this.value;
  }

  setItem(key: string, value: string) {
    expect(key).toBe(THEME_LIBRARY_STORAGE_KEY);
    this.value = value;
  }
}

// The exact `--murmur-*` values OpenCode renders with after the mutable-palette
// resolver. Differences from OPENCODE_LIGHT/OPENCODE_DARK are the documented
// contrast repairs in docs/features/appearance.md.
const RESOLVED_LIGHT: MurmurTokens = {
  background: '#f8f8f8',
  surface: '#f8f8f8',
  'surface-container-low': '#fafafa',
  'surface-container': '#f2f2f2',
  'surface-container-high': '#eeeeee',
  'surface-container-lowest': '#ffffff',
  'surface-container-highest': '#dbdbdb',
  primary: '#304ce6',
  'primary-dim': '#2840cf',
  'on-primary': '#ffffff',
  'on-surface': '#161616',
  'on-surface-variant': '#5c5c5c',
  'outline-variant': '#cccccc',
  error: '#a91b29',
  success: '#00642b',
  warning: '#705300',
};

const RESOLVED_DARK: MurmurTokens = {
  background: '#121212',
  surface: '#121212',
  'surface-container-low': '#242424',
  'surface-container': '#2e2e2e',
  'surface-container-high': '#3a3a3a',
  'surface-container-lowest': '#262626',
  'surface-container-highest': '#3c3c3c',
  primary: '#a2bcff',
  'primary-dim': '#8eacff',
  'on-primary': '#161616',
  'on-surface': '#fafafa',
  'on-surface-variant': '#aeaeae',
  'outline-variant': '#454545',
  error: '#ff9a95',
  success: '#6bd586',
  warning: '#f2cf76',
};

const opencode = BUILTIN_THEME_LIBRARY_ENTRIES.find((entry) => entry.id === OPENCODE_THEME_ID)!;

describe('OpenCode built-in palette', () => {
  it('stores the Rune @ 100b7d93 values on Murmur roles', () => {
    expect(OPENCODE_LIGHT).toMatchObject({
      background: '#f8f8f8',
      'surface-container-lowest': '#ffffff',
      'surface-container-low': '#fafafa',
      'surface-container': '#f2f2f2',
      'surface-container-high': '#eeeeee',
      'surface-container-highest': '#dbdbdb',
      'on-surface': '#161616',
      'on-surface-variant': '#5c5c5c',
      primary: '#3b5cf6',
      'outline-variant': '#cccccc',
      success: '#198b43',
      error: '#b82d35',
      warning: '#cb9f34',
    });
    expect(OPENCODE_DARK).toMatchObject({
      background: '#121212',
      'surface-container-lowest': '#161616',
      'surface-container-low': '#242424',
      'surface-container': '#2e2e2e',
      'surface-container-high': '#3a3a3a',
      'surface-container-highest': '#5c5c5c',
      'on-surface': '#fafafa',
      'on-surface-variant': '#aeaeae',
      primary: '#a2bcff',
      'primary-dim': '#7698fd',
      'outline-variant': '#454545',
      success: '#6bd586',
      error: '#f17471',
      warning: '#f2cf76',
    });
    expect(Object.keys(OPENCODE_LIGHT)).toEqual([...MURMUR_TOKEN_NAMES]);
    expect(Object.keys(OPENCODE_DARK)).toEqual([...MURMUR_TOKEN_NAMES]);
  });

  it.each([
    ['light', RESOLVED_LIGHT],
    ['dark', RESOLVED_DARK],
  ] as const)('resolves %s to exact --murmur-* values inside the contrast matrix', (appearance, expected) => {
    const resolved = resolveTheme(OPENCODE_THEME, appearance);
    expect(resolved.tokens).toEqual(expected);
    expect(semanticContrastFailures(resolved.tokens)).toEqual([]);
    expect(nonTextContrastFailures(resolved.tokens)).toEqual([]);
    // Reapplying the compiled palette is stable.
    expect(resolveTheme({ version: 1, presetId: 'custom', [appearance]: resolved.tokens }, appearance).tokens)
      .toEqual(expected);

    const root = document.createElement('html');
    applyResolvedTheme(resolved, root);
    for (const token of MURMUR_TOKEN_NAMES) {
      expect(root.style.getPropertyValue(`--murmur-${token}`)).toBe(expected[token]);
    }
  });

  it('records exactly the documented contrast repairs and no emergency fallback', () => {
    const summary = (appearance: 'light' | 'dark') => resolveTheme(OPENCODE_THEME, appearance)
      .adjustments.map(({ token, from, to, reason }) => `${token} ${from}->${to} ${reason}`);
    expect(summary('light')).toEqual([
      'primary #3b5cf6->#304ce6 contrast',
      'error #b82d35->#a91b29 contrast',
      'success #198b43->#00642b contrast',
      'warning #cb9f34->#705300 contrast',
    ]);
    expect(summary('dark')).toEqual([
      'surface-container-lowest #161616->#262626 contrast',
      'surface-container-highest #5c5c5c->#3c3c3c contrast',
      'error #f17471->#ff9a95 contrast',
    ]);
  });
});

describe('OpenCode built-in library entry', () => {
  it('is a paired, read-only built-in outside the stored library', () => {
    expect(opencode).toEqual({
      version: 1,
      id: 'opencode',
      label: 'OpenCode',
      modes: ['light', 'dark'],
      theme: OPENCODE_THEME,
      source: { kind: 'local' },
    });
    expect(isBuiltinThemeId('opencode')).toBe(true);
    expect(isBuiltinThemeId('sonic')).toBe(false);
    expect(findThemeLibraryEntry(emptyThemeLibrary(), 'opencode')).toBe(opencode);
  });

  it('cannot be removed or shadowed by an installed theme', () => {
    const storage = new MemoryStorage();
    const library = installThemeLibraryEntries(0, [
      makeLocalThemeEntry('paper', 'Paper', { version: 1, presetId: 'custom', accent: '#075f7a' }),
    ], storage);
    const before = storage.value;
    expect(() => removeThemeLibraryEntries(library.revision, ['opencode'], storage))
      .toThrow(/not installed/);
    expect(() => installThemeLibraryEntries(library.revision, [
      makeLocalThemeEntry('opencode', 'OpenCode', OPENCODE_THEME),
    ], storage)).toThrow(/already installed/);
    expect(storage.value).toBe(before);
    expect(availableThemeId('OpenCode', new Set())).toBe('opencode-2');
  });

  it('moves a previously saved "opencode" theme to a fresh ID without losing it', () => {
    const storage = new MemoryStorage();
    const saved = makeLocalThemeEntry('opencode', 'OpenCode', {
      version: 1,
      presetId: 'custom',
      accent: '#ff00aa',
    });
    const next = makeLocalThemeEntry('opencode-2', 'Opencode 2', { version: 1, presetId: 'custom' });
    storage.value = JSON.stringify({ version: 1, revision: 3, themes: [saved, next] });
    const loaded = loadThemeLibrary(storage);
    expect(loaded.status).toBe('ready');
    if (loaded.status !== 'ready') return;
    expect(loaded.needsRepair).toBe(true);
    expect(loaded.document.themes.map((entry) => entry.id)).toEqual(['opencode-3', 'opencode-2']);
    expect(loaded.document.themes[0]).toEqual({ ...saved, id: 'opencode-3' });
  });

  it('applies both variants from the card, or one per swatch', () => {
    const current = createAppearanceDocument('light');
    const library = emptyThemeLibrary();
    const pair = previewThemeLibrarySelection(current, library, 'opencode');
    expect(pair.selection).toEqual({ light: 'opencode', dark: 'opencode' });
    expect(pair.mode).toBe('light');
    expect(pair.theme).toEqual(OPENCODE_THEME);
    expect(pair.light).toEqual(RESOLVED_LIGHT);
    expect(pair.dark).toEqual(RESOLVED_DARK);
    expect(previewThemeLibraryPairSelection(current, library, 'opencode', 'opencode').theme)
      .toEqual(OPENCODE_THEME);

    const applied = createAppearanceDocument(pair.mode, pair.theme, 2, pair.selection);
    expect(effectiveAppearanceSelection(applied, library)).toEqual({ light: 'opencode', dark: 'opencode' });

    const darkOnly = previewThemeLibrarySelection(current, library, 'opencode', 'dark');
    expect(darkOnly.mode).toBe('dark');
    expect(darkOnly.selection).toEqual({ light: 'sonic', dark: 'opencode' });
    expect(darkOnly.dark).toEqual(RESOLVED_DARK);
  });

  it('duplicates into an ordinary removable local theme with the same palette', () => {
    const storage = new MemoryStorage();
    const label = 'Copy of OpenCode';
    const copy = makeLocalThemeEntry(availableThemeId(label, new Set()), label, opencode.theme, opencode.modes);
    const library = installThemeLibraryEntries(0, [copy], storage);
    expect(library.themes).toEqual([{
      version: 1,
      id: 'copy-of-opencode',
      label: 'Copy of OpenCode',
      modes: ['light', 'dark'],
      theme: OPENCODE_THEME,
      source: { kind: 'local' },
    }]);
    expect(resolveTheme(library.themes[0]!.theme, 'dark').tokens).toEqual(RESOLVED_DARK);
    expect(removeThemeLibraryEntries(library.revision, ['copy-of-opencode'], storage).themes).toEqual([]);
  });

  it('exports a portable library-theme file that imports back to the same palette', () => {
    const text = exportThemeLibraryEntryText(opencode);
    expect(JSON.parse(text)).toEqual({
      version: 2,
      name: 'OpenCode',
      modes: ['light', 'dark'],
      theme: { version: 1, presetId: 'custom', light: OPENCODE_LIGHT, dark: OPENCODE_DARK },
    });
    const imported = previewAppearanceImport(text);
    expect(imported.label).toBe('OpenCode');
    expect(imported.modes).toEqual(['light', 'dark']);
    expect(imported.light).toEqual(RESOLVED_LIGHT);
    expect(imported.dark).toEqual(RESOLVED_DARK);
  });
});
