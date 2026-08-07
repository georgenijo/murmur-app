const SKIPPED_VERSION_KEY = 'skipped-update-version';
const LAST_CHECK_KEY = 'updater-last-check';
const PENDING_UPDATE_KEY = 'pending-update-release-notes';
const MAX_RELEASE_NOTES_LENGTH = 50_000;
export const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000; // 6 hours
export const CHECK_TIMER_TICK_MS = 15 * 60 * 1000; // 15 minutes

const LATEST_JSON_URL =
  'https://github.com/georgenijo/murmur-app/releases/latest/download/latest-v2.json';

// --- Semver comparison ---

export function parseSemver(version: string): [number, number, number] | null {
  const match = version.trim().match(/^v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/);
  if (!match) return null;
  return [parseInt(match[1], 10), parseInt(match[2], 10), parseInt(match[3], 10)];
}

/**
 * Returns -1 if a < b, 0 if equal, 1 if a > b.
 * Returns null if either version is unparseable — callers must handle this.
 */
export function compareSemver(a: string, b: string): -1 | 0 | 1 | null {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  if (!pa || !pb) return null;
  for (let i = 0; i < 3; i++) {
    if (pa[i] < pb[i]) return -1;
    if (pa[i] > pb[i]) return 1;
  }
  return 0;
}

/**
 * Check if currentVersion is below minVersion.
 * Returns true (force update) if either version is unparseable — fail-safe
 * so that malformed versions cannot bypass min_version enforcement.
 */
export function isBelowMinVersion(currentVersion: string, minVersion: string): boolean {
  const result = compareSemver(currentVersion, minVersion);
  if (result === null) return true; // unparseable → force update
  return result < 0;
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
    if (!val) return 0;
    const parsed = parseInt(val, 10);
    return Number.isFinite(parsed) ? parsed : 0;
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

// --- Post-update release notes ---

export interface CompletedUpdate {
  version: string;
  notes: string;
}

/**
 * Save the release payload before Tauri replaces and relaunches the app.
 * localStorage survives the install, while in-memory updater state does not.
 */
export function setPendingUpdate(update: CompletedUpdate): void {
  try {
    localStorage.setItem(PENDING_UPDATE_KEY, JSON.stringify({
      version: update.version,
      notes: update.notes.slice(0, MAX_RELEASE_NOTES_LENGTH),
    }));
  } catch { /* ignore */ }
}

export function clearPendingUpdate(): void {
  try {
    localStorage.removeItem(PENDING_UPDATE_KEY);
  } catch { /* ignore */ }
}

/**
 * Return release notes only when the relaunched app is the exact version that
 * was downloaded. Mismatched or malformed payloads are stale and are removed.
 */
export function getPendingUpdateForVersion(currentVersion: string): CompletedUpdate | null {
  try {
    const stored = localStorage.getItem(PENDING_UPDATE_KEY);
    if (!stored) return null;

    const parsed: unknown = JSON.parse(stored);
    if (
      typeof parsed !== 'object' ||
      parsed === null ||
      !('version' in parsed) ||
      !('notes' in parsed) ||
      typeof parsed.version !== 'string' ||
      typeof parsed.notes !== 'string' ||
      normalizedVersionIdentity(parsed.version) === null ||
      normalizedVersionIdentity(currentVersion) === null ||
      normalizedVersionIdentity(parsed.version) !== normalizedVersionIdentity(currentVersion)
    ) {
      clearPendingUpdate();
      return null;
    }

    return {
      version: parsed.version,
      notes: parsed.notes.slice(0, MAX_RELEASE_NOTES_LENGTH),
    };
  } catch {
    clearPendingUpdate();
    return null;
  }
}

function normalizedVersionIdentity(version: string): string | null {
  if (!parseSemver(version)) return null;
  return version.trim().replace(/^v/, '');
}

// --- min_version fetch ---

export type MinVersionPolicy =
  | { status: 'present'; minVersion: string }
  | { status: 'absent' }
  | { status: 'unavailable'; message: string };

/**
 * Fetch the custom min_version field from the current update channel.
 * Absence is an intentional optional-update policy; transport and schema
 * failures stay distinct so callers cannot silently downgrade enforcement.
 */
export async function fetchMinVersionPolicy(): Promise<MinVersionPolicy> {
  try {
    const response = await fetch(LATEST_JSON_URL, { cache: 'no-store' });
    if (!response.ok) {
      return {
        status: 'unavailable',
        message: `Update policy request failed with status ${response.status}.`,
      };
    }
    const data: unknown = await response.json();
    if (typeof data !== 'object' || data === null) {
      return { status: 'unavailable', message: 'Update policy response was not an object.' };
    }
    if (!('min_version' in data)) return { status: 'absent' };
    if (typeof data.min_version !== 'string') {
      return { status: 'unavailable', message: 'Update policy min_version was not a string.' };
    }
    return { status: 'present', minVersion: data.min_version };
  } catch (error) {
    return { status: 'unavailable', message: String(error) };
  }
}

// --- Update state types ---

export type UpdateStatus =
  | { phase: 'idle' }
  | { phase: 'checking' }
  | { phase: 'available'; version: string; notes: string; isForced: boolean }
  | { phase: 'preparing'; version: string }
  | { phase: 'downloading'; version: string; progress: number }
  | { phase: 'ready'; version: string }
  | {
      phase: 'error';
      stage: 'check' | 'install';
      message: string;
      isForced: boolean;
      recovery?: 'reinstall';
    }
  | { phase: 'up-to-date' };
