/**
 * Opt-in transcript rows for the ⌘K palette (`paletteTranscriptSearch`).
 *
 * Pure: takes the in-memory history and the palette query and returns rows
 * that copy one transcript when run. Matching is the History search rule
 * (case-insensitive, every word must appear), so a query finds the same
 * transcripts in both places. Nothing is shown until the query has at least
 * `MIN_TRANSCRIPT_QUERY_LENGTH` characters, so opening the palette never
 * puts transcript text on screen by itself.
 */
import type { PaletteCommand } from './commandPalette';
import {
  filterHistory,
  formatTimestamp,
  historyDayLabel,
  searchTokens,
  sortForDisplay,
  type HistoryEntry,
} from './history';

export const TRANSCRIPT_SECTION = 'Transcripts';
export const MIN_TRANSCRIPT_QUERY_LENGTH = 2;
export const MAX_TRANSCRIPT_RESULTS = 5;
export const TRANSCRIPT_SNIPPET_LENGTH = 96;
/** Characters of context kept before the first match when the snippet is shifted. */
const LEAD_CONTEXT = 24;

/**
 * One line of the transcript around its first match. Whitespace collapses to
 * single spaces; an ellipsis marks text cut from either end.
 */
export function transcriptSnippet(
  text: string,
  query: string,
  maxLength: number = TRANSCRIPT_SNIPPET_LENGTH,
): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  if (flat.length <= maxLength) return flat;
  const lower = flat.toLowerCase();
  let start = 0;
  // Lower-casing can change length for a few scripts; then indices no longer
  // line up, so keep the start of the text rather than guess.
  if (lower.length === flat.length) {
    const first = searchTokens(query)
      .map((token) => lower.indexOf(token))
      .filter((index) => index >= 0)
      .reduce((min, index) => Math.min(min, index), Number.POSITIVE_INFINITY);
    // The row truncates on the right at whatever width the dialog has, so the
    // match must sit near the start; a short tail is fine.
    if (Number.isFinite(first) && first > LEAD_CONTEXT + 8) {
      const target = first - LEAD_CONTEXT;
      // Start on a word boundary: after the first space at or past the target,
      // or at the match itself when there is none.
      const space = flat.indexOf(' ', target);
      start = space >= 0 && space < first ? space + 1 : first;
    }
  }
  const prefix = start > 0 ? '…' : '';
  const room = maxLength - prefix.length;
  let body = flat.slice(start, start + room);
  if (start + room < flat.length) {
    body = body.slice(0, room - 1);
    const space = body.lastIndexOf(' ');
    if (space > room / 2) body = body.slice(0, space);
    return `${prefix}${body.trimEnd()}…`;
  }
  return `${prefix}${body}`;
}

/** Newest matching transcripts, at most `limit`. Empty for a short query. */
export function matchTranscripts(
  entries: HistoryEntry[],
  query: string,
  limit: number = MAX_TRANSCRIPT_RESULTS,
): HistoryEntry[] {
  if (query.replace(/\s+/g, '').length < MIN_TRANSCRIPT_QUERY_LENGTH) return [];
  return sortForDisplay(filterHistory(entries, { query }))
    .filter((entry) => entry.text.trim().length > 0)
    .slice(0, limit);
}

/** Palette rows for matching transcripts; running one copies its full text. */
export function transcriptPaletteCommands(
  entries: HistoryEntry[],
  query: string,
  copy: (text: string) => void | Promise<void>,
): PaletteCommand[] {
  return matchTranscripts(entries, query).map((entry) => ({
    id: `transcript-${entry.id}`,
    title: transcriptSnippet(entry.text, query),
    section: TRANSCRIPT_SECTION,
    meta: `${historyDayLabel(entry.timestamp)} · ${formatTimestamp(entry.timestamp)}`,
    actionVerb: 'copy',
    highlightMatches: true,
    run: () => copy(entry.text),
  }));
}
