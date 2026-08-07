import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  parseSemver,
  compareSemver,
  isBelowMinVersion,
  getSkippedVersion,
  setSkippedVersion,
  clearSkippedVersion,
  getLastCheckTimestamp,
  setLastCheckTimestamp,
  isDueForCheck,
  setPendingUpdate,
  clearPendingUpdate,
  getPendingUpdateForVersion,
  fetchMinVersionPolicy,
  CHECK_INTERVAL_MS,
} from './updater';

describe('parseSemver', () => {
  it('parses valid semver strings', () => {
    expect(parseSemver('1.2.3')).toEqual([1, 2, 3]);
    expect(parseSemver('0.6.2')).toEqual([0, 6, 2]);
    expect(parseSemver('10.20.30')).toEqual([10, 20, 30]);
  });

  it('parses semver with pre-release and build metadata', () => {
    expect(parseSemver('1.2.3-beta.1')).toEqual([1, 2, 3]);
    expect(parseSemver('1.2.3+build.123')).toEqual([1, 2, 3]);
    expect(parseSemver('1.2.3-rc.1+build')).toEqual([1, 2, 3]);
  });

  it('handles v prefix and whitespace', () => {
    expect(parseSemver('v1.2.3')).toEqual([1, 2, 3]);
    expect(parseSemver(' 1.2.3 ')).toEqual([1, 2, 3]);
    expect(parseSemver(' v0.6.2 ')).toEqual([0, 6, 2]);
  });

  it('returns null for invalid strings', () => {
    expect(parseSemver('')).toBeNull();
    expect(parseSemver('abc')).toBeNull();
    expect(parseSemver('1.2')).toBeNull();
  });
});

describe('compareSemver', () => {
  it('detects equal versions', () => {
    expect(compareSemver('1.0.0', '1.0.0')).toBe(0);
    expect(compareSemver('0.6.2', '0.6.2')).toBe(0);
  });

  it('detects less-than', () => {
    expect(compareSemver('0.5.0', '0.6.0')).toBe(-1);
    expect(compareSemver('0.6.1', '0.6.2')).toBe(-1);
    expect(compareSemver('0.6.2', '1.0.0')).toBe(-1);
  });

  it('detects greater-than', () => {
    expect(compareSemver('0.6.3', '0.6.2')).toBe(1);
    expect(compareSemver('1.0.0', '0.9.9')).toBe(1);
  });

  it('returns null for unparseable versions', () => {
    expect(compareSemver('bad', '0.6.2')).toBeNull();
    expect(compareSemver('0.6.2', 'bad')).toBeNull();
    expect(compareSemver('bad', 'bad')).toBeNull();
  });
});

describe('isBelowMinVersion', () => {
  it('returns true when current < min', () => {
    expect(isBelowMinVersion('0.6.0', '0.7.0')).toBe(true);
  });

  it('returns false when current >= min', () => {
    expect(isBelowMinVersion('0.7.0', '0.7.0')).toBe(false);
    expect(isBelowMinVersion('0.8.0', '0.7.0')).toBe(false);
  });

  it('returns true (force update) when versions are unparseable', () => {
    expect(isBelowMinVersion('bad', '0.7.0')).toBe(true);
    expect(isBelowMinVersion('0.7.0', 'bad')).toBe(true);
    expect(isBelowMinVersion('bad', 'bad')).toBe(true);
  });
});

describe('skipped version storage', () => {
  beforeEach(() => localStorage.clear());

  it('returns null when nothing stored', () => {
    expect(getSkippedVersion()).toBeNull();
  });

  it('round-trips a version string', () => {
    setSkippedVersion('0.7.0');
    expect(getSkippedVersion()).toBe('0.7.0');
  });

  it('clears the stored version', () => {
    setSkippedVersion('0.7.0');
    clearSkippedVersion();
    expect(getSkippedVersion()).toBeNull();
  });
});

describe('check interval', () => {
  beforeEach(() => localStorage.clear());

  it('returns 0 when no timestamp stored', () => {
    expect(getLastCheckTimestamp()).toBe(0);
  });

  it('round-trips a timestamp', () => {
    const ts = Date.now();
    setLastCheckTimestamp(ts);
    expect(getLastCheckTimestamp()).toBe(ts);
  });

  it('isDueForCheck returns true when never checked', () => {
    expect(isDueForCheck()).toBe(true);
  });

  it('isDueForCheck returns false right after setting timestamp', () => {
    setLastCheckTimestamp(Date.now());
    expect(isDueForCheck()).toBe(false);
  });

  it('isDueForCheck returns true when timestamp is old enough', () => {
    setLastCheckTimestamp(Date.now() - CHECK_INTERVAL_MS - 1);
    expect(isDueForCheck()).toBe(true);
  });
});

describe('post-update release notes', () => {
  beforeEach(() => localStorage.clear());

  it('returns the saved notes for the exact installed version', () => {
    setPendingUpdate({
      version: '0.22.0',
      notes: '## New Features\n\n- Added a post-update summary.',
    });

    expect(getPendingUpdateForVersion('0.22.0')).toEqual({
      version: '0.22.0',
      notes: '## New Features\n\n- Added a post-update summary.',
    });
  });

  it('accepts an equivalent v-prefixed installed version', () => {
    setPendingUpdate({ version: 'v0.22.0', notes: 'Bug fixes.' });

    expect(getPendingUpdateForVersion('0.22.0')?.version).toBe('v0.22.0');
  });

  it('removes stale notes when the installed version does not match', () => {
    setPendingUpdate({ version: '0.22.0', notes: 'Old notes.' });

    expect(getPendingUpdateForVersion('0.23.0')).toBeNull();
    expect(getPendingUpdateForVersion('0.22.0')).toBeNull();
  });

  it('does not confuse prerelease and final builds with the same core semver', () => {
    setPendingUpdate({ version: '0.22.0-beta.1', notes: 'Preview notes.' });

    expect(getPendingUpdateForVersion('0.22.0')).toBeNull();
  });

  it('rejects invalid versions even when both strings match', () => {
    setPendingUpdate({ version: 'not-a-version', notes: 'Invalid notes.' });

    expect(getPendingUpdateForVersion('not-a-version')).toBeNull();
  });

  it('fails closed and removes malformed storage', () => {
    localStorage.setItem('pending-update-release-notes', '{broken');

    expect(getPendingUpdateForVersion('0.22.0')).toBeNull();
    expect(localStorage.getItem('pending-update-release-notes')).toBeNull();
  });

  it('can be dismissed permanently', () => {
    setPendingUpdate({ version: '0.22.0', notes: 'Notes.' });
    clearPendingUpdate();

    expect(getPendingUpdateForVersion('0.22.0')).toBeNull();
  });
});

describe('minimum-version policy fetch', () => {
  it('distinguishes an intentionally absent policy', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ version: '0.24.2' }),
    }));

    await expect(fetchMinVersionPolicy()).resolves.toEqual({ status: 'absent' });
    vi.unstubAllGlobals();
  });

  it('returns a present policy', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ min_version: '0.23.0' }),
    }));

    await expect(fetchMinVersionPolicy()).resolves.toEqual({
      status: 'present',
      minVersion: '0.23.0',
    });
    vi.unstubAllGlobals();
  });

  it('does not collapse transport and schema failures into absence', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: false,
      status: 503,
    }));
    await expect(fetchMinVersionPolicy()).resolves.toMatchObject({
      status: 'unavailable',
      message: expect.stringContaining('503'),
    });

    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ min_version: 23 }),
    }));
    await expect(fetchMinVersionPolicy()).resolves.toMatchObject({
      status: 'unavailable',
      message: expect.stringContaining('not a string'),
    });
    vi.unstubAllGlobals();
  });
});
