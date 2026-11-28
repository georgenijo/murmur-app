import { describe, expect, it } from 'vitest';
import {
  countExpiredHistory,
  isExpiredHistoryEntry,
  isHistoryRetentionDays,
  pruneExpiredHistory,
  type HistoryEntry,
} from './history';
import { DEFAULT_SETTINGS, parseSettingsBlob } from './settings';

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.UTC(2026, 10, 27, 12);

function entry(id: string, ageMs: number, pinned = false): HistoryEntry {
  return { schemaVersion: 2, id, text: `text ${id}`, timestamp: NOW - ageMs, duration: 1, source: 'recording', pinned };
}

describe('automatic transcript deletion', () => {
  it('never deletes anything with the default "Never" window', () => {
    const entries = [entry('old', 400 * DAY), entry('new', 1)];
    expect(DEFAULT_SETTINGS.historyRetentionDays).toBe(0);
    expect(countExpiredHistory(entries, 0, NOW)).toBe(0);
    expect(pruneExpiredHistory(entries, 0, NOW)).toBe(entries);
  });

  it('uses a rolling age: exactly at the boundary is kept, one ms past is deleted', () => {
    expect(isExpiredHistoryEntry(entry('edge', 7 * DAY), 7, NOW)).toBe(false);
    expect(isExpiredHistoryEntry(entry('past', 7 * DAY + 1), 7, NOW)).toBe(true);
  });

  it('keeps pinned, future-dated, and unusable timestamps', () => {
    const odd: HistoryEntry = { ...entry('nan', 0), timestamp: Number.NaN };
    const entries = [entry('pinned', 100 * DAY, true), entry('future', -5 * DAY), odd];
    expect(countExpiredHistory(entries, 1, NOW)).toBe(0);
    expect(pruneExpiredHistory(entries, 1, NOW)).toBe(entries);
  });

  it('removes only expired unpinned entries and preserves stored order', () => {
    const entries = [entry('a', 40 * DAY), entry('b', 35 * DAY, true), entry('c', 31 * DAY), entry('d', 2 * DAY)];
    expect(countExpiredHistory(entries, 30, NOW)).toBe(2);
    expect(pruneExpiredHistory(entries, 30, NOW).map((e) => e.id)).toEqual(['b', 'd']);
  });

  it('accepts only the offered windows', () => {
    for (const value of [0, 1, 7, 30, 90]) expect(isHistoryRetentionDays(value)).toBe(true);
    for (const value of [2, -1, 365, '7', null, undefined, Number.NaN]) expect(isHistoryRetentionDays(value)).toBe(false);
  });

  it('parses a stored window and falls back to "Never" for anything else', () => {
    const stored = (value: unknown) => parseSettingsBlob(JSON.stringify({ ...DEFAULT_SETTINGS, historyRetentionDays: value }));
    expect(stored(30).historyRetentionDays).toBe(30);
    expect(stored(45).historyRetentionDays).toBe(0);
    expect(stored('7').historyRetentionDays).toBe(0);
    const { historyRetentionDays: _omit, ...legacy } = DEFAULT_SETTINGS;
    expect(parseSettingsBlob(JSON.stringify(legacy)).historyRetentionDays).toBe(0);
  });
});
