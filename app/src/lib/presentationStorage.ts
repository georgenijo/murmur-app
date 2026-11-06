// Hydrated snapshots, writes and removal tombstones stay authoritative for this
// renderer session. Legacy import reads bypass this adapter and fail closed.
export const PRESENTATION_KEYS = [
  'dictation-settings', 'murmur-theme-library', 'murmur-appearance',
  'murmur_discovery_v1', 'murmur.settings-disclosure.v1', 'usage-dashboard-collapsed',
  'murmur-microphone-startup-benchmark-reports', 'murmur_onboarding_complete',
] as const;
const ownedKeys = new Set<string>([
  ...PRESENTATION_KEYS, 'murmur-overlay-vertical-offset', 'murmur-ui-latency-v1',
]);
export const PRESENTATION_CHANGED = 'murmur-presentation-changed';
export function ownsPresentationKey(key: string): boolean { return ownedKeys.has(key); }
const values = new Map<string, string | null>();
const generations = new Map<string, number>();
let reload: ((keys: readonly string[]) => void) | undefined;
let reported = false;
function cacheFailure(): void {
  if (reported) return;
  reported = true;
  console.warn('Presentation cache unavailable; using in-memory state.');
}
function remember(key: string, value: string | null): void {
  if (!ownedKeys.has(key)) return;
  values.set(key, value);
  generations.set(key, (generations.get(key) ?? 0) + 1);
}
export function setPresentationReloader(handler: (keys: readonly string[]) => void): void {
  reload = handler;
}
export const presentationStorage = {
  /** Read ownership is shared by all async native readers in this renderer. */
  generation(key: string): number { return generations.get(key) ?? 0; },
  beginRead(key: string): number {
    if (!ownedKeys.has(key)) return 0;
    const generation = this.generation(key) + 1;
    generations.set(key, generation);
    return generation;
  },
  rememberIfCurrent(key: string, value: string | null, generation: number): boolean {
    if (!ownedKeys.has(key) || generation !== this.generation(key)) return false;
    remember(key, value);
    return true;
  },
  /** Native notifications update this renderer without rewriting shared DOM. */
  rememberItem(key: string, value: string): void { remember(key, value); },
  getItem(key: string): string | null {
    if (values.has(key)) return values.get(key) ?? null;
    try { return localStorage.getItem(key); }
    catch {
      cacheFailure();
      throw new Error('Presentation cache unavailable');
    }
  },
  setItem(key: string, value: string): void {
    remember(key, value);
    try { localStorage.setItem(key, value); }
    catch { cacheFailure(); }
  },
  removeItem(key: string): void {
    remember(key, null);
    try { localStorage.removeItem(key); }
    catch { cacheFailure(); }
  },
};

window.addEventListener('storage', event => {
  if (event.storageArea !== null && event.storageArea !== localStorage) return;
  if (event.key !== null && !ownedKeys.has(event.key)) return;
  const keys = event.key === null ? [...ownedKeys] : [event.key];
  // DOM payloads have no commit revision. In a hydrated native renderer they
  // are hints only, including delayed deletion/clear events.
  if (reload) reload(keys);
  else {
    // Plain-browser presentation reads the current cache, never an event's
    // potentially queued old value. No unowned entries or tombstones are kept.
    for (const key of keys) {
      try {
        const value = localStorage.getItem(key);
        remember(key, value);
        if (value === null) values.delete(key);
      }
      catch { /* An unreadable cache cannot invalidate a usable session copy. */ }
    }
  }
});
