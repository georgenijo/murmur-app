import { beforeEach, describe, expect, it } from 'vitest';
import {
  APPEARANCE_STORAGE_KEY,
  APPEARANCE_REVISION_ROLLOVER_AT,
  MAX_APPEARANCE_BYTES,
  MAX_THEME_FILE_BYTES,
  MAX_APPEARANCE_REVISION,
  createAppearanceDocument,
  exportAppearanceText,
  exportThemeLibraryEntryText,
  isNewerAppearanceRevision,
  loadAppearanceDocument,
  nextAppearanceRevision,
  previewAppearanceImport,
  readAppearancePreview,
  writeAppearanceDocument,
  writeAppearanceExport,
  makeLocalThemeEntry,
  type StorageLike,
} from '.';

class MemoryStorage implements StorageLike {
  value: string | null = null;
  getItem(key: string) {
    expect(key).toBe(APPEARANCE_STORAGE_KEY);
    return this.value;
  }
  setItem(key: string, value: string) {
    expect(key).toBe(APPEARANCE_STORAGE_KEY);
    this.value = value;
  }
}

describe('appearance storage', () => {
  let storage: MemoryStorage;

  beforeEach(() => {
    storage = new MemoryStorage();
  });

  it('uses Sonic/System for empty storage without requesting repair', () => {
    expect(loadAppearanceDocument(storage)).toMatchObject({
      document: { version: 1, revision: 0, mode: 'system', theme: { presetId: 'sonic' } },
      needsRepair: false,
      error: null,
    });
  });

  it.each([
    ['corrupt JSON', '{not json'],
    ['oversized', 'x'.repeat(MAX_APPEARANCE_BYTES + 1)],
    ['unknown version', JSON.stringify({ version: 2 })],
  ])('fails closed for %s storage', (_case, value) => {
    storage.value = value;
    const loaded = loadAppearanceDocument(storage);
    expect(loaded.document).toMatchObject({ mode: 'system', theme: { presetId: 'sonic' } });
    expect(loaded.needsRepair).toBe(true);
    expect(loaded.error).not.toBeNull();
  });

  it('preserves authoritative configuration and repairs a partial cache', () => {
    const source = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#abcdef',
    }, 12);
    storage.value = JSON.stringify({
      ...source,
      cache: { version: 1, light: source.cache.light },
    });
    const loaded = loadAppearanceDocument(storage);
    expect(loaded.document).toMatchObject({
      revision: 12,
      mode: 'dark',
      theme: { presetId: 'custom', accent: '#abcdef' },
    });
    expect(loaded.needsRepair).toBe(true);
    expect(loaded.document.cache.dark).toBeDefined();
  });

  it('reserves exhausted revisions and repairs saturated documents to a writable baseline', () => {
    for (const exhausted of [
      MAX_APPEARANCE_REVISION - 1,
      MAX_APPEARANCE_REVISION,
      Number.MAX_SAFE_INTEGER,
    ]) {
      const saturated = createAppearanceDocument('dark', {
        version: 1,
        presetId: 'custom',
        accent: '#123456',
      });
      saturated.revision = exhausted;
      storage.value = JSON.stringify(saturated);

      const loaded = loadAppearanceDocument(storage);
      expect(loaded.document).toMatchObject({
        revision: 0,
        mode: 'dark',
        theme: { presetId: 'custom', accent: '#123456' },
      });
      expect(loaded.needsRepair).toBe(true);
      expect(loaded.error).toBeNull();
      expect(createAppearanceDocument('dark', saturated.theme, exhausted).revision).toBe(0);
    }
  });

  it('rolls the last writable revision to one without producing an invalid document', () => {
    expect(nextAppearanceRevision(APPEARANCE_REVISION_ROLLOVER_AT - 1))
      .toBe(APPEARANCE_REVISION_ROLLOVER_AT);
    expect(nextAppearanceRevision(APPEARANCE_REVISION_ROLLOVER_AT)).toBe(1);
    expect(createAppearanceDocument('dark', undefined, 1).revision).toBe(1);
    expect(isNewerAppearanceRevision(APPEARANCE_REVISION_ROLLOVER_AT, 1)).toBe(true);
    expect(isNewerAppearanceRevision(1, APPEARANCE_REVISION_ROLLOVER_AT)).toBe(true);
    expect(isNewerAppearanceRevision(10, 9)).toBe(false);
  });

  it('round-trips a canonical document', () => {
    const document = createAppearanceDocument('light', {
      version: 1,
      presetId: 'custom',
      foreground: '#112233',
      contrast: -40,
    }, 3);
    writeAppearanceDocument(document, storage);
    expect(loadAppearanceDocument(storage)).toEqual({
      document,
      needsRepair: false,
      error: null,
    });
  });

  it('preserves independent library ownership in the active document', () => {
    const document = createAppearanceDocument('system', {
      version: 1,
      presetId: 'custom',
      light: { background: '#ffffff' },
      dark: { background: '#101010' },
    }, 8, { light: 'paper', dark: 'midnight' });
    writeAppearanceDocument(document, storage);
    expect(loadAppearanceDocument(storage).document.selection).toEqual({
      light: 'paper',
      dark: 'midnight',
    });
  });

  it('repairs semantically equivalent documents into canonical key order once', () => {
    const document = createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#abcdef',
    }, 7);
    storage.value = JSON.stringify({
      cache: document.cache,
      theme: document.theme,
      mode: document.mode,
      revision: document.revision,
      version: document.version,
    });
    expect(storage.value).not.toBe(JSON.stringify(document));

    const loaded = loadAppearanceDocument(storage);
    expect(loaded).toEqual({
      document,
      needsRepair: true,
      error: null,
    });

    writeAppearanceDocument(loaded.document, storage);
    expect(loadAppearanceDocument(storage)).toEqual({
      document,
      needsRepair: false,
      error: null,
    });
  });

  it('reports storage access errors without throwing into rendering', () => {
    const unavailable: StorageLike = {
      getItem: () => { throw new Error('denied'); },
      setItem: () => { throw new Error('denied'); },
    };
    const loaded = loadAppearanceDocument(unavailable);
    expect(loaded.document.mode).toBe('system');
    expect(loaded.error).toContain('denied');
  });
});

describe('theme file transport helpers', () => {
  it('exports authoritative configuration without a cache', () => {
    const text = exportAppearanceText(createAppearanceDocument('dark', {
      version: 1,
      presetId: 'custom',
      accent: '#123456',
    }, 99));
    const parsed = JSON.parse(text);
    expect(parsed).toEqual({
      version: 1,
      mode: 'dark',
      theme: { version: 1, presetId: 'custom', accent: '#123456' },
    });
    expect(parsed.cache).toBeUndefined();
    expect(parsed.revision).toBeUndefined();
  });

  it.each([
    ['malformed', '{'],
    ['unsupported document version', JSON.stringify({ version: 2 })],
    ['unsupported theme version', JSON.stringify({ version: 1, mode: 'system', theme: { version: 2 } })],
    ['invalid mode', JSON.stringify({ version: 1, mode: 'sepia', theme: { version: 1, presetId: 'sonic' } })],
    ['oversized', ' '.repeat(MAX_THEME_FILE_BYTES + 1)],
  ])('rejects %s imports before commit', (_case, text) => {
    expect(() => previewAppearanceImport(text)).toThrow();
  });

  it('imports JSONC VS Code themes through the same accessibility resolver', () => {
    const preview = previewAppearanceImport(`{
      // VS Code-style comments and trailing commas are accepted.
      "name": "night-drive",
      "type": "dark",
      "colors": {
        "editor.background": "#101214",
        "editor.foreground": "#eef2f4",
        "button.background": "#7cccf0",
      },
    }`);
    expect(preview).toMatchObject({
      mode: 'dark',
      label: 'Night Drive',
      modes: ['dark'],
      theme: { presetId: 'custom' },
    });
    expect(preview.dark.background).toBe('#101214');
  });

  it('exports and previews named library theme files without source provenance', () => {
    const entry = makeLocalThemeEntry('paper', 'Paper', {
      version: 1,
      presetId: 'custom',
      background: '#fbfbfa',
      foreground: '#202020',
      accent: '#176b88',
    }, ['light']);
    const text = exportThemeLibraryEntryText(entry);
    const raw = JSON.parse(text);
    expect(raw).toMatchObject({ version: 2, name: 'Paper', modes: ['light'] });
    expect(raw).not.toHaveProperty('source');
    expect(raw).not.toHaveProperty('id');
    expect(previewAppearanceImport(text)).toMatchObject({
      label: 'Paper',
      modes: ['light'],
      mode: 'light',
    });
  });

  it('uses provided read/write callbacks and never the clipboard', async () => {
    const document = createAppearanceDocument();
    let readPath = '';
    let written: { path: string; contents: string } | null = null;
    const preview = await readAppearancePreview('/tmp/theme.json', async (path) => {
      readPath = path;
      return exportAppearanceText(document);
    });
    await writeAppearanceExport('/tmp/out.json', document, async (path, contents) => {
      written = { path, contents };
    });
    expect(readPath).toBe('/tmp/theme.json');
    expect(preview.theme.presetId).toBe('sonic');
    expect(written).toEqual({
      path: '/tmp/out.json',
      contents: exportAppearanceText(document),
    });
  });
});
