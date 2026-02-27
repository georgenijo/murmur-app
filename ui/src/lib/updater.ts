const SKIPPED_VERSION_KEY = 'skipped-update-version';
const LAST_CHECK_KEY = 'updater-last-check';
export const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

const LATEST_JSON_URL =
  'https://github.com/georgenijo/murmur-app/releases/latest/download/latest.json';

// --- Semver comparison ---

export function parseSemver(version: string): [number, number, number] | null {
  const match = version.match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!match) return null;
  return [parseInt(match[1], 10), parseInt(match[2], 10), parseInt(match[3], 10)];
}

/**
 * Returns -1 if a < b, 0 if equal, 1 if a > b.
 * Returns 0 if either version is unparseable (fail-safe: treat as equal).
 */
export function compareSemver(a: string, b: string): -1 | 0 | 1 {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  if (!pa || !pb) return 0;
  for (let i = 0; i < 3; i++) {
    if (pa[i] < pb[i]) return -1;
    if (pa[i] > pb[i]) return 1;
  }
  return 0;
}

// --- Skipped version management ---

export function getSkippedVersion(): string | null {
  try {
    return localStorage.getItem(SKIPPED_VERSION_KEY);
  } catch {
    return null;
  }
}

export function setSkippedVersion(version: string): void {
  try {
    localStorage.setItem(SKIPPED_VERSION_KEY, version);
  } catch { /* ignore */ }
}

export function clearSkippedVersion(): void {
  try {
    localStorage.removeItem(SKIPPED_VERSION_KEY);
  } catch { /* ignore */ }
}

// --- Check interval management ---

export function getLastCheckTimestamp(): number {
  try {
    const val = localStorage.getItem(LAST_CHECK_KEY);
    return val ? parseInt(val, 10) : 0;
  } catch {
    return 0;
  }
}

export function setLastCheckTimestamp(ts: number): void {
  try {
    localStorage.setItem(LAST_CHECK_KEY, String(ts));
  } catch { /* ignore */ }
}

export function isDueForCheck(): boolean {
  return Date.now() - getLastCheckTimestamp() >= CHECK_INTERVAL_MS;
}

// --- min_version fetch ---

/**
 * Fetch the custom min_version field from latest.json.
 * Returns null if absent, fetch fails, or JSON is invalid.
 */
export async function fetchMinVersion(): Promise<string | null> {
  try {
    const response = await fetch(LATEST_JSON_URL);
    if (!response.ok) return null;
    const data = await response.json();
    if (typeof data.min_version === 'string') return data.min_version;
    return null;
  } catch {
    return null;
  }
}

// --- Update state types ---

export type UpdateStatus =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'available'; version: string; notes: string; isForced: boolean }
  | { phase: 'downloading'; version: string; progress: number }
  | { phase: 'ready'; version: string }
  | { phase: 'error'; message: string }
  | { phase: 'up-to-date' };
