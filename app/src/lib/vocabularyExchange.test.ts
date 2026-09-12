import { describe, expect, it } from 'vitest';
import type { VocabularyEntry } from './settings';
import {
  commitVocabularyImport, exportVocabularyFile, MAX_VOCABULARY_FILE_BYTES, parseVocabularyFile,
  previewVocabularyImport, type VocabularyExchangeSettings,
} from './vocabularyExchange';

const globalEntry: VocabularyEntry = {
  id: 'tauri', written: 'Tauri', aliases: ['Tori', 'Tory'], enabled: true, scope: { kind: 'global' },
};
const scopedEntries: VocabularyEntry[] = [
  globalEntry,
  { id: 'editor', written: 'Editor', aliases: ['ed'], enabled: true, scope: { kind: 'app', bundleId: 'com.example.Editor' } },
  { id: 'project', written: 'Project', aliases: ['proj'], enabled: false, scope: { kind: 'project', bundleId: 'com.example.Editor', root: '/private/project' } },
];
const empty: VocabularyExchangeSettings = { vocabularyEntries: [], voiceCommands: [] };
const source: VocabularyExchangeSettings = { vocabularyEntries: scopedEntries, voiceCommands: [] };
const file = () => parseVocabularyFile(exportVocabularyFile(source));

function expectRejected(update: object) {
  expect(() => parseVocabularyFile(JSON.stringify({ ...file(), ...update }))).toThrow();
}

describe('Vocabulary JSON boundary', () => {
  it('round trips every scope and pretty prints a versioned file', () => {
    const contents = exportVocabularyFile(source);
    expect(contents).toContain('\n  "format": "murmur-vocabulary"');
    expect(parseVocabularyFile(contents)).toEqual({ format: 'murmur-vocabulary', version: 1, entries: scopedEntries });
    expect(new TextEncoder().encode(contents).length).toBeLessThanOrEqual(MAX_VOCABULARY_FILE_BYTES);
  });

  it('rejects malformed fields, duplicate keys, unsupported scope, and limits', () => {
    expectRejected({ format: 'other' });
    expectRejected({ version: 2 });
    expectRejected({ entries: [{ ...globalEntry, secret: true }] });
    expectRejected({ entries: [{ ...globalEntry, aliases: ['x'.repeat(257)] }] });
    expectRejected({ entries: [{ ...globalEntry, aliases: Array.from({ length: 17 }, () => 'alias') }] });
    expectRejected({ entries: [{ ...globalEntry, scope: { kind: 'cloud' } }] });
    expectRejected({ entries: [{ ...globalEntry, scope: { kind: 'app', bundleId: 'not-a-bundle' } }] });
    expect(() => parseVocabularyFile('{"version":1,"version":2}')).toThrow('Duplicate JSON keys');
    expect(() => parseVocabularyFile('x'.repeat(MAX_VOCABULARY_FILE_BYTES + 1))).toThrow('256 KiB');
  });

  it('rejects invalid enabled entries while preserving disabled entries in a valid export', () => {
    expect(() => exportVocabularyFile({ vocabularyEntries: [{ ...globalEntry, written: '' }], voiceCommands: [] })).toThrow();
    const disabled = { ...globalEntry, enabled: false, written: 'Disabled' };
    expect(parseVocabularyFile(exportVocabularyFile({ vocabularyEntries: [disabled], voiceCommands: [] })).entries).toEqual([disabled]);
  });
});

describe('Vocabulary import merge', () => {
  it('counts exact duplicates and imports new entries without mutating settings', () => {
    const preview = previewVocabularyImport(file(), empty);
    expect(preview.kind).toBe('ready');
    expect(preview.counts).toEqual({ entries: 3, duplicates: 0 });
    const next = commitVocabularyImport(preview, empty);
    expect(next.vocabularyEntries).toEqual(scopedEntries);
    const duplicatePreview = previewVocabularyImport(file(), source);
    expect(duplicatePreview).toMatchObject({ kind: 'ready', counts: { entries: 0, duplicates: 3 } });
    expect(source.vocabularyEntries).toEqual(scopedEntries);
  });

  it('flags ID and overlapping written-term conflicts before commit', () => {
    const idConflict = previewVocabularyImport(file(), { ...empty, vocabularyEntries: [{ ...globalEntry, aliases: ['different'] }] });
    expect(idConflict.kind).toBe('conflict');
    expect(idConflict.kind === 'conflict' && idConflict.conflicts.join(' ')).toContain('Entry ID conflict');
    const writtenConflict = previewVocabularyImport(file(), { ...empty, vocabularyEntries: [{ ...globalEntry, id: 'other' }] });
    expect(writtenConflict.kind).toBe('conflict');
    expect(writtenConflict.kind === 'conflict' && writtenConflict.conflicts.join(' ')).toContain('Written term conflict');
    expect(() => commitVocabularyImport(writtenConflict, { ...empty, vocabularyEntries: [{ ...globalEntry, id: 'other' }] })).toThrow();
  });

  it('rejects a stale preview after saved spellings change', () => {
    const preview = previewVocabularyImport(file(), empty);
    expect(() => commitVocabularyImport(preview, { ...empty, vocabularyEntries: [globalEntry] })).toThrow('changed');
  });
});
