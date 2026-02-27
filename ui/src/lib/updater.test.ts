import { describe, it, expect, beforeEach } from 'vitest';
import {
  parseSemver,
  compareSemver,
  getSkippedVersion,
  setSkippedVersion,
  clearSkippedVersion,
  getLastCheckTimestamp,
  setLastCheckTimestamp,
  isDueForCheck,
  CHECK_INTERVAL_MS,
} from './updater';

describe('parseSemver', () => {
  it('parses valid semver strings', () => {
    expect(parseSemver('1.2.3')).toEqual([1, 2, 3]);
    expect(parseSemver('0.6.2')).toEqual([0, 6, 2]);
    expect(parseSemver('10.20.30')).toEqual([10, 20, 30]);
  });

  it('parses semver with extra text after patch', () => {
    expect(parseSemver('1.2.3-beta.1')).toEqual([1, 2, 3]);
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

  it('returns 0 for unparseable versions (fail-safe)', () => {
    expect(compareSemver('bad', '0.6.2')).toBe(0);
    expect(compareSemver('0.6.2', 'bad')).toBe(0);
    expect(compareSemver('bad', 'bad')).toBe(0);
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
