import { describe, it, expect, beforeEach } from 'vitest';
import {
  parseSemver,
  compareSemver,
  isBelowMinVersion,
  getLastCheckTimestamp,
  setLastCheckTimestamp,
  isDueForCheck,
  setPendingUpdate,
  clearPendingUpdate,
  getPendingUpdateForVersion,
  parseMinVersionPolicy,
  getUpdateReleaseNotes,
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

describe('minimum-version policy parsing', () => {
  it('distinguishes an intentionally absent policy', () => {
    expect(parseMinVersionPolicy({ version: '0.24.2' })).toEqual({ status: 'absent' });
  });

  it('returns a present policy', () => {
    expect(parseMinVersionPolicy({ min_version: '0.23.0' })).toEqual({
      status: 'present',
      minVersion: '0.23.0',
    });
  });

  it('does not collapse missing or malformed native metadata into absence', () => {
    expect(parseMinVersionPolicy(undefined)).toMatchObject({
      status: 'unavailable',
      message: expect.stringContaining('not an object'),
    });
    expect(parseMinVersionPolicy({ min_version: 23 })).toMatchObject({
      status: 'unavailable',
      message: expect.stringContaining('not a string'),
    });
  });
});


describe('cumulative release notes', () => {
  const manifest = { release_history: [
    { version: '0.59.3', notes: 'Third fix' },
    { version: '0.59.1', notes: 'Already installed' },
    { version: '0.59.5', notes: 'Stale latest notes' },
    { version: '0.59.2', notes: 'Second fix' },
    { version: '0.59.6', notes: 'Future fix' },
    { version: '0.59.4', notes: 'Fourth fix' },
    { version: '0.59.4-beta.1', notes: 'Preview' },
    { version: 'bad', notes: 'Invalid' },
    null,
  ] };

  it('includes every missed release newest first and excludes installed/future/preview versions', () => {
    const notes = getUpdateReleaseNotes(manifest, '0.59.1', '0.59.5', 'Latest fix');
    expect(notes).toBe('# v0.59.5\n\nLatest fix\n\n---\n\n# v0.59.4\n\nFourth fix\n\n---\n\n# v0.59.3\n\nThird fix\n\n---\n\n# v0.59.2\n\nSecond fix');
  });

  it('uses numeric version ordering and accepts v prefixes', () => {
    const notes = getUpdateReleaseNotes({ release_history: [
      { version: 'v0.59.9', notes: 'Nine' }, { version: '0.59.10', notes: 'Ten' },
    ] }, 'v0.59.8', '0.59.11', 'Eleven');
    expect(notes.indexOf('v0.59.10')).toBeLessThan(notes.indexOf('v0.59.9'));
    expect(notes.match(/# v/g)).toHaveLength(3);
  });

  it('keeps single-release notes and falls back for missing or malformed history', () => {
    for (const raw of [null, {}, { release_history: 'bad' }, { release_history: [null, {}, { version: '0.59.3', notes: 42 }] }]) {
      expect(getUpdateReleaseNotes(raw, '0.59.4', '0.59.5', 'Latest')).toBe('Latest');
    }
    expect(getUpdateReleaseNotes(manifest, '0.59.4', '0.59.5', 'Latest')).toBe('Latest');
    expect(getUpdateReleaseNotes(manifest, 'bad', '0.59.5', 'Latest')).toBe('Latest');
    expect(getUpdateReleaseNotes(manifest, '0.59.1-beta', '0.59.5', 'Latest')).toBe('Latest');
  });
});
