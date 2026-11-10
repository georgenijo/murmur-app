export const DISCOVERY_STORAGE_KEY = 'murmur_discovery_v2';
export const DISCOVERY_CHANGED_EVENT = 'murmur-discovery-changed';

export const DISCOVERY_HINT_IDS = [
  'double_tap_timing',
  'correct_and_teach',
  'browser_modes',
] as const;

export type DiscoveryHintId = (typeof DISCOVERY_HINT_IDS)[number];

export type DiscoveryDestination =
  | { kind: 'settings'; page: string; target: string }
  | { kind: 'teach' };

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

interface DiscoveryStateV2 {
  version: 2;
  pendingHints: DiscoveryHintId[];
  dismissedHints: DiscoveryHintId[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isHintId(value: string): value is DiscoveryHintId {
  return DISCOVERY_HINT_IDS.some((id) => id === value);
}

function unique<T extends string>(values: T[]): T[] {
  return [...new Set(values)];
}

function parseState(value: unknown): DiscoveryStateV2 | null {
  if (!isRecord(value) || (value.version !== 1 && value.version !== 2)) return null;
  // v1 progress-only records may omit hints; v2 requires its two array fields.
  if (value.version === 2 && (!Array.isArray(value.pendingHints) || !Array.isArray(value.dismissedHints))) return null;
  const knownIds = (items: unknown): DiscoveryHintId[] => Array.isArray(items)
    ? unique(items.filter((id): id is DiscoveryHintId => typeof id === 'string' && isHintId(id)))
    : [];
  const dismissedHints = knownIds(value.dismissedHints);
  return {
    version: 2,
    pendingHints: knownIds(value.pendingHints).filter((id) => !dismissedHints.includes(id)),
    dismissedHints,
  };
}

const LEGACY_STORAGE_KEY = 'murmur_discovery_v1';

function readState(raw: string | null): DiscoveryStateV2 | null {
  if (raw === null) return null;
  try { return parseState(JSON.parse(raw)); } catch { return null; }
}

export function loadDiscoveryState(): DiscoveryStateV2 | null {
  try {
    const stored = localStorage.getItem(DISCOVERY_STORAGE_KEY);
    const legacy = localStorage.getItem(LEGACY_STORAGE_KEY);
    const current = readState(stored);
    const previous = readState(legacy);
    if (legacy !== null) {
      try { JSON.parse(legacy); } catch {
        // Unreadable JSON cannot contribute hint IDs, and must not linger forever.
        localStorage.removeItem(LEGACY_STORAGE_KEY);
      }
    }
    if (!current && !previous) return null;
    const dismissedHints = unique([...(current?.dismissedHints ?? []), ...(previous?.dismissedHints ?? [])]);
    const state: DiscoveryStateV2 = {
      version: 2,
      pendingHints: unique([...(current?.pendingHints ?? []), ...(previous?.pendingHints ?? [])])
        .filter((id) => !dismissedHints.includes(id)),
      dismissedHints,
    };
    try {
      localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(state));
      // Keep recoverable legacy data until its replacement has been persisted.
      localStorage.removeItem(LEGACY_STORAGE_KEY);
    } catch {
      // Return validated hints in memory; a later load can retry the migration.
    }
    return state;
  } catch {
    return null;
  }
}

function saveState(state: DiscoveryStateV2): DiscoveryStateV2 {
  try {
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(state));
    localStorage.removeItem(LEGACY_STORAGE_KEY);
    window.dispatchEvent(new Event(DISCOVERY_CHANGED_EVENT));
  } catch {
    // Discovery never blocks the main app when storage is unavailable.
  }
  return state;
}

export function initializeDiscoveryAfterSetup(): DiscoveryStateV2 {
  return saveState({ version: 2, pendingHints: [], dismissedHints: [] });
}

export function initializeGrandfatheredDiscovery(): DiscoveryStateV2 {
  return loadDiscoveryState() ?? initializeDiscoveryAfterSetup();
}

export function enqueueDiscoveryHint(id: DiscoveryHintId): DiscoveryStateV2 | null {
  const state = loadDiscoveryState();
  if (!state || state.pendingHints.includes(id) || state.dismissedHints.includes(id)) return state;
  return saveState({ ...state, pendingHints: [...state.pendingHints, id] });
}

export function dismissDiscoveryHint(id: DiscoveryHintId): DiscoveryStateV2 | null {
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
    localStorage.removeItem(DISCOVERY_STORAGE_KEY);
    localStorage.removeItem(LEGACY_STORAGE_KEY);
    window.dispatchEvent(new Event(DISCOVERY_CHANGED_EVENT));
  } catch {
    // Non-fatal.
  }
}
