import { afterEach, describe, expect, it } from 'vitest';
import type { HistoryEntry } from './history';
import type { VocabularyEntry } from './settings';
import {
  DISMISSED_SUGGESTIONS_KEY, MAX_DISMISSED_SUGGESTIONS, loadDismissedSuggestions,
  saveDismissedSuggestions, suggestVocabularyTerms,
} from './vocabularySuggestions';

function history(...texts: string[]): HistoryEntry[] {
  return texts.map((text, index) => ({ id: String(index), text, timestamp: index, duration: 1 }));
}

function vocab(written: string, aliases: string[] = []): VocabularyEntry {
  return { id: written, written, aliases, enabled: true, scope: { kind: 'global' } };
}

const terms = (entries: HistoryEntry[], vocabulary: VocabularyEntry[] = [], dismissed?: Set<string>) =>
  suggestVocabularyTerms(entries, vocabulary, dismissed).map((suggestion) => suggestion.term);

describe('suggestVocabularyTerms', () => {
  it('suggests identifiers and mid-sentence names that recur across transcripts', () => {
    const entries = history(
      'Ask Priya to review the GitHub branch before the CPA deploy.',
      'The GitHub check failed again, ping Priya.',
      'Priya said the CPA gateway is fine.',
    );
    expect(suggestVocabularyTerms(entries, [])).toEqual([
      { term: 'Priya', transcripts: 3, occurrences: 3 },
      { term: 'CPA', transcripts: 2, occurrences: 2 },
      { term: 'GitHub', transcripts: 2, occurrences: 2 },
    ]);
  });

  it('requires at least two distinct transcripts', () => {
    expect(terms(history('Kubernetes and Kubernetes again in one note.', 'nothing else'))).toEqual([]);
  });

  it('ignores sentence-start capitals and words also said in lowercase', () => {
    const entries = history(
      'Thanks for the update. Great work today.',
      'Great, thanks. Thanks again!',
      'That was great, and Thanks for asking.',
    );
    expect(terms(entries)).toEqual([]);
  });

  it('ignores common capitalized words, weekdays, months and pure numbers', () => {
    const entries = history(
      'I think we meet on Monday in March at 9 AM, OK?',
      'Then I said Monday in March works, OK and 2026 too.',
    );
    expect(terms(entries)).toEqual([]);
  });

  it('excludes terms already saved as written forms or aliases in any scope', () => {
    const entries = history('Use Tauri with Zustand and Vite.', 'Tauri, Zustand, and Vite again.');
    const appScoped: VocabularyEntry = { ...vocab('Vite'), scope: { kind: 'app', bundleId: 'com.example' } };
    expect(terms(entries, [vocab('tauri'), vocab('State lib', ['Zustand']), appScoped])).toEqual([]);
  });

  it('excludes dismissed terms by normalized value', () => {
    const entries = history('ship with Zustand', 'more Zustand');
    expect(terms(entries, [], new Set(['zustand']))).toEqual([]);
  });

  it('skips emails, URLs and paths', () => {
    const entries = history(
      'mail Alex@Example.com or open https://Murmur.App/Docs today, see /Users/Shared/Notes',
      'again Alex@Example.com and https://Murmur.App/Docs plus /Users/Shared/Notes',
    );
    expect(terms(entries)).toEqual([]);
  });

  it('merges possessives and casing, showing the most frequent spelling', () => {
    const entries = history('the Tauri app', "Tauri's window", 'a TAURI build');
    expect(suggestVocabularyTerms(entries, [])).toEqual([{ term: 'Tauri', transcripts: 3, occurrences: 3 }]);
  });

  it('keeps identifier shapes even when also seen in lowercase', () => {
    const entries = history('push to github', 'the GitHub PR', 'GitHub actions');
    expect(terms(entries)).toEqual(['GitHub']);
  });

  it('recognizes snake_case and letter-digit identifiers', () => {
    const entries = history('call load_history then use gpt5', 'load_history and gpt5 again');
    expect(terms(entries)).toEqual(['gpt5', 'load_history']);
  });

  it('is deterministic and bounded by the limit', () => {
    const names = ['Zed', 'Ava', 'Bo', 'Cy', 'Dee'];
    const entries = history(...names.map((name) => `and ${name} too`), ...names.map((name) => `and ${name} too`));
    const first = suggestVocabularyTerms(entries, [], new Set(), 3).map((s) => s.term);
    expect(first).toEqual(['Ava', 'Bo', 'Cy']);
    expect(suggestVocabularyTerms([...entries].reverse(), [], new Set(), 3).map((s) => s.term)).toEqual(first);
  });

  it('tolerates malformed entries without text', () => {
    const entries = [{ id: 'x', timestamp: 0, duration: 0 } as unknown as HistoryEntry, ...history('and Zed', 'and Zed')];
    expect(terms(entries)).toEqual(['Zed']);
  });
});

describe('dismissed suggestion storage', () => {
  afterEach(() => localStorage.clear());

  it('round-trips, bounds, and removes the key when empty', () => {
    saveDismissedSuggestions(new Set(['zustand', 'tauri']));
    expect(loadDismissedSuggestions()).toEqual(new Set(['zustand', 'tauri']));
    const many = new Set(Array.from({ length: MAX_DISMISSED_SUGGESTIONS + 5 }, (_, index) => `t${index}`));
    saveDismissedSuggestions(many);
    const loaded = loadDismissedSuggestions();
    expect(loaded.size).toBe(MAX_DISMISSED_SUGGESTIONS);
    expect(loaded.has('t0')).toBe(false);
    saveDismissedSuggestions(new Set());
    expect(localStorage.getItem(DISMISSED_SUGGESTIONS_KEY)).toBeNull();
  });

  it('treats corrupt storage as empty', () => {
    localStorage.setItem(DISMISSED_SUGGESTIONS_KEY, '{not json');
    expect(loadDismissedSuggestions()).toEqual(new Set());
    localStorage.setItem(DISMISSED_SUGGESTIONS_KEY, '{"a":1}');
    expect(loadDismissedSuggestions()).toEqual(new Set());
  });
});
