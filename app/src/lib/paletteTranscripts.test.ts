import { describe, expect, it, vi } from 'vitest';
import type { HistoryEntry } from './history';
import {
  MAX_TRANSCRIPT_RESULTS,
  TRANSCRIPT_SECTION,
  TRANSCRIPT_SNIPPET_LENGTH,
  matchTranscripts,
  transcriptPaletteCommands,
  transcriptSnippet,
} from './paletteTranscripts';

function entry(id: string, text: string, timestamp: number, extra: Partial<HistoryEntry> = {}): HistoryEntry {
  return { schemaVersion: 2, id, text, timestamp, duration: 3, source: 'recording', ...extra };
}

const base = new Date(2026, 10, 20, 9, 0).getTime();

describe('matchTranscripts', () => {
  const entries = [
    entry('old', 'Release notes for the Tauri update', base),
    entry('mid', 'Lunch with Sam about the release', base + 60_000),
    entry('new', 'Draft the release checklist', base + 120_000),
    entry('file', 'Interview audio', base + 30_000, { source: 'file', sourceName: 'release-call.wav' }),
  ];

  it('returns nothing for an empty or one-character query', () => {
    expect(matchTranscripts(entries, '')).toEqual([]);
    expect(matchTranscripts(entries, '   ')).toEqual([]);
    expect(matchTranscripts(entries, 'r')).toEqual([]);
    expect(matchTranscripts(entries, ' r ')).toEqual([]);
  });

  it('uses the History rule: case-insensitive, every word, newest first', () => {
    expect(matchTranscripts(entries, 'RELEASE').map((e) => e.id)).toEqual(['new', 'mid', 'file', 'old']);
    expect(matchTranscripts(entries, 'release tauri').map((e) => e.id)).toEqual(['old']);
    expect(matchTranscripts(entries, 'release banana')).toEqual([]);
  });

  it('matches an imported file by its source name, like History search', () => {
    expect(matchTranscripts(entries, 'release-call').map((e) => e.id)).toEqual(['file']);
  });

  it('caps the result count and skips blank transcripts', () => {
    const many = Array.from({ length: 12 }, (_, i) => entry(`e${i}`, `note ${i}`, base + i));
    many.push(entry('blank', '   ', base + 100, { sourceName: 'note.wav' }));
    const found = matchTranscripts(many, 'note');
    expect(found).toHaveLength(MAX_TRANSCRIPT_RESULTS);
    expect(found.map((e) => e.id)).toEqual(['e11', 'e10', 'e9', 'e8', 'e7']);
  });

  it('does not modify the history it reads', () => {
    const copy = structuredClone(entries);
    matchTranscripts(entries, 'release');
    expect(entries).toEqual(copy);
  });
});

describe('transcriptSnippet', () => {
  it('collapses whitespace and keeps short text whole', () => {
    expect(transcriptSnippet('  one\n\ntwo\tthree  ', 'two')).toBe('one two three');
  });

  it('truncates long text at a word with a trailing ellipsis', () => {
    const text = 'alpha '.repeat(40).trim();
    const snippet = transcriptSnippet(text, 'alpha');
    expect(snippet.length).toBeLessThanOrEqual(TRANSCRIPT_SNIPPET_LENGTH);
    expect(snippet.endsWith('…')).toBe(true);
    expect(snippet.startsWith('alpha')).toBe(true);
    expect(snippet).not.toMatch(/alph…|\s…/);
  });

  it('shifts to show a late match with leading context', () => {
    const text = `${'filler words here '.repeat(10)}the budget review is due Friday ${'and more after '.repeat(10)}`;
    const snippet = transcriptSnippet(text, 'budget');
    expect(snippet.startsWith('…')).toBe(true);
    expect(snippet.endsWith('…')).toBe(true);
    expect(snippet).toContain('the budget review is due Friday');
    expect(snippet.length).toBeLessThanOrEqual(TRANSCRIPT_SNIPPET_LENGTH);
  });

  it('a late match sits near the start, on a word boundary, even near the end', () => {
    const text = `${'Local fixture dictation keeps the activity view tied to '.repeat(3)}real transcript timestamps.`;
    const snippet = transcriptSnippet(text, 'transcript');
    expect(snippet.startsWith('…')).toBe(true);
    expect(snippet.endsWith('timestamps.')).toBe(true);
    // The dialog truncates the row on the right, so the match must come early.
    expect(snippet.indexOf('transcript')).toBeLessThanOrEqual(26);
    // Never starts mid-word.
    const firstWord = snippet.slice(1).split(' ')[0];
    expect(text.split(' ')).toContain(firstWord);
  });

  it('starts at the match when no space precedes it in the context window', () => {
    const text = `${'x'.repeat(80)} ${'y'.repeat(40)}needle tail`;
    const snippet = transcriptSnippet(text, 'needle');
    expect(snippet.startsWith('…needle')).toBe(true);
  });

  it('a match only in the source name keeps the start of the text', () => {
    const text = 'x '.repeat(100);
    expect(transcriptSnippet(text, 'meeting').startsWith('x x')).toBe(true);
  });

  it('keeps the start when lower-casing changes the length', () => {
    const text = `İ ${'word '.repeat(40)}target`;
    expect(transcriptSnippet(text, 'target').startsWith('İ word')).toBe(true);
  });
});

describe('transcriptPaletteCommands', () => {
  it('builds copy rows with a date label and copies the full stored text', async () => {
    const long = `Start ${'middle '.repeat(30)}end of the dictation`;
    const today = new Date();
    today.setHours(15, 26, 0, 0);
    const copy = vi.fn();
    const [row, ...rest] = transcriptPaletteCommands([entry('a1', long, today.getTime())], 'middle', copy);
    expect(rest).toEqual([]);
    expect(row.id).toBe('transcript-a1');
    expect(row.section).toBe(TRANSCRIPT_SECTION);
    expect(row.actionVerb).toBe('copy');
    expect(row.meta).toMatch(/^Today · /);
    expect(row.title.length).toBeLessThanOrEqual(TRANSCRIPT_SNIPPET_LENGTH);
    await row.run();
    expect(copy).toHaveBeenCalledWith(long);
  });

  it('returns no rows for a short query', () => {
    expect(transcriptPaletteCommands([entry('a', 'hello', base)], 'h', vi.fn())).toEqual([]);
  });
});
