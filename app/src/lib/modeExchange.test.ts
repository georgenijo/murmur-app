import { describe, expect, it } from 'vitest';
import { AVAILABLE_MODEL_OPTIONS, BUILTIN_MODES, DEFAULT_SETTINGS, LANGUAGE_OPTIONS, loadSettings, type MurmurMode, type AppProfile } from './settings';
import { commitModeImport, exportModeFile, MAX_MODE_FILE_BYTES, parseModeFile, previewModeImport, type ModeExchangeSettings } from './modeExchange';

const mode: MurmurMode = {
  ...BUILTIN_MODES[0], id: 'mode.focus', name: 'Focus', builtIn: false,
  enabled: false, writingStyle: 'notes', cleanupEnabled: true, smartFormattingEnabled: false,
  cliFormattingEnabled: true, vocabularyPolicy: 'technical', contextPolicy: 'project',
  modelId: 'base.en', language: 'en', autoPaste: true,
};
const profile: AppProfile = {
  bundleId: 'com.example.Editor', label: 'Editor', autoPasteOverride: false,
  cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
  writingStyle: null, ideContextEnabled: false, ideProjectRoots: ['/private/project'], queryContextExcluded: true,
};
const rule = { id: 'site.example', browserBundleId: 'com.apple.Safari', host: 'example.com', modeId: mode.id, enabled: true };
const empty: ModeExchangeSettings = { modes: [], appProfiles: [], browserSiteRules: [], siteModeLookupEnabled: false };
const source: ModeExchangeSettings = { modes: [mode], appProfiles: [{ ...profile, modeId: mode.id }], browserSiteRules: [rule], siteModeLookupEnabled: true };
const file = () => parseModeFile(exportModeFile(source));

function expectRejected(update: object) {
  expect(() => parseModeFile(JSON.stringify({ ...file(), ...update }))).toThrow();
}

describe('Mode JSON boundary', () => {
  it('keeps a full export with escaped strings within the import byte limit', () => {
    const modelId = AVAILABLE_MODEL_OPTIONS.reduce((longest, option) => option.value.length > longest.value.length ? option : longest).value;
    const language = LANGUAGE_OPTIONS.reduce((longest, option) => option.value.length > longest.value.length ? option : longest).value;
    const modes: MurmurMode[] = Array.from({ length: 100 }, (_, index) => ({
      ...mode, id: '"'.repeat(123) + String(index).padStart(2, '0'), name: '"'.repeat(128),
      writingStyle: 'code_technical', cleanupEnabled: false, smartFormattingEnabled: false,
      cliFormattingEnabled: false, autoPaste: false, modelId, language,
    }));
    const settings: ModeExchangeSettings = {
      ...empty, modes,
      appProfiles: modes.map((item, index) => ({ ...profile,
        bundleId: 'com.' + 'x'.repeat(249) + String(index).padStart(2, '0'), modeId: item.id,
      })),
      browserSiteRules: Array.from({ length: 128 }, (_, index) => ({
        id: '"'.repeat(125) + String(index).padStart(3, '0'),
        browserBundleId: 'company.thebrowser.Browser',
        host: ['a'.repeat(63), 'b'.repeat(63), 'c'.repeat(63), 'd'.repeat(54) + String(index).padStart(3, '0'), 'com'].join('.'),
        modeId: modes[index % modes.length].id, enabled: false,
      })),
    };
    const contents = exportModeFile(settings);
    expect(new TextEncoder().encode(contents).length).toBeLessThanOrEqual(MAX_MODE_FILE_BYTES);
    expect(parseModeFile(contents).modes).toEqual(modes);
  });
  it('round trips every Mode policy into clean persisted settings without exporting private profile content or consent', () => {
    const json = exportModeFile(source);
    expect(json).not.toContain('/private/project');
    expect(json).not.toContain('siteModeLookupEnabled');
    expect(json).not.toContain('queryContextExcluded');
    const preview = previewModeImport(parseModeFile(json), empty);
    expect(preview.counts).toEqual({ modes: 1, bindings: 0, sites: 1, duplicates: 0, missingApps: 1 });
    const next = commitModeImport(preview, empty);
    expect(next.modes).toEqual([mode]);
    expect(next.siteModeLookupEnabled).toBe(false);
    localStorage.setItem('dictation-settings', JSON.stringify({ ...DEFAULT_SETTINGS, ...next }));
    expect(loadSettings().modes).toEqual([mode]);
    localStorage.clear();
  });
  it('exports only custom Mode bindings and site rules', () => {
    const json = exportModeFile({ ...source, modes: [BUILTIN_MODES[0], mode], appProfiles: [{ ...profile, modeId: BUILTIN_MODES[0].id }], browserSiteRules: [{ ...rule, modeId: BUILTIN_MODES[0].id }] });
    expect(parseModeFile(json)).toMatchObject({ modes: [mode], appBindings: [], browserSiteRules: [] });
    expect(json).not.toContain('builtin.');
  });
  it.each(BUILTIN_MODES)('rejects spoofing $id with builtIn false', (builtin) => {
    expectRejected({ modes: [{ ...mode, id: builtin.id }] });
  });
  it.each([
    { builtIn: true }, { enabled: 1 }, { autoPaste: 'true' }, { cleanupEnabled: undefined },
    { smartFormattingEnabled: {} }, { cliFormattingEnabled: 0 }, { writingStyle: 'inherit' },
    { modelId: 'unknown' }, { language: 'xx-not-real' }, { vocabularyPolicy: 'all' },
    { contextPolicy: 'cloud' }, { name: '' }, { id: 'builtin.future' }, { name: 'x'.repeat(129) },
    { name: 'é'.repeat(65) }, { name: '\ud800' }, { id: ' spaced ' }, { name: 'bad\nname' }, { secret: 'unknown field' },
  ])('rejects malformed Mode fields %j', (update) => expectRejected({ modes: [{ ...mode, ...update }] }));
  it.each([
    '{"version":1,"version":2}', '{"version":1,"\\u0076ersion":1}',
    '{/*comment*/"version":1}', '{"version":1,}', 'null', '[]', '',
    '['.repeat(10) + ']'.repeat(10),
  ])('rejects malformed or ambiguous JSON %s', (json) => expect(() => parseModeFile(json)).toThrow());
  it('rejects unsupported versions, extra keys and oversized inputs', () => {
    expectRejected({ version: 2 }); expectRejected({ format: 'other' }); expectRejected({ private: 'data' });
    expect(() => parseModeFile(' '.repeat(MAX_MODE_FILE_BYTES + 1))).toThrow('256 KiB');
    expectRejected({ modes: Array.from({ length: 101 }, () => mode) });
    expectRejected({ appBindings: Array.from({ length: 101 }, () => ({ bundleId: profile.bundleId, modeId: mode.id })) });
    expectRejected({ browserSiteRules: Array.from({ length: 129 }, () => rule) });
  });
  it.each([{ host: 'https://example.com/private' }, { host: 'Example.com' }, { browserBundleId: 'com.fake.Browser' }, { enabled: 1 }, { modeId: 'builtin.everyday' }])('rejects unsafe site rules %j', (update) => expectRejected({ browserSiteRules: [{ ...rule, ...update }] }));
  it('rejects dangling references and invalid bundle IDs', () => {
    expectRejected({ appBindings: [{ bundleId: profile.bundleId, modeId: 'mode.missing' }] });
    expectRejected({ appBindings: [{ bundleId: 'bad/id', modeId: mode.id }] });
  });
});

describe('Mode import merge', () => {
  it('skips exact duplicates, including field-order differences, and leaves the source untouched', () => {
    const before = JSON.stringify(source);
    const preview = previewModeImport(file(), source);
    expect(preview.kind).toBe('ready');
    expect(preview.counts).toEqual({ modes: 0, bindings: 0, sites: 0, duplicates: 3, missingApps: 0 });
    expect(commitModeImport(preview, source)).toEqual(source);
    expect(JSON.stringify(source)).toBe(before);
  });
  it('rejects the entire import for a conflicting Mode, binding, or site rule', () => {
    for (const existing of [
      { ...empty, modes: [{ ...mode, name: 'Other' }] },
      { ...empty, appProfiles: [{ ...profile, modeId: 'builtin.notes' }] },
      { ...empty, browserSiteRules: [{ ...rule, enabled: false }] },
      { ...empty, browserSiteRules: [{ ...rule, id: 'different-id' }] },
      { ...empty, appProfiles: [profile, profile] },
    ]) {
      const before = JSON.stringify(existing);
      const preview = previewModeImport(file(), existing);
      expect(preview.kind).toBe('conflict');
      expect(() => commitModeImport(preview, existing)).toThrow();
      expect(JSON.stringify(existing)).toBe(before);
    }
  });
  it('binds existing unbound apps while preserving all profile privacy and fine-tuning settings', () => {
    const existing = { ...empty, appProfiles: [profile] };
    const next = commitModeImport(previewModeImport(file(), existing), existing);
    expect(next.appProfiles).toEqual([{ ...profile, modeId: mode.id }]);
    expect(next.siteModeLookupEnabled).toBe(false);
  });
  it('rejects conflicting IDs within one file', () => {
    const data = file();
    const preview = previewModeImport({ ...data, modes: [mode, { ...mode, name: 'Different' }] }, empty);
    expect(preview.kind).toBe('conflict');
  });
  it('rejects merged limits before settings can silently truncate', () => {
    expect(previewModeImport(file(), { ...empty, modes: Array.from({ length: 100 }, (_, i) => ({ ...mode, id: `mode.${i}` })) }).kind).toBe('conflict');
    expect(previewModeImport(file(), { ...empty, browserSiteRules: Array.from({ length: 128 }, (_, i) => ({ ...rule, id: `site.${i}`, host: `host${i}.com` })) }).kind).toBe('conflict');
  });
  it('rejects previews after any imported setting or consent changes', () => {
    const preview = previewModeImport(file(), empty);
    for (const changed of [source, { ...empty, siteModeLookupEnabled: true }, { ...empty, appProfiles: [profile] }, { ...empty, browserSiteRules: [rule] }]) {
      expect(() => commitModeImport(preview, changed)).toThrow('changed');
    }
  });
});
