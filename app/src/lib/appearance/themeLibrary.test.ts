import { describe, expect, it } from 'vitest';

import {
  MAX_THEME_LIBRARY_BYTES,
  THEME_LIBRARY_STORAGE_KEY,
  appearanceSelection,
  composeThemeSelection,
  createAppearanceDocument,
  effectiveAppearanceSelection,
  installThemeLibraryEntries,
  loadThemeLibrary,
  makeLocalThemeEntry,
  previewThemeLibrarySelection,
  previewThemeLibraryPairSelection,
  removeThemeLibraryEntries,
  replaceThemeLibraryCollection,
  writeThemeLibrary,
  type ThemeLibraryDocumentV1,
  type ThemeLibraryEntryV1,
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

function localTheme(id: string, label = id): ThemeLibraryEntryV1 {
  return makeLocalThemeEntry(id, label, {
    version: 1,
    presetId: 'custom',
    accent: id.endsWith('dark') ? '#8ccfff' : '#075f7a',
    background: id.endsWith('dark') ? '#101417' : '#f7fafc',
    foreground: id.endsWith('dark') ? '#eef5f8' : '#20282c',
  });
}

function collectionTheme(
  id: string,
  collectionId = 'open-vsx:sample.theme',
): ThemeLibraryEntryV1 {
  return {
    ...localTheme(id),
    source: {
      kind: 'open-vsx',
      extensionId: 'sample.theme',
      version: '1.0.0',
      license: 'MIT',
      sourceUrl: 'https://example.com/sample/theme',
    },
    collection: { id: collectionId, label: 'Sample Theme' },
  };
}

describe('theme library storage', () => {
  it('loads an empty library and round-trips canonical entries', () => {
    const storage = new MemoryStorage();
    expect(loadThemeLibrary(storage)).toEqual({
      status: 'ready',
      document: { version: 1, revision: 0, themes: [] },
      needsRepair: false,
    });

    const document: ThemeLibraryDocumentV1 = {
      version: 1,
      revision: 4,
      themes: [localTheme('my-theme', 'My Theme')],
    };
    writeThemeLibrary(document, storage);
    expect(loadThemeLibrary(storage)).toEqual({
      status: 'ready',
      document,
      needsRepair: false,
    });
  });

  it.each([
    ['corrupt JSON', '{'],
    ['unsupported schema', JSON.stringify({ version: 2, revision: 0, themes: [] })],
    ['oversized storage', 'x'.repeat(MAX_THEME_LIBRARY_BYTES + 1)],
  ])('fails closed for %s', (_label, value) => {
    const storage = new MemoryStorage();
    storage.value = value;
    const loaded = loadThemeLibrary(storage);
    expect(loaded.status).toBe('unavailable');
    expect(loaded.document.themes).toEqual([]);
  });

  it('drops invalid and duplicate entries during an explicit repair pass', () => {
    const storage = new MemoryStorage();
    const valid = localTheme('valid-theme');
    storage.value = JSON.stringify({
      version: 1,
      revision: 9,
      themes: [valid, valid, { ...valid, id: '../escape' }],
    });
    const loaded = loadThemeLibrary(storage);
    expect(loaded).toMatchObject({
      status: 'ready',
      needsRepair: true,
      document: { revision: 9, themes: [{ id: 'valid-theme' }] },
    });
  });

  it('rejects stale revisions and duplicate IDs without partial writes', () => {
    const storage = new MemoryStorage();
    writeThemeLibrary({ version: 1, revision: 2, themes: [] }, storage);
    const before = storage.value;
    expect(() => installThemeLibraryEntries(1, [localTheme('one')], storage))
      .toThrow(/changed while this operation was running/);
    expect(storage.value).toBe(before);

    const installed = installThemeLibraryEntries(2, [localTheme('one')], storage);
    expect(() => installThemeLibraryEntries(installed.revision, [localTheme('one')], storage))
      .toThrow(/already installed/);
    expect(loadThemeLibrary(storage).document).toEqual(installed);
  });

  it('atomically replaces and removes complete extension collections', () => {
    const storage = new MemoryStorage();
    const oldCollection = [collectionTheme('old-light'), collectionTheme('old-dark')];
    writeThemeLibrary({
      version: 1,
      revision: 3,
      themes: [localTheme('personal'), ...oldCollection],
    }, storage);
    const replacement = [collectionTheme('new-light')];
    const updated = replaceThemeLibraryCollection(
      3,
      'open-vsx:sample.theme',
      replacement,
      oldCollection,
      storage,
    );
    expect(updated.themes.map((theme) => theme.id)).toEqual(['personal', 'new-light']);
    expect(() => replaceThemeLibraryCollection(
      updated.revision,
      'open-vsx:sample.theme',
      replacement,
      oldCollection,
      storage,
    )).toThrow(/changed while its update was downloading/);

    const removed = removeThemeLibraryEntries(updated.revision, ['new-light'], storage);
    expect(removed.themes.map((theme) => theme.id)).toEqual(['personal']);
  });
});

describe('theme library selection', () => {
  it('migrates legacy active themes to Sonic or Custom ownership', () => {
    expect(appearanceSelection(createAppearanceDocument())).toEqual({ light: 'sonic', dark: 'sonic' });
    expect(appearanceSelection(createAppearanceDocument('system', {
      version: 1,
      presetId: 'custom',
      accent: '#123456',
    }))).toEqual({ light: 'custom', dark: 'custom' });
  });

  it('labels source ownership as Custom when the stored theme no longer matches it', () => {
    const custom = createAppearanceDocument('system', {
      version: 1,
      presetId: 'custom',
      accent: '#123456',
    }, 1, { light: 'sonic', dark: 'sonic' });

    expect(effectiveAppearanceSelection(custom, {
      version: 1,
      revision: 0,
      themes: [],
    })).toEqual({ light: 'custom', dark: 'custom' });

    const imported = localTheme('paper-light', 'Paper');
    const library = { version: 1 as const, revision: 1, themes: [imported] };
    const preview = previewThemeLibrarySelection(
      createAppearanceDocument(),
      library,
      imported.id,
      'light',
    );
    const applied = createAppearanceDocument(
      preview.mode,
      preview.theme,
      2,
      preview.selection,
    );
    expect(effectiveAppearanceSelection(applied, library)).toEqual({
      light: imported.id,
      dark: 'sonic',
    });
  });

  it('composes independently selected light and dark variants into the active cache', () => {
    const light = localTheme('paper-light');
    const dark = localTheme('midnight-dark');
    const library = { version: 1 as const, revision: 1, themes: [light, dark] };
    const current = createAppearanceDocument();
    const preview = previewThemeLibrarySelection(current, library, light.id, 'light');
    const mixed = previewThemeLibrarySelection(
      createAppearanceDocument(preview.mode, preview.theme, 1, preview.selection),
      library,
      dark.id,
      'dark',
    );
    expect(mixed.selection).toEqual({ light: 'paper-light', dark: 'midnight-dark' });
    expect(mixed.theme.light).toEqual(light.theme.light ?? mixed.light);
    expect(mixed.light.background).not.toBe(mixed.dark.background);
    expect(composeThemeSelection(current, library, mixed.selection!)).toEqual(mixed.theme);
  });

  it('refuses missing or mode-incompatible owners', () => {
    const lightOnly = makeLocalThemeEntry('light-only', 'Light only', {
      version: 1,
      presetId: 'custom',
      background: '#ffffff',
      foreground: '#111111',
      accent: '#006080',
    }, ['light']);
    const library = { version: 1 as const, revision: 1, themes: [lightOnly] };
    expect(() => previewThemeLibrarySelection(createAppearanceDocument(), library, 'missing'))
      .toThrow(/not installed/);
    expect(() => previewThemeLibrarySelection(createAppearanceDocument(), library, lightOnly.id, 'dark'))
      .toThrow(/no dark variant/);
  });

  it('composes a collection light and dark choice in one transaction', () => {
    const light = makeLocalThemeEntry('aurora-light', 'Aurora Light', {
      version: 1,
      presetId: 'custom',
      background: '#f8f4ec',
      foreground: '#171717',
      accent: '#17627a',
    }, ['light']);
    const dark = makeLocalThemeEntry('aurora-dark', 'Aurora Dark', {
      version: 1,
      presetId: 'custom',
      background: '#11151a',
      foreground: '#f2f5f7',
      accent: '#8ccfff',
    }, ['dark']);
    const library = { version: 1 as const, revision: 1, themes: [light, dark] };
    const preview = previewThemeLibraryPairSelection(
      createAppearanceDocument(),
      library,
      light.id,
      dark.id,
    );

    expect(preview.selection).toEqual({ light: light.id, dark: dark.id });
    expect(preview.light.background).not.toBe(preview.dark.background);
    expect(preview.theme).toEqual(
      composeThemeSelection(createAppearanceDocument(), library, preview.selection!),
    );
    expect(() => previewThemeLibraryPairSelection(
      createAppearanceDocument(),
      library,
      dark.id,
      dark.id,
    )).toThrow(/no light variant/);
  });
});
