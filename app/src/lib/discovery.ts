export const DISCOVERY_STORAGE_KEY = 'murmur_discovery_v2';
export const DISCOVERY_CHANGED_EVENT = 'murmur-discovery-changed';

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
  if (!isRecord(value) || (value.version !== 1 && value.version !== 2)
    || !Array.isArray(value.pendingHints) || !value.pendingHints.every((id) => typeof id === 'string' && isHintId(id))
    || !Array.isArray(value.dismissedHints) || !value.dismissedHints.every((id) => typeof id === 'string' && isHintId(id))) return null;
  const dismissedHints = unique(value.dismissedHints as DiscoveryHintId[]);
  return {
    version: 2,
    pendingHints: unique(value.pendingHints as DiscoveryHintId[]).filter((id) => !dismissedHints.includes(id)),
    dismissedHints,
  };
}

export function loadDiscoveryState(): DiscoveryStateV2 | null {
  try {
    const stored = localStorage.getItem(DISCOVERY_STORAGE_KEY);
    if (stored !== null) return parseState(JSON.parse(stored));
    const legacy = localStorage.getItem('murmur_discovery_v1');
    if (legacy === null) return null;
    const state = parseState(JSON.parse(legacy));
    if (state) localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(state));
    localStorage.removeItem('murmur_discovery_v1');
    return state;
  } catch {
    return null;
  }
}

function saveState(state: DiscoveryStateV2): DiscoveryStateV2 {
  try {
    localStorage.setItem(DISCOVERY_STORAGE_KEY, JSON.stringify(state));
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
    localStorage.removeItem('murmur_discovery_v1');
    window.dispatchEvent(new Event(DISCOVERY_CHANGED_EVENT));
  } catch {
    // Non-fatal.
  }
}
