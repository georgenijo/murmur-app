import { presentationStorage } from './presentationStorage';
import { sourceStorage, persistSourceState } from './webkitLifecycle';
import type { DictationStats } from './stats';
import type { Settings } from './settings';

export const DISCOVERY_STORAGE_KEY = 'murmur_discovery_v1';
export const DISCOVERY_CHANGED_EVENT = 'murmur-discovery-changed';

export const CHECKLIST_ITEM_IDS = [
  'command_palette',
  'mode_binding',
  'meeting',
  'correction',
  'shortcuts',
] as const;

export type ChecklistItemId = (typeof CHECKLIST_ITEM_IDS)[number];

export const DISCOVERY_HINT_IDS = [
  'double_tap_timing',
  'correct_and_teach',
  'browser_modes',
] as const;

export type DiscoveryHintId = (typeof DISCOVERY_HINT_IDS)[number];

export type DiscoveryDestination =
  | { kind: 'palette' }
  | { kind: 'settings'; page: string; target: string }
  | { kind: 'main'; page: 'meetings' }
  | { kind: 'teach' };

export function checklistDestination(id: ChecklistItemId): DiscoveryDestination {
  switch (id) {
    case 'command_palette': return { kind: 'palette' };
    case 'mode_binding': return { kind: 'settings', page: 'modes', target: 'modes' };
    case 'meeting': return { kind: 'main', page: 'meetings' };
    case 'correction': return { kind: 'teach' };
    case 'shortcuts': return { kind: 'settings', page: 'shortcuts', target: 'shortcuts' };
    default: {
      const exhaustive: never = id;
      return exhaustive;
    }
  }
}

export function hintDestination(id: DiscoveryHintId): DiscoveryDestination {
  switch (id) {
    case 'double_tap_timing': return { kind: 'settings', page: 'recording', target: 'hotkey-feedback' };
    case 'correct_and_teach': return { kind: 'teach' };
    case 'browser_modes': return { kind: 'settings', page: 'modes', target: 'modes' };
    default: {
      const exhaustive: never = id;
      return exhaustive;
    }
  }
}

export interface DiscoveryEvidence {
  transforms: number;
  meetings: number;
  correctionsTaught: number;
  modeBindings: string[];
}

interface DiscoveryStateV1 {
  version: 1;
  checklistDismissed: boolean;
  completed: ChecklistItemId[];
  baseline: DiscoveryEvidence;
  pendingHints: DiscoveryHintId[];
  dismissedHints: DiscoveryHintId[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string');
}

function isChecklistItemId(value: string): value is ChecklistItemId {
  return CHECKLIST_ITEM_IDS.some((id) => id === value);
}

function isHintId(value: string): value is DiscoveryHintId {
  return DISCOVERY_HINT_IDS.some((id) => id === value);
}

function unique<T extends string>(values: T[]): T[] {
  return [...new Set(values)];
}

function parseState(value: unknown): DiscoveryStateV1 | null {
  if (!isRecord(value) || value.version !== 1 || typeof value.checklistDismissed !== 'boolean'
    || !isStringArray(value.completed)
    || !isRecord(value.baseline) || !isCount(value.baseline.transforms)
    || !isCount(value.baseline.meetings) || !isCount(value.baseline.correctionsTaught)
    || !isStringArray(value.baseline.modeBindings)
    || !isStringArray(value.pendingHints) || !value.pendingHints.every(isHintId)
    || !isStringArray(value.dismissedHints) || !value.dismissedHints.every(isHintId)) return null;
  const dismissedHints = unique(value.dismissedHints);
  const pendingHints = unique(value.pendingHints).filter((hint) => !dismissedHints.includes(hint));
  return {
    version: 1,
    checklistDismissed: value.checklistDismissed,
    completed: unique(value.completed.filter(isChecklistItemId)),
    baseline: {
      transforms: value.baseline.transforms,
      meetings: value.baseline.meetings,
      correctionsTaught: value.baseline.correctionsTaught,
      modeBindings: unique(value.baseline.modeBindings),
    },
    pendingHints,
    dismissedHints,
  };
}

export function discoveryEvidence(
  settings: Pick<Settings, 'appProfiles'>,
  stats: DictationStats,
): DiscoveryEvidence {
  return {
    transforms: stats.activity.transforms.runs,
    meetings: stats.activity.meetings.count,
    correctionsTaught: stats.activity.corrections.taught,
    modeBindings: settings.appProfiles
      .filter((profile) => typeof profile.modeId === 'string' && profile.modeId.length > 0)
      .map((profile) => `${profile.bundleId}:${profile.modeId}`)
      .sort(),
  };
}

export function loadDiscoveryState(): DiscoveryStateV1 | null {
  try {
    const stored = presentationStorage.getItem(DISCOVERY_STORAGE_KEY);
    return stored === null ? null : parseState(JSON.parse(stored));
  } catch {
    return null;
  }
}

function saveState(state: DiscoveryStateV1): DiscoveryStateV1 {
  try {
    sourceStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(state));
    window.dispatchEvent(new Event(DISCOVERY_CHANGED_EVENT));
  } catch {
    // Discovery never blocks the main app when storage is unavailable.
  }
  return state;
}

function initialState(evidence: DiscoveryEvidence, completed: ChecklistItemId[]): DiscoveryStateV1 {
  return {
    version: 1,
    checklistDismissed: false,
    completed,
    baseline: {
      transforms: evidence.transforms,
      meetings: evidence.meetings,
      correctionsTaught: evidence.correctionsTaught,
      modeBindings: evidence.modeBindings,
    },
    pendingHints: [],
    dismissedHints: [],
  };
}

/** A completed setup starts a new practice cycle from the evidence already present. */
export function initializeDiscoveryAfterSetup(evidence: DiscoveryEvidence): DiscoveryStateV1 {
  return saveState(initialState(evidence, []));
}

/** Existing installs retain proof from local usage and explicit configuration. */
export function initializeGrandfatheredDiscovery(evidence: DiscoveryEvidence): DiscoveryStateV1 {
  const existing = loadDiscoveryState();
  if (existing) return existing;
  const completed: ChecklistItemId[] = [];
  if (evidence.meetings > 0) completed.push('meeting');
  if (evidence.correctionsTaught > 0) completed.push('correction');
  if (evidence.modeBindings.length > 0) completed.push('mode_binding');
  return saveState(initialState(evidence, completed));
}

export function syncDiscoveryEvidence(evidence: DiscoveryEvidence): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  if (!state) return null;
  const baseline = {
    ...state.baseline,
    transforms: Math.min(state.baseline.transforms, evidence.transforms),
    meetings: Math.min(state.baseline.meetings, evidence.meetings),
    correctionsTaught: Math.min(state.baseline.correctionsTaught, evidence.correctionsTaught),
  };
  const completed = new Set(state.completed);
  if (evidence.meetings > baseline.meetings) completed.add('meeting');
  if (evidence.correctionsTaught > baseline.correctionsTaught) completed.add('correction');
  if (evidence.modeBindings.some((binding) => !state.baseline.modeBindings.includes(binding))) {
    completed.add('mode_binding');
  }
  const baselineChanged = baseline.transforms !== state.baseline.transforms
    || baseline.meetings !== state.baseline.meetings
    || baseline.correctionsTaught !== state.baseline.correctionsTaught;
  if (completed.size === state.completed.length && !baselineChanged) return state;
  return saveState({ ...state, baseline, completed: [...completed] });
}

export function completeChecklistItem(id: ChecklistItemId): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  if (!state || state.completed.includes(id)) return state;
  return saveState({ ...state, completed: [...state.completed, id] });
}

export function dismissDiscoveryChecklist(): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  return state ? saveState({ ...state, checklistDismissed: true }) : null;
}

export function reopenDiscoveryChecklist(): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  return state ? saveState({ ...state, checklistDismissed: false }) : null;
}

export function enqueueDiscoveryHint(id: DiscoveryHintId): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  if (!state || state.pendingHints.includes(id) || state.dismissedHints.includes(id)) return state;
  return saveState({ ...state, pendingHints: [...state.pendingHints, id] });
}

export function dismissDiscoveryHint(id: DiscoveryHintId): DiscoveryStateV1 | null {
  const state = loadDiscoveryState();
  if (!state || state.dismissedHints.includes(id)) return state;
  return saveState({
    ...state,
    pendingHints: state.pendingHints.filter((pending) => pending !== id),
    dismissedHints: [...state.dismissedHints, id],
  });
}

export function resetDiscovery(): void {
  try {
    persistSourceState(DISCOVERY_STORAGE_KEY, 'null');
    presentationStorage.removeItem(DISCOVERY_STORAGE_KEY);
    window.dispatchEvent(new Event(DISCOVERY_CHANGED_EVENT));
  } catch {
    // Non-fatal.
  }
}
