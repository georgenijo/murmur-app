import { describe, it, expect, beforeEach } from 'vitest';
import {
  addHistoryEntry,
  clearHistory,
  entrySource,
  filterHistory,
  formatDuration,
  formatExportTimestamp,
  formatHistoryExport,
  historyExportFileName,
  loadHistory,
  matchSegments,
  saveHistory,
  searchTokens,
  sortForDisplay,
  trimHistory,
  updateHistoryEntry,
  type HistoryEntry,
} from './history';

function entry(overrides: Partial<HistoryEntry> & { id: string }): HistoryEntry {
  return {
    text: 'hello world',
    timestamp: 1_700_000_000_000,
    duration: 3,
    source: 'recording',
    ...overrides,
  };
}

describe('trimHistory', () => {
  it('keeps the newest 200 entries', () => {
    const entries = Array.from({ length: 210 }, (_, i) =>
      entry({ id: `e${i}`, text: `entry ${i}` }));
    const trimmed = trimHistory(entries);
    expect(trimmed).toHaveLength(200);
    expect(trimmed[0].id).toBe('e10');
    expect(trimmed[199].id).toBe('e209');
  });

  it('leaves a list under the cap untouched', () => {
    const entries = Array.from({ length: 200 }, (_, i) => entry({ id: `e${i}` }));
    expect(trimHistory(entries)).toHaveLength(200);
  });

  it('preserves stored order and survives duplicate ids', () => {
    const entries = [
      entry({ id: 'same', text: 'first' }),
      entry({ id: 'same', text: 'second' }),
      entry({ id: 'other', text: 'third' }),
    ];
    expect(trimHistory(entries).map((e) => e.text)).toEqual(['first', 'second', 'third']);
  });
});

describe('persistence', () => {
  beforeEach(() => {
    clearHistory();
  });

  it('round-trips through localStorage and trims on save', () => {
    const entries = Array.from({ length: 205 }, (_, i) => entry({ id: `e${i}` }));
    saveHistory(entries);
    expect(loadHistory()).toHaveLength(200);
  });

  it('returns an empty list when the stored blob is not an array', () => {
    localStorage.setItem('dictation-history', JSON.stringify({ nope: true }));
    expect(loadHistory()).toEqual([]);
  });

  it('returns an empty list when the stored blob is corrupt', () => {
    localStorage.setItem('dictation-history', '{not json');
    expect(loadHistory()).toEqual([]);
  });
});

describe('addHistoryEntry', () => {
  it('assigns unique ids even within the same millisecond', () => {
    let entries: HistoryEntry[] = [];
    entries = addHistoryEntry(entries, 'one', 1);
    entries = addHistoryEntry(entries, 'two', 1);
    expect(entries[0].id).not.toBe(entries[1].id);
  });

  it('trims to the cap as new entries arrive', () => {
    let entries: HistoryEntry[] = [];
    for (let i = 0; i < 210; i++) entries = addHistoryEntry(entries, `text ${i}`, 1);
    expect(entries).toHaveLength(200);
    expect(entries[entries.length - 1].text).toBe('text 209');
  });
});

describe('updateHistoryEntry', () => {
  it('replaces only the addressed entry text', () => {
    const entries = [entry({ id: 'a', text: 'old' }), entry({ id: 'b', text: 'keep' })];
    const next = updateHistoryEntry(entries, 'a', 'new');
    expect(next[0].text).toBe('new');
    expect(next[1].text).toBe('keep');
  });
});

describe('searchTokens', () => {
  it('lowercases and splits on whitespace', () => {
    expect(searchTokens('  Tauri   Rust ')).toEqual(['tauri', 'rust']);
  });
  it('returns nothing for a blank query', () => {
    expect(searchTokens('   ')).toEqual([]);
  });
});

describe('filterHistory', () => {
  const entries = [
    entry({ id: 'mic', text: 'ship the Tauri release notes' }),
    entry({ id: 'file', text: 'imported meeting audio', source: 'file', sourceName: 'standup.wav' }),
    entry({ id: 'note', text: 'remember the rust invariant' }),
  ];

  it('returns everything with no query or filter', () => {
    expect(filterHistory(entries)).toHaveLength(3);
  });

  it('matches case-insensitively', () => {
    expect(filterHistory(entries, { query: 'TAURI' }).map((e) => e.id)).toEqual(['mic']);
  });

  it('requires every token to match (AND, not OR)', () => {
    expect(filterHistory(entries, { query: 'the release' }).map((e) => e.id)).toEqual(['mic']);
    expect(filterHistory(entries, { query: 'tauri rust' })).toEqual([]);
  });

  it('matches the source file name too', () => {
    expect(filterHistory(entries, { query: 'standup' }).map((e) => e.id)).toEqual(['file']);
  });

  it('filters by source', () => {
    expect(filterHistory(entries, { filter: 'file' }).map((e) => e.id)).toEqual(['file']);
    expect(filterHistory(entries, { filter: 'recording' }).map((e) => e.id)).toEqual(['mic', 'note']);
  });

  it('combines a filter and a query', () => {
    expect(filterHistory(entries, { filter: 'file', query: 'tauri' })).toEqual([]);
  });

  it('treats a missing source as a recording', () => {
    const legacy = [entry({ id: 'legacy', source: undefined })];
    expect(entrySource(legacy[0])).toBe('recording');
    expect(filterHistory(legacy, { filter: 'recording' })).toHaveLength(1);
  });

  it('preserves stored order', () => {
    expect(filterHistory(entries, { query: 'e' }).map((e) => e.id)).toEqual(['mic', 'file', 'note']);
  });
});

describe('sortForDisplay', () => {
  it('puts newest first', () => {
    const entries = [
      entry({ id: 'old', timestamp: 100 }),
      entry({ id: 'new', timestamp: 300 }),
      entry({ id: 'mid', timestamp: 200 }),
    ];
    expect(sortForDisplay(entries).map((e) => e.id)).toEqual(['new', 'mid', 'old']);
  });

  it('is stable for equal timestamps (newest stored last wins)', () => {
    const entries = [
      entry({ id: 'a', timestamp: 100 }),
      entry({ id: 'b', timestamp: 100 }),
    ];
    expect(sortForDisplay(entries).map((e) => e.id)).toEqual(['b', 'a']);
  });

  it('does not mutate the input', () => {
    const entries = [entry({ id: 'a', timestamp: 1 }), entry({ id: 'b', timestamp: 2 })];
    sortForDisplay(entries);
    expect(entries.map((e) => e.id)).toEqual(['a', 'b']);
  });
});

describe('matchSegments', () => {
  it('returns one plain segment for an empty query', () => {
    expect(matchSegments('hello', '')).toEqual([{ text: 'hello', match: false }]);
  });

  it('highlights a case-insensitive match while preserving original casing', () => {
    expect(matchSegments('Hello Tauri', 'tauri')).toEqual([
      { text: 'Hello ', match: false },
      { text: 'Tauri', match: true },
    ]);
  });

  it('highlights every occurrence', () => {
    expect(matchSegments('a b a', 'a')).toEqual([
      { text: 'a', match: true },
      { text: ' b ', match: false },
      { text: 'a', match: true },
    ]);
  });

  it('merges overlapping token matches into one range', () => {
    expect(matchSegments('therein', 'there here')).toEqual([
      { text: 'there', match: true },
      { text: 'in', match: false },
    ]);
  });

  it('reassembles to the original text', () => {
    const text = 'The quick brown fox jumps over the lazy dog';
    const joined = matchSegments(text, 'the o').map((s) => s.text).join('');
    expect(joined).toBe(text);
  });

  it('treats regex metacharacters literally', () => {
    expect(matchSegments('cost is $5.00 (net)', '$5.00')).toEqual([
      { text: 'cost is ', match: false },
      { text: '$5.00', match: true },
      { text: ' (net)', match: false },
    ]);
  });

  it('returns a single plain segment when nothing matches', () => {
    expect(matchSegments('hello', 'zzz')).toEqual([{ text: 'hello', match: false }]);
  });

  it('handles empty text', () => {
    expect(matchSegments('', 'a')).toEqual([{ text: '', match: false }]);
  });
});

describe('formatting helpers', () => {
  it('formats durations under and over a minute', () => {
    expect(formatDuration(0)).toBe('0s');
    expect(formatDuration(12.4)).toBe('12s');
    expect(formatDuration(75)).toBe('1m 15s');
    expect(formatDuration(-3)).toBe('0s');
  });

  it('formats export timestamps as local YYYY-MM-DD HH:MM:SS', () => {
    const at = new Date(2026, 6, 27, 9, 5, 3);
    expect(formatExportTimestamp(at.getTime())).toBe('2026-07-27 09:05:03');
  });

  it('builds a dated export file name per format', () => {
    const at = new Date(2026, 6, 27, 14, 32);
    expect(historyExportFileName('markdown', at)).toBe('murmur-history-2026-07-27-1432.md');
    expect(historyExportFileName('text', at)).toBe('murmur-history-2026-07-27-1432.txt');
    expect(historyExportFileName('json', at)).toBe('murmur-history-2026-07-27-1432.json');
  });
});

describe('formatHistoryExport', () => {
  const exportedAt = new Date(2026, 6, 27, 14, 32, 0);
  const entries = [
    entry({
      id: 'a',
      text: 'first transcript',
      timestamp: new Date(2026, 6, 27, 9, 0, 0).getTime(),
      duration: 4,
      teachingContext: { bundleId: 'com.example.app' } as never,
    }),
    entry({
      id: 'b',
      text: 'second transcript',
      timestamp: new Date(2026, 6, 27, 10, 0, 0).getTime(),
      duration: 65,
      source: 'file',
      sourceName: 'notes.m4a',
    }),
  ];

  it('writes markdown newest first with metadata', () => {
    const md = formatHistoryExport(entries, 'markdown', exportedAt);
    expect(md).toContain('# Murmur transcript history');
    expect(md).toContain('Exported 2026-07-27 14:32:00 · 2 entries');
    expect(md.indexOf('second transcript')).toBeLessThan(md.indexOf('first transcript'));
    expect(md).toContain('- Source: notes.m4a');
    expect(md).toContain('- Duration: 1m 5s');
    expect(md.endsWith('\n')).toBe(true);
  });

  it('writes plain text blocks', () => {
    const txt = formatHistoryExport(entries, 'text', exportedAt);
    expect(txt).toContain('[2026-07-27 10:00:00 · notes.m4a · 1m 5s]');
    expect(txt).toContain('[2026-07-27 09:00:00 · Mic · 4s]');
    expect(txt).toContain('first transcript');
  });

  it('writes self-identifying JSON', () => {
    const parsed = JSON.parse(formatHistoryExport(entries, 'json', exportedAt));
    expect(parsed.schema).toBe('murmur.history.v1');
    expect(parsed.count).toBe(2);
    expect(parsed.entries[0].id).toBe('b');
    expect(parsed.entries[0].sourceName).toBe('notes.m4a');
    expect(parsed.entries[1].source).toBe('recording');
  });

  it('never exports teaching context in any format', () => {
    for (const format of ['markdown', 'text', 'json'] as const) {
      const output = formatHistoryExport(entries, format, exportedAt);
      expect(output).not.toContain('com.example.app');
      expect(output).not.toContain('teachingContext');
    }
  });

  it('handles an empty selection', () => {
    expect(formatHistoryExport([], 'text', exportedAt)).toBe('\n');
    expect(JSON.parse(formatHistoryExport([], 'json', exportedAt)).count).toBe(0);
  });
});
