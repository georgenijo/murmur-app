/**
 * One-time migration for the retired Modes feature.
 *
 * Modes were reusable policies (writing style, boolean pipeline overrides,
 * model, language, project context) that an app profile or the global
 * `activeModeId` selected. They are gone, but a user's profiles must keep
 * behaving the same wherever a profile or a global setting has a home for the
 * value. This module reads the persisted legacy keys and folds each mode's
 * values into those homes, once.
 *
 * The legacy schema is frozen here on purpose: the parser keeps the exact
 * rejection rules the Mode sanitizer applied, and uses a copy of the built-in
 * mode table because Rust ignored persisted edits to built-ins. Nothing else
 * in the app reads Modes any more.
 *
 * Fields with no home are not migrated and are covered by the release notes:
 * per-mode model/language on profile-bound modes, vocabulary policy, CLI
 * formatting or writing style for apps with no profile, project context for an
 * unbound profile, and the fail-closed state of a profile bound to a missing or
 * disabled mode.
 */

export const LEGACY_MODE_KEYS = [
  'modes',
  'activeModeId',
  'siteModeLookupEnabled',
  'browserSiteRules',
] as const;

const EVERYDAY_MODE_ID = 'builtin.everyday';

type LegacyWritingStyle = 'conversational' | 'polished' | 'code_technical' | 'verbatim' | 'notes';

const LEGACY_WRITING_STYLES: ReadonlySet<string> = new Set<LegacyWritingStyle>([
  'conversational', 'polished', 'code_technical', 'verbatim', 'notes',
]);

// Frozen copies of the catalogs the legacy sanitizer validated against.
const LEGACY_MODEL_IDS: ReadonlySet<string> = new Set([
  'parakeet-tdt-0.6b-v3-coreml', 'tiny.en', 'base.en', 'small.en', 'medium.en',
  'large-v3-turbo', 'parakeet-tdt-0.6b-v2-fp16',
]);
const LEGACY_LANGUAGES: ReadonlySet<string> = new Set([
  'auto', 'en', 'es', 'fr', 'de', 'it', 'pt', 'nl', 'ja', 'ko', 'zh', 'ru', 'pl', 'tr', 'hi', 'ar',
]);

export interface LegacyMode {
  id: string;
  enabled: boolean;
  writingStyle: LegacyWritingStyle | null;
  cleanupEnabled: boolean | null;
  smartFormattingEnabled: boolean | null;
  cliFormattingEnabled: boolean | null;
  contextPolicy: 'none' | 'project';
  modelId: string | null;
  language: string | null;
  autoPaste: boolean | null;
}

const builtin = (
  id: string,
  writingStyle: LegacyWritingStyle | null,
  cliFormattingEnabled: boolean | null = null,
): LegacyMode => ({
  id, enabled: true, writingStyle, cleanupEnabled: null, smartFormattingEnabled: null,
  cliFormattingEnabled, contextPolicy: 'none', modelId: null, language: null, autoPaste: null,
});

const BUILTIN_MODES: readonly LegacyMode[] = [
  builtin(EVERYDAY_MODE_ID, null),
  builtin('builtin.messages', 'conversational'),
  builtin('builtin.email', 'polished'),
  builtin('builtin.notes', 'notes'),
  builtin('builtin.technical', 'code_technical'),
  builtin('builtin.terminal', 'code_technical', true),
  builtin('builtin.verbatim', 'verbatim'),
];

export interface LegacyModeState {
  /** True when the persisted document carried any legacy Mode key. */
  present: boolean;
  /** Valid user modes, after the legacy rejection rules. Built-ins excluded. */
  modes: LegacyMode[];
  /** Usable global mode id; Everyday when the stored one was missing or disabled. */
  activeModeId: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === 'object' && !Array.isArray(value);

/**
 * The retired Mode sanitizer, verbatim in its decisions: any invalid field
 * drops the whole mode, the first id wins, and built-in ids shadow custom ones.
 */
function parseLegacyModes(raw: unknown): LegacyMode[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set(BUILTIN_MODES.map((mode) => mode.id));
  const modes: LegacyMode[] = [];
  for (const value of raw.slice(0, 100)) {
    if (!isRecord(value)) continue;
    const id = typeof value.id === 'string' ? value.id.trim() : '';
    const name = typeof value.name === 'string' ? value.name.trim() : '';
    if (!id || !name || id.length > 128 || name.length > 128 || seen.has(id)) continue;
    if (value.builtIn === true) continue;
    if (value.modelId != null && (typeof value.modelId !== 'string' || !LEGACY_MODEL_IDS.has(value.modelId))) continue;
    if (value.language != null && (typeof value.language !== 'string' || !LEGACY_LANGUAGES.has(value.language))) continue;
    if (value.writingStyle != null
      && (typeof value.writingStyle !== 'string' || !LEGACY_WRITING_STYLES.has(value.writingStyle))) continue;
    if ([value.enabled, value.cleanupEnabled, value.smartFormattingEnabled, value.cliFormattingEnabled, value.autoPaste]
      .some((field) => field != null && typeof field !== 'boolean')) continue;
    if (value.vocabularyPolicy != null
      && !['inherit', 'general', 'technical'].includes(value.vocabularyPolicy as string)) continue;
    if (value.contextPolicy != null && !['none', 'project'].includes(value.contextPolicy as string)) continue;
    const nullableBoolean = (input: unknown) => (typeof input === 'boolean' ? input : null);
    modes.push({
      id,
      enabled: value.enabled !== false,
      writingStyle: typeof value.writingStyle === 'string' ? value.writingStyle as LegacyWritingStyle : null,
      cleanupEnabled: nullableBoolean(value.cleanupEnabled),
      smartFormattingEnabled: nullableBoolean(value.smartFormattingEnabled),
      cliFormattingEnabled: nullableBoolean(value.cliFormattingEnabled),
      contextPolicy: value.contextPolicy === 'project' ? 'project' : 'none',
      modelId: typeof value.modelId === 'string' ? value.modelId : null,
      language: typeof value.language === 'string' ? value.language : null,
      autoPaste: nullableBoolean(value.autoPaste),
    });
    seen.add(id);
  }
  return modes;
}

function usableMode(legacy: Pick<LegacyModeState, 'modes'>, id: string | null): LegacyMode | null {
  if (id === null) return null;
  return BUILTIN_MODES.find((mode) => mode.id === id)
    ?? legacy.modes.find((mode) => mode.id === id && mode.enabled)
    ?? null;
}

/** Read the legacy Mode keys out of a parsed settings document. */
export function readLegacyModeState(parsed: Record<string, unknown>): LegacyModeState {
  const has = (record: object, key: string) => Object.prototype.hasOwnProperty.call(record, key);
  const profileCarriesMode = Array.isArray(parsed.appProfiles)
    && parsed.appProfiles.some((profile) => isRecord(profile) && has(profile, 'modeId'));
  const present = profileCarriesMode || LEGACY_MODE_KEYS.some((key) => has(parsed, key));
  const modes = parseLegacyModes(parsed.modes);
  const activeModeId = typeof parsed.activeModeId === 'string'
    && usableMode({ modes }, parsed.activeModeId)
    ? parsed.activeModeId
    : EVERYDAY_MODE_ID;
  return { present, modes, activeModeId };
}

/** Delete every legacy Mode key from a parsed document, profiles included. */
export function stripLegacyModeKeys(parsed: Record<string, unknown>): void {
  for (const key of LEGACY_MODE_KEYS) delete parsed[key];
  if (Array.isArray(parsed.appProfiles)) {
    for (const profile of parsed.appProfiles) {
      if (isRecord(profile)) delete profile.modeId;
    }
  }
}

/** The installed model a global legacy mode would select, if any. */
export function legacyGlobalModelId(legacy: LegacyModeState): string | null {
  return usableMode(legacy, legacy.activeModeId)?.modelId ?? null;
}

export interface MigratableProfile {
  bundleId: string;
  autoPasteOverride: boolean | null;
  cleanupOverride: boolean | null;
  smartFormattingOverride: boolean | null;
  cliFormattingOverride: boolean | null;
  /** `'inherit'` is accepted as unset; the app never persists it. */
  writingStyle: string | null;
  ideContextEnabled: boolean;
  ideProjectRoots: string[];
  modeId?: string | null;
}

export interface MigratableSettings<P extends MigratableProfile = MigratableProfile> {
  model: string;
  language: string;
  autoPaste: boolean;
  cleanupEnabled: boolean;
  smartFormattingEnabled: boolean;
  appProfiles: P[];
}

export interface LegacyModeMigrationContext {
  /** Installed model ids. `null` when unknown: no model is adopted. */
  installedModelIds: ReadonlySet<string> | null;
}

const isStyleSet = (style: string | null): style is LegacyWritingStyle =>
  style !== null && style !== 'inherit' && LEGACY_WRITING_STYLES.has(style);

type BooleanOverride = 'autoPasteOverride' | 'cleanupOverride' | 'smartFormattingOverride' | 'cliFormattingOverride';

const FOLDED_OVERRIDES: ReadonlyArray<readonly [BooleanOverride, 'autoPaste' | 'cleanupEnabled' | 'smartFormattingEnabled' | 'cliFormattingEnabled']> = [
  ['autoPasteOverride', 'autoPaste'],
  ['cleanupOverride', 'cleanupEnabled'],
  ['smartFormattingOverride', 'smartFormattingEnabled'],
  ['cliFormattingOverride', 'cliFormattingEnabled'],
];

/**
 * Fold legacy Modes into profiles and globals. Pure; returns new settings whose
 * profiles no longer carry `modeId`. A no-op when `legacy.present` is false.
 *
 * Resolution matches the retired resolver: the first profile for a bundle id
 * selects the mode (`profile.modeId`, else the global mode; an unusable id folds
 * nothing and does not fall back), and each field comes from the first profile
 * for that bundle that sets it.
 */
export function applyLegacyModes<P extends MigratableProfile>(
  input: MigratableSettings<P>,
  legacy: LegacyModeState,
  context: LegacyModeMigrationContext,
): MigratableSettings<P> {
  const withoutModeIds = input.appProfiles.map((profile) => {
    const copy = { ...profile };
    delete copy.modeId;
    return copy;
  });
  if (!legacy.present) return { ...input, appProfiles: withoutModeIds as P[] };

  // Group by bundle id. The first profile of a group selects the mode and owns
  // project context; `withoutModeIds` is index-aligned with the input.
  interface Group { first: P; members: P[]; mode: LegacyMode | null; bound: boolean }
  const groups = new Map<string, Group>();
  input.appProfiles.forEach((original, index) => {
    const profile = withoutModeIds[index] as P;
    const group = groups.get(profile.bundleId);
    if (group) {
      group.members.push(profile);
      return;
    }
    const boundId = typeof original.modeId === 'string' && original.modeId.trim()
      ? original.modeId.trim()
      : null;
    // A bound profile with an unusable id folds nothing; only an unbound
    // profile follows the global mode.
    const mode = usableMode(legacy, boundId ?? legacy.activeModeId);
    groups.set(profile.bundleId, { first: profile, members: [profile], mode, bound: boundId !== null });
  });

  // 1. Fold each bundle's mode into its profiles, against the original globals.
  for (const { first, members, mode, bound } of groups.values()) {
    if (!mode || mode.id === EVERYDAY_MODE_ID) continue;
    if (mode.writingStyle !== null && !members.some((profile) => isStyleSet(profile.writingStyle))) {
      first.writingStyle = mode.writingStyle;
    }
    for (const [override, field] of FOLDED_OVERRIDES) {
      const value = mode[field];
      if (value !== null && !members.some((profile) => profile[override] !== null)) {
        first[override] = value;
      }
    }
    // The retired resolver also enabled context for an unbound profile under a
    // global project mode, but nothing ever scanned its roots; that is a
    // release note, not a value to invent.
    if (bound && mode.contextPolicy === 'project'
      && first.ideProjectRoots.length > 0 && !first.ideContextEnabled) {
      first.ideContextEnabled = true;
    }
  }

  // 2. Before the globals move, pin every profile whose result they would
  //    change, using the ORIGINAL global value. A set writing style decides
  //    cleanup and smart formatting itself, so those profiles need no pin.
  const globalMode = usableMode(legacy, legacy.activeModeId);
  const adopted = globalMode && globalMode.id !== EVERYDAY_MODE_ID ? globalMode : null;
  if (adopted) {
    const pinnable = [
      ['autoPasteOverride', adopted.autoPaste, input.autoPaste, false],
      ['cleanupOverride', adopted.cleanupEnabled, input.cleanupEnabled, true],
      ['smartFormattingOverride', adopted.smartFormattingEnabled, input.smartFormattingEnabled, true],
    ] as const;
    for (const [override, adoptedValue, originalValue, styleDecides] of pinnable) {
      if (adoptedValue === null || adoptedValue === originalValue) continue;
      for (const { first, members, mode } of groups.values()) {
        if (mode?.id === adopted.id) continue;
        if (members.some((profile) => profile[override] !== null)) continue;
        if (styleDecides && members.some((profile) => isStyleSet(profile.writingStyle))) continue;
        first[override] = originalValue;
      }
    }
  }

  // 3. Move the globals that have an exact home.
  const next: MigratableSettings<P> = { ...input, appProfiles: withoutModeIds as P[] };
  if (adopted) {
    if (adopted.autoPaste !== null) next.autoPaste = adopted.autoPaste;
    if (adopted.cleanupEnabled !== null) next.cleanupEnabled = adopted.cleanupEnabled;
    if (adopted.smartFormattingEnabled !== null) next.smartFormattingEnabled = adopted.smartFormattingEnabled;
    if (adopted.modelId !== null && context.installedModelIds?.has(adopted.modelId)) next.model = adopted.modelId;
    if (adopted.language !== null) next.language = adopted.language;
  }
  return next;
}
