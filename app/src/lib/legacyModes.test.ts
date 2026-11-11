import { describe, expect, it } from 'vitest';
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import legacyMatrix from './fixtures/modes-retirement/legacy-matrix.json';
import migratedMatrix from './fixtures/modes-retirement/migrated-matrix.json';
import {
  applyLegacyModes,
  legacyGlobalModelId,
  readLegacyModeState,
  stripLegacyModeKeys,
  type MigratableProfile,
  type MigratableSettings,
} from './legacyModes';

type Profile = MigratableProfile & { label: string };

const profile = (bundleId: string, overrides: Partial<Profile> = {}): Profile => ({
  bundleId,
  label: bundleId,
  autoPasteOverride: null,
  cleanupOverride: null,
  smartFormattingOverride: null,
  cliFormattingOverride: null,
  writingStyle: null,
  ideContextEnabled: false,
  ideProjectRoots: [],
  ...overrides,
});

const mode = (id: string, overrides: Record<string, unknown> = {}) => ({
  id, name: id, builtIn: false, enabled: true, writingStyle: null, cleanupEnabled: null,
  smartFormattingEnabled: null, cliFormattingEnabled: null, vocabularyPolicy: 'inherit',
  contextPolicy: 'none', modelId: null, language: null, autoPaste: null, ...overrides,
});

const globals = (overrides: Partial<MigratableSettings<Profile>> = {}): MigratableSettings<Profile> => ({
  model: 'tiny.en', language: 'en', autoPaste: true, cleanupEnabled: false,
  smartFormattingEnabled: false, appProfiles: [], ...overrides,
});

interface Doc {
  modes?: unknown[];
  activeModeId?: string;
  siteModeLookupEnabled?: boolean;
  browserSiteRules?: unknown[];
}

function migrate(
  settings: MigratableSettings<Profile>,
  doc: Doc = {},
  installed: string[] | null = [],
) {
  const legacy = readLegacyModeState({
    modes: [], activeModeId: 'builtin.everyday', ...doc,
    appProfiles: settings.appProfiles,
  });
  return applyLegacyModes(settings, legacy, {
    installedModelIds: installed === null ? null : new Set(installed),
  });
}

const only = (result: MigratableSettings<Profile>) => result.appProfiles[0];

describe('legacy Mode parser', () => {
  it('reuses the retired sanitizer rejection rules', () => {
    const valid = mode('custom.ok', { writingStyle: 'notes', modelId: 'small.en', language: 'de' });
    const state = readLegacyModeState({
      modes: [
        valid,
        mode('custom.badstyle', { writingStyle: 'inherit' }),
        mode('custom.badmodel', { modelId: 'gpt-4' }),
        mode('custom.badlang', { language: 'xx' }),
        mode('custom.badbool', { autoPaste: 'yes' }),
        mode('custom.badvocab', { vocabularyPolicy: 'loud' }),
        mode('custom.badcontext', { contextPolicy: 'always' }),
        mode('builtin.email', { writingStyle: 'verbatim' }),
        mode('custom.claimsbuiltin', { builtIn: true }),
        mode('  ', {}),
        { id: 'custom.noname' },
        mode('custom.ok', { writingStyle: 'verbatim' }),
        'junk',
      ],
    });
    expect(state.modes.map((entry) => entry.id)).toEqual(['custom.ok']);
    expect(state.modes[0]).toMatchObject({ writingStyle: 'notes', modelId: 'small.en', language: 'de' });
  });

  it('bounds the list to 100 modes and 128 characters per id or name', () => {
    const many = Array.from({ length: 101 }, (_, index) => mode(`custom.${index}`));
    expect(readLegacyModeState({ modes: many }).modes).toHaveLength(100);
    expect(readLegacyModeState({ modes: [mode('x'.repeat(129))] }).modes).toHaveLength(0);
    expect(readLegacyModeState({ modes: [mode('custom.a', { name: 'n'.repeat(129) })] }).modes).toHaveLength(0);
  });

  it('keeps the first id and lets built-ins shadow custom modes', () => {
    const state = readLegacyModeState({
      modes: [mode('custom.a', { autoPaste: true }), mode('custom.a', { autoPaste: false })],
    });
    expect(state.modes).toHaveLength(1);
    expect(state.modes[0].autoPaste).toBe(true);
  });

  it('falls back to Everyday for a missing, disabled, or non-string active mode', () => {
    const modes = [mode('custom.off', { enabled: false }), mode('custom.on')];
    expect(readLegacyModeState({ modes, activeModeId: 'custom.on' }).activeModeId).toBe('custom.on');
    expect(readLegacyModeState({ modes, activeModeId: 'custom.off' }).activeModeId).toBe('builtin.everyday');
    expect(readLegacyModeState({ modes, activeModeId: 'custom.gone' }).activeModeId).toBe('builtin.everyday');
    expect(readLegacyModeState({ modes, activeModeId: 7 }).activeModeId).toBe('builtin.everyday');
    expect(readLegacyModeState({ modes, activeModeId: 'builtin.terminal' }).activeModeId).toBe('builtin.terminal');
  });

  it('reports whether any legacy key is present', () => {
    expect(readLegacyModeState({ model: 'tiny.en' }).present).toBe(false);
    for (const key of ['modes', 'activeModeId', 'siteModeLookupEnabled', 'browserSiteRules']) {
      expect(readLegacyModeState({ [key]: key === 'activeModeId' ? 'builtin.everyday' : [] }).present).toBe(true);
    }
    expect(readLegacyModeState({ appProfiles: [{ bundleId: 'a', modeId: 'builtin.email' }] }).present).toBe(true);
    expect(readLegacyModeState({ appProfiles: [{ bundleId: 'a' }] }).present).toBe(false);
  });

  it('strips every legacy key including profile bindings', () => {
    const doc: Record<string, unknown> = {
      model: 'tiny.en', modes: [], activeModeId: 'builtin.email', siteModeLookupEnabled: true, browserSiteRules: [],
      appProfiles: [{ bundleId: 'a', modeId: 'builtin.email', label: 'A' }],
    };
    stripLegacyModeKeys(doc);
    expect(doc).toEqual({ model: 'tiny.en', appProfiles: [{ bundleId: 'a', label: 'A' }] });
  });
});

describe('writing style (rule 1)', () => {
  it('copies the bound mode style into a null profile style', () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.email' })] }));
    expect(only(result).writingStyle).toBe('polished');
  });

  it('treats inherit as unset', () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.messages', writingStyle: 'inherit' })] }));
    expect(only(result).writingStyle).toBe('conversational');
  });

  it("keeps the profile's own style", () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.email', writingStyle: 'notes' })] }));
    expect(only(result).writingStyle).toBe('notes');
  });

  it('writes nothing when the mode has no style', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { modeId: 'custom.plain' })] }),
      { modes: [mode('custom.plain')] },
    );
    expect(only(result).writingStyle).toBeNull();
  });

  it('lets a later duplicate that sets a style stand and fills only the first profile otherwise', () => {
    const supplied = migrate(globals({
      appProfiles: [profile('a', { modeId: 'builtin.messages' }), profile('a', { writingStyle: 'verbatim' })],
    }));
    expect(supplied.appProfiles.map((entry) => entry.writingStyle)).toEqual([null, 'verbatim']);
    const absent = migrate(globals({
      appProfiles: [profile('a', { modeId: 'builtin.messages' }), profile('a', { writingStyle: 'inherit' })],
    }));
    expect(absent.appProfiles.map((entry) => entry.writingStyle)).toEqual(['conversational', 'inherit']);
  });

  it('takes the mode from the first profile of a bundle only', () => {
    const result = migrate(globals({
      appProfiles: [profile('a', { modeId: 'builtin.email' }), profile('a', { modeId: 'builtin.verbatim' })],
    }));
    expect(result.appProfiles.map((entry) => entry.writingStyle)).toEqual(['polished', null]);
  });
});

describe('boolean overrides (rule 2)', () => {
  const cli = mode('custom.flags', {
    cleanupEnabled: false, smartFormattingEnabled: true, cliFormattingEnabled: false, autoPaste: true,
  });

  it('copies each non-null mode value into a null profile override', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { modeId: 'custom.flags' })] }),
      { modes: [cli] },
    );
    expect(only(result)).toMatchObject({
      autoPasteOverride: true, cleanupOverride: false, smartFormattingOverride: true, cliFormattingOverride: false,
    });
  });

  it('never overwrites a profile value, including false', () => {
    const result = migrate(
      globals({
        appProfiles: [profile('a', {
          modeId: 'custom.flags', autoPasteOverride: false, cleanupOverride: true,
          smartFormattingOverride: false, cliFormattingOverride: true,
        })],
      }),
      { modes: [cli] },
    );
    expect(only(result)).toMatchObject({
      autoPasteOverride: false, cleanupOverride: true, smartFormattingOverride: false, cliFormattingOverride: true,
    });
  });

  it('leaves a field alone when the mode value is null', () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.email' })] }));
    expect(only(result)).toMatchObject({
      autoPasteOverride: null, cleanupOverride: null, smartFormattingOverride: null, cliFormattingOverride: null,
    });
  });

  it('carries Terminal forced CLI formatting', () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.terminal' })] }));
    expect(only(result).cliFormattingOverride).toBe(true);
  });

  it('uses first-match precedence across duplicate profiles', () => {
    const result = migrate(
      globals({
        appProfiles: [
          profile('a', { modeId: 'custom.flags' }),
          profile('a', { cleanupOverride: true }),
          profile('b', { modeId: 'custom.flags', autoPasteOverride: false }),
          profile('b'),
        ],
      }),
      { modes: [cli] },
    );
    // Bundle a: a later duplicate supplies cleanup, so the first one is not filled for it.
    expect(result.appProfiles[0]).toMatchObject({ cleanupOverride: null, autoPasteOverride: true });
    expect(result.appProfiles[1].cleanupOverride).toBe(true);
    // Bundle b: the first profile already sets auto paste; the second is never filled.
    expect(result.appProfiles[2].autoPasteOverride).toBe(false);
    expect(result.appProfiles[3]).toMatchObject({ autoPasteOverride: null, cleanupOverride: null });
  });

  it('writes nothing for an Everyday binding', () => {
    const input = globals({ appProfiles: [profile('a', { modeId: 'builtin.everyday' })] });
    const result = migrate(input);
    expect(only(result)).toEqual(profile('a'));
  });
});

describe('project context (rule 3)', () => {
  const project = mode('custom.project', { contextPolicy: 'project', writingStyle: 'code_technical' });

  it('enables context for a bound project mode with roots', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { modeId: 'custom.project', ideProjectRoots: ['/tmp/a'] })] }),
      { modes: [project] },
    );
    expect(only(result).ideContextEnabled).toBe(true);
  });

  it('leaves a profile without roots unchanged', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { modeId: 'custom.project' })] }),
      { modes: [project] },
    );
    expect(only(result).ideContextEnabled).toBe(false);
    expect(only(result).writingStyle).toBe('code_technical');
  });

  it('writes nothing for an unbound profile under a global project mode', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { ideProjectRoots: ['/tmp/a'] })] }),
      { modes: [project], activeModeId: 'custom.project' },
    );
    expect(only(result).ideContextEnabled).toBe(false);
  });

  it('ignores context for built-in and non-project modes', () => {
    const result = migrate(globals({
      appProfiles: [profile('a', { modeId: 'builtin.technical', ideProjectRoots: ['/tmp/a'] })],
    }));
    expect(only(result).ideContextEnabled).toBe(false);
  });

  it('applies context from the first duplicate profile only', () => {
    const result = migrate(
      globals({
        appProfiles: [
          profile('a', { modeId: 'custom.project' }),
          profile('a', { ideProjectRoots: ['/tmp/a'] }),
        ],
      }),
      { modes: [project] },
    );
    expect(result.appProfiles.map((entry) => entry.ideContextEnabled)).toEqual([false, false]);
  });
});

describe('unusable modes', () => {
  it('folds nothing for a disabled custom mode and does not fall back to the global mode', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { modeId: 'custom.off' })] }),
      {
        modes: [mode('custom.off', { enabled: false, writingStyle: 'verbatim', autoPaste: false }),
          mode('custom.global', { writingStyle: 'polished', autoPaste: false })],
        activeModeId: 'custom.global',
      },
    );
    expect(only(result)).toEqual(profile('a', { autoPasteOverride: true }));
  });

  it('folds nothing for a missing custom mode or a non-existent built-in id', () => {
    for (const modeId of ['custom.gone', 'builtin.nope']) {
      const result = migrate(globals({ appProfiles: [profile('a', { modeId })] }));
      expect(only(result)).toEqual(profile('a'));
    }
  });

  it('resets a disabled global mode to Everyday and migrates nothing', () => {
    const input = globals({ appProfiles: [profile('a')] });
    const result = migrate(input, {
      modes: [mode('custom.off', { enabled: false, autoPaste: false, writingStyle: 'verbatim' })],
      activeModeId: 'custom.off',
    });
    expect(result).toEqual(input);
  });
});

describe('unbound profiles under a non-Everyday global mode', () => {
  it('follow the global mode like the retired resolver', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a')] }),
      { activeModeId: 'builtin.terminal' },
    );
    expect(only(result)).toMatchObject({ writingStyle: 'code_technical', cliFormattingOverride: true });
  });

  it('keep their own values over the global mode', () => {
    const result = migrate(
      globals({ appProfiles: [profile('a', { writingStyle: 'verbatim', cliFormattingOverride: false })] }),
      { activeModeId: 'builtin.terminal' },
    );
    expect(only(result)).toMatchObject({ writingStyle: 'verbatim', cliFormattingOverride: false });
  });
});

describe('global exact-fit fields', () => {
  const g = mode('custom.g', {
    autoPaste: false, cleanupEnabled: true, smartFormattingEnabled: true, modelId: 'small.en', language: 'de',
    writingStyle: 'polished', cliFormattingEnabled: true, vocabularyPolicy: 'technical',
  });

  it('moves auto paste, cleanup, smart formatting, language, and an installed model', () => {
    const result = migrate(globals(), { modes: [g], activeModeId: 'custom.g' }, ['small.en']);
    expect(result).toMatchObject({
      autoPaste: false, cleanupEnabled: true, smartFormattingEnabled: true, model: 'small.en', language: 'de',
    });
  });

  it('leaves the model alone when it is not installed or installation is unknown', () => {
    for (const installed of [[], ['tiny.en'], null]) {
      const result = migrate(globals(), { modes: [g], activeModeId: 'custom.g' }, installed);
      expect(result.model).toBe('tiny.en');
      expect(result.language).toBe('de');
    }
  });

  it('copies only the fields the mode sets', () => {
    const result = migrate(
      globals({ autoPaste: true, cleanupEnabled: true }),
      { modes: [mode('custom.sparse', { smartFormattingEnabled: true })], activeModeId: 'custom.sparse' },
    );
    expect(result).toMatchObject({
      autoPaste: true, cleanupEnabled: true, smartFormattingEnabled: true, model: 'tiny.en', language: 'en',
    });
  });

  it('does not touch globals for built-in global modes', () => {
    const input = globals();
    const result = migrate(input, { activeModeId: 'builtin.email' });
    expect(result).toMatchObject({
      autoPaste: input.autoPaste, cleanupEnabled: input.cleanupEnabled, model: input.model, language: input.language,
    });
  });

  it('exposes the global model for installed-model lookup', () => {
    const state = readLegacyModeState({ modes: [g], activeModeId: 'custom.g' });
    expect(legacyGlobalModelId(state)).toBe('small.en');
    expect(legacyGlobalModelId(readLegacyModeState({ modes: [g] }))).toBeNull();
  });
});

describe('pinning against the original globals', () => {
  const g = mode('custom.g', { autoPaste: false, cleanupEnabled: true, smartFormattingEnabled: false });
  const doc = { modes: [g, mode('custom.other', { autoPaste: true })], activeModeId: 'custom.g' };
  const base = { autoPaste: true, cleanupEnabled: false, smartFormattingEnabled: true };

  it('pins an Everyday-bound profile to the old global values', () => {
    const result = migrate(
      globals({ ...base, appProfiles: [profile('a', { modeId: 'builtin.everyday' })] }),
      doc,
    );
    expect(only(result)).toMatchObject({
      autoPasteOverride: true, cleanupOverride: false, smartFormattingOverride: true,
    });
    expect(result).toMatchObject({ autoPaste: false, cleanupEnabled: true, smartFormattingEnabled: false });
  });

  it('pins a profile bound to an invalid mode to the old global values', () => {
    const result = migrate(globals({ ...base, appProfiles: [profile('a', { modeId: 'custom.gone' })] }), doc);
    expect(only(result)).toMatchObject({
      autoPasteOverride: true, cleanupOverride: false, smartFormattingOverride: true,
    });
  });

  it('does not pin cleanup or smart formatting where a writing style decides them', () => {
    const result = migrate(
      globals({ ...base, appProfiles: [profile('a', { modeId: 'builtin.verbatim' })] }),
      doc,
    );
    expect(only(result)).toMatchObject({
      writingStyle: 'verbatim', autoPasteOverride: true, cleanupOverride: null, smartFormattingOverride: null,
    });
  });

  it('pins only what a bound custom mode leaves to the global', () => {
    const result = migrate(
      globals({ ...base, appProfiles: [profile('a', { modeId: 'custom.other' })] }),
      doc,
    );
    expect(only(result)).toMatchObject({
      autoPasteOverride: true, cleanupOverride: false, smartFormattingOverride: true,
    });
  });

  it('does not pin profiles that follow the global mode itself', () => {
    const result = migrate(
      globals({ ...base, appProfiles: [profile('a', { modeId: 'custom.g' }), profile('b')] }),
      doc,
    );
    expect(result.appProfiles.map((entry) => entry.autoPasteOverride)).toEqual([false, false]);
  });

  it("does not overwrite a profile's own override", () => {
    const result = migrate(
      globals({ ...base, appProfiles: [profile('a', { modeId: 'builtin.everyday', autoPasteOverride: false })] }),
      doc,
    );
    expect(only(result).autoPasteOverride).toBe(false);
  });

  it('skips pins for fields whose global does not change', () => {
    const result = migrate(
      globals({ ...base, autoPaste: false, appProfiles: [profile('a', { modeId: 'builtin.everyday' })] }),
      doc,
    );
    expect(only(result).autoPasteOverride).toBeNull();
  });

  it('pins one profile per bundle and honors duplicate first-match order', () => {
    const result = migrate(
      globals({
        ...base,
        appProfiles: [
          profile('a', { modeId: 'builtin.everyday' }),
          profile('a', { autoPasteOverride: false }),
        ],
      }),
      doc,
    );
    expect(result.appProfiles.map((entry) => entry.autoPasteOverride)).toEqual([null, false]);
  });
});

describe('idempotence and no-ops', () => {
  it('is a no-op when no legacy key is present', () => {
    const input = globals({ appProfiles: [profile('a', { autoPasteOverride: false })] });
    const legacy = readLegacyModeState({ appProfiles: input.appProfiles });
    expect(legacy.present).toBe(false);
    expect(applyLegacyModes(input, legacy, { installedModelIds: new Set() })).toEqual(input);
  });

  it('migrates once: the stripped output has nothing left to migrate and re-running changes nothing', () => {
    const document: Record<string, unknown> = {
      ...globals({
        appProfiles: [
          profile('a', { modeId: 'builtin.email' }),
          profile('b', { modeId: 'builtin.everyday' }),
          profile('c'),
        ],
      }),
      modes: [mode('custom.g', { autoPaste: false, cleanupEnabled: true, language: 'de', modelId: 'small.en' })],
      activeModeId: 'custom.g',
      siteModeLookupEnabled: true,
      browserSiteRules: [],
    };
    const legacy = readLegacyModeState(document);
    const first = applyLegacyModes(document as unknown as MigratableSettings<Profile>, legacy, {
      installedModelIds: new Set(['small.en']),
    });
    const persisted: Record<string, unknown> = { ...document, ...first };
    stripLegacyModeKeys(persisted);

    const again = readLegacyModeState(persisted);
    expect(again.present).toBe(false);
    const second = applyLegacyModes(persisted as unknown as MigratableSettings<Profile>, again, {
      installedModelIds: new Set(['small.en']),
    });
    expect(second).toEqual(persisted);
    expect(second.appProfiles).toEqual(first.appProfiles);
  });

  it('does not mutate its input', () => {
    const input = globals({ appProfiles: [profile('a', { modeId: 'builtin.email' })] });
    const snapshot = JSON.stringify(input);
    migrate(input);
    expect(JSON.stringify(input)).toBe(snapshot);
  });

  it('drops profile mode ids even when the profile folds nothing', () => {
    const result = migrate(globals({ appProfiles: [profile('a', { modeId: 'builtin.everyday' })] }));
    expect('modeId' in only(result)).toBe(false);
  });
});

// ── Fixture matrix ──────────────────────────────────────────────────────────

interface MatrixCase {
  id: string;
  installedModels: string[];
  settings: MigratableSettings<Profile> & { voiceCommandsEnabled: boolean };
}

function migrateMatrixCase(entry: MatrixCase) {
  const settings = structuredClone(entry.settings);
  const legacy = readLegacyModeState(settings as unknown as Record<string, unknown>);
  const result = applyLegacyModes(settings, legacy, {
    installedModelIds: new Set(entry.installedModels),
  });
  return {
    id: entry.id,
    settings: {
      model: result.model,
      language: result.language,
      autoPaste: result.autoPaste,
      cleanupEnabled: result.cleanupEnabled,
      smartFormattingEnabled: result.smartFormattingEnabled,
      voiceCommandsEnabled: entry.settings.voiceCommandsEnabled,
      appProfiles: result.appProfiles,
    },
  };
}

describe('legacy settings matrix', () => {
  const cases = (legacyMatrix as unknown as { cases: MatrixCase[] }).cases;

  it('migrates to the committed migrated-settings fixture', () => {
    const actual = { version: 1, cases: cases.map(migrateMatrixCase) };
    if (process.env.MURMUR_UPDATE_MODES_FIXTURES) {
      writeFileSync(
        resolve(process.cwd(), 'src/lib/fixtures/modes-retirement/migrated-matrix.json'),
        `${JSON.stringify(actual, null, 2)}\n`,
      );
      return;
    }
    expect(actual).toEqual(migratedMatrix);
  });

  it('leaves no legacy key behind and migrates a second time without change', () => {
    for (const entry of cases) {
      const migrated = migrateMatrixCase(entry);
      expect(JSON.stringify(migrated)).not.toContain('modeId');
      const again = migrateMatrixCase({
        id: entry.id,
        installedModels: entry.installedModels,
        settings: { ...(migrated.settings as object) } as MatrixCase['settings'],
      });
      expect(again).toEqual(migrated);
    }
  });
});
