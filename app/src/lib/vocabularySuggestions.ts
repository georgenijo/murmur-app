import type { HistoryEntry } from './history';
import type { VocabularyEntry } from './settings';
import { normalizeVocabularyValue } from './vocabulary';

/**
 * Suggested vocabulary terms, computed locally and in memory from saved
 * transcript history. Nothing here changes delivered text: a suggestion only
 * becomes a vocabulary entry when the user explicitly adds it.
 */
export interface VocabularySuggestion {
  /** The most frequent spelling seen in history. */
  term: string;
  /** Distinct transcripts that contain the term. */
  transcripts: number;
  /** Total occurrences across those transcripts. */
  occurrences: number;
}

export const MAX_VOCABULARY_SUGGESTIONS = 12;
export const MIN_SUGGESTION_TRANSCRIPTS = 2;
/** Only this much of each transcript is scanned, so a huge entry stays cheap. */
export const MAX_SCANNED_CHARS_PER_ENTRY = 20_000;
const MAX_TERM_CHARS = 48;

/**
 * Capitalized words that are ordinary English rather than personal vocabulary.
 * Weekdays and months are proper nouns every recognizer already spells.
 */
const COMMON_CAPITALIZED = new Set([
  'i', "i'm", "i'll", "i've", "i'd", 'ok', 'okay', 'am', 'pm', 'mr', 'mrs', 'ms', 'dr',
  'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday',
  'january', 'february', 'march', 'april', 'may', 'june', 'july', 'august',
  'september', 'october', 'november', 'december',
]);

type Shape = 'identifier' | 'capitalized' | 'lowercase' | 'other';

/** Words, with internal apostrophes, dots, hyphens and underscores kept. */
const TOKEN = /[\p{L}\p{N}](?:[\p{L}\p{N}_'’.-]*[\p{L}\p{N}])?/gu;

function shapeOf(token: string): Shape {
  const letters = token.replace(/[^\p{L}]/gu, '');
  if (!letters) return 'other';
  // camelCase / PascalCase with an inner capital (GitHub, macOS), snake_case,
  // letter+digit mixes (gpt5), and acronyms of two or more capitals (CPA).
  if (/\p{Ll}\p{Lu}/u.test(token) || /[\p{L}]_[\p{L}\p{N}]/u.test(token)) return 'identifier';
  if (/\p{L}/u.test(token) && /\p{N}/u.test(token)) return 'identifier';
  if (letters.length >= 2 && letters === letters.toUpperCase() && letters !== letters.toLowerCase()) {
    return 'identifier';
  }
  if (letters === letters.toLowerCase()) return 'lowercase';
  if (/^\p{Lu}/u.test(letters)) return 'capitalized';
  return 'other';
}

/** Strip a trailing English possessive so `Tauri's` counts as `Tauri`. */
function stripPossessive(token: string): string {
  return token.replace(/['’]s$/u, '');
}

interface TermStats {
  spellings: Map<string, number>;
  transcripts: Set<number>;
  occurrences: number;
  identifier: boolean;
  midSentenceCapitalized: boolean;
  lowercaseSeen: boolean;
}

/**
 * Rank terms that recur across saved transcripts and are not yet vocabulary.
 *
 * A term qualifies when it has an identifier shape, or when it is capitalized
 * in the middle of a sentence and never appears in lowercase (so "Thanks"
 * after a comma does not qualify). It must recur in at least
 * {@link MIN_SUGGESTION_TRANSCRIPTS} distinct transcripts. Existing written
 * forms and spoken aliases in every scope, dismissed terms, email addresses,
 * URLs and paths are excluded. Results are deterministic: most transcripts
 * first, then most occurrences, then alphabetical.
 */
export function suggestVocabularyTerms(
  history: readonly HistoryEntry[],
  vocabulary: readonly VocabularyEntry[],
  dismissed: ReadonlySet<string> = new Set(),
  limit = MAX_VOCABULARY_SUGGESTIONS,
): VocabularySuggestion[] {
  const known = new Set<string>();
  for (const entry of vocabulary) {
    known.add(normalizeVocabularyValue(entry.written));
    for (const alias of entry.aliases) known.add(normalizeVocabularyValue(alias));
  }

  const stats = new Map<string, TermStats>();
  history.forEach((entry, index) => {
    if (typeof entry.text !== 'string' || !entry.text) return;
    const text = entry.text.slice(0, MAX_SCANNED_CHARS_PER_ENTRY);
    let sentenceStart = true;
    let last = 0;
    for (const match of text.matchAll(TOKEN)) {
      const start = match.index ?? 0;
      const gap = text.slice(last, start);
      last = start + match[0].length;
      if (/[.!?:]\s*$|\n/u.test(gap)) sentenceStart = true;
      const precededBy = text[start - 1] ?? '';
      const followedBy = text[last] ?? '';
      const atSentenceStart = sentenceStart;
      sentenceStart = false;
      // Emails, URLs, paths and @mentions are not vocabulary.
      if (/[@/\\]/.test(precededBy) || /[@/\\]/.test(followedBy) || /^:\/\//.test(text.slice(last, last + 3))) continue;
      const token = stripPossessive(match[0]).replace(/\.+$/u, '');
      if (!token || Array.from(token).length > MAX_TERM_CHARS) continue;
      const shape = shapeOf(token);
      if (shape === 'other') continue;
      const key = normalizeVocabularyValue(token);
      if (/^\p{N}+$/u.test(key)) continue;
      let term = stats.get(key);
      if (!term) {
        term = {
          spellings: new Map(), transcripts: new Set(), occurrences: 0,
          identifier: false, midSentenceCapitalized: false, lowercaseSeen: false,
        };
        stats.set(key, term);
      }
      if (shape === 'lowercase') { term.lowercaseSeen = true; continue; }
      if (shape === 'identifier') term.identifier = true;
      if (shape === 'capitalized' && !atSentenceStart) term.midSentenceCapitalized = true;
      term.spellings.set(token, (term.spellings.get(token) ?? 0) + 1);
      term.transcripts.add(index);
      term.occurrences += 1;
    }
  });

  const suggestions: VocabularySuggestion[] = [];
  for (const [key, term] of stats) {
    if (term.transcripts.size < MIN_SUGGESTION_TRANSCRIPTS) continue;
    if (known.has(key) || dismissed.has(key) || COMMON_CAPITALIZED.has(key)) continue;
    const qualifies = term.identifier || (term.midSentenceCapitalized && !term.lowercaseSeen);
    if (!qualifies) continue;
    const spelling = [...term.spellings].sort((left, right) =>
      right[1] - left[1] || (left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0))[0][0];
    suggestions.push({ term: spelling, transcripts: term.transcripts.size, occurrences: term.occurrences });
  }
  suggestions.sort((left, right) =>
    right.transcripts - left.transcripts
    || right.occurrences - left.occurrences
    || (left.term < right.term ? -1 : left.term > right.term ? 1 : 0));
  return suggestions.slice(0, Math.max(0, limit));
}

export const DISMISSED_SUGGESTIONS_KEY = 'murmur.vocabularySuggestions.dismissed';
export const MAX_DISMISSED_SUGGESTIONS = 200;

/** Dismissed terms, normalized. Storage failure reads as an empty set. */
export function loadDismissedSuggestions(): Set<string> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(DISMISSED_SUGGESTIONS_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((value): value is string => typeof value === 'string')
      .slice(-MAX_DISMISSED_SUGGESTIONS));
  } catch {
    return new Set();
  }
}

/** Persist dismissals, keeping the newest {@link MAX_DISMISSED_SUGGESTIONS}. */
export function saveDismissedSuggestions(dismissed: ReadonlySet<string>): void {
  try {
    const values = [...dismissed].slice(-MAX_DISMISSED_SUGGESTIONS);
    if (values.length === 0) localStorage.removeItem(DISMISSED_SUGGESTIONS_KEY);
    else localStorage.setItem(DISMISSED_SUGGESTIONS_KEY, JSON.stringify(values));
  } catch {
    // Dismissals are a convenience; failing to store one never blocks the editor.
  }
}
