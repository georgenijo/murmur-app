/** Where a history entry's text came from. */
import type { TeachingContext } from './correctAndTeach';

export type HistorySource = 'recording' | 'file';

export interface HistoryEntry {
  id: string;
  text: string;
  timestamp: number;
  duration: number; // recording duration in seconds
  /** Origin of the entry. Absent on entries saved before this field existed
   *  (treated as 'recording' when displayed). */
  source?: HistorySource;
  /** For file transcriptions, the source file's base name (for display). */
  sourceName?: string;
  /** Local recording-start scope metadata used only for explicit teaching. */
  teachingContext?: TeachingContext;
  /** User-pinned. Pinned entries are exempt from the rolling trim and from
   *  "Clear history"; absent on entries saved before this field existed. */
  pinned?: boolean;
}

const STORAGE_KEY = 'dictation-history';

/** Rolling cap on ordinary (unpinned) entries. */
const MAX_ENTRIES = 50;

/**
 * Hard ceiling on pinned entries. Pinned entries are exempt from the rolling
 * trim, so they need their own bound — otherwise a user who keeps pinning would
 * grow the localStorage blob without limit.
 */
export const MAX_PINNED_ENTRIES = 25;

export function isPinned(entry: HistoryEntry): boolean {
  return entry.pinned === true;
}

/**
 * Drop the oldest entries beyond the caps, keeping pinned and unpinned entries
 * under independent budgets and preserving the stored oldest-first order.
 *
 * Indices, not ids, decide what survives: ids are millisecond timestamps and
 * two entries created in the same millisecond can collide.
 */
export function trimHistory(entries: HistoryEntry[]): HistoryEntry[] {
  let pinnedKept = 0;
  let unpinnedKept = 0;
  const keep = new Array<boolean>(entries.length).fill(false);
  for (let i = entries.length - 1; i >= 0; i--) {
    if (isPinned(entries[i])) {
      if (pinnedKept < MAX_PINNED_ENTRIES) {
        keep[i] = true;
        pinnedKept++;
      }
    } else if (unpinnedKept < MAX_ENTRIES) {
      keep[i] = true;
      unpinnedKept++;
    }
  }
  return entries.filter((_, index) => keep[index]);
}

export function loadHistory(): HistoryEntry[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored);
      if (Array.isArray(parsed)) return parsed as HistoryEntry[];
    }
  } catch (e) {
    console.error('Failed to load history:', e);
  }
  return [];
}

export function saveHistory(entries: HistoryEntry[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(trimHistory(entries)));
  } catch (e) {
    console.error('Failed to save history:', e);
  }
}

export function addHistoryEntry(
  entries: HistoryEntry[],
  text: string,
  duration: number,
  source: HistorySource = 'recording',
  sourceName?: string,
  teachingContext?: TeachingContext,
): HistoryEntry[] {
  const newEntry: HistoryEntry = {
    id: nextEntryId(),
    text,
    timestamp: Date.now(),
    duration,
    source,
    ...(sourceName ? { sourceName } : {}),
    ...(teachingContext ? { teachingContext } : {}),
  };
  return trimHistory([...entries, newEntry]);
}

/**
 * Monotonic suffix so two entries created inside the same millisecond can't
 * share an id (which would make React keys and pin toggles ambiguous).
 */
let entrySequence = 0;
function nextEntryId(): string {
  entrySequence += 1;
  return `${Date.now()}-${entrySequence}`;
}

export function updateHistoryEntry(
  entries: HistoryEntry[],
  id: string,
  text: string,
): HistoryEntry[] {
  return entries.map((entry) => entry.id === id ? { ...entry, text } : entry);
}

/** How many more entries may be pinned before the ceiling is reached. */
export function remainingPinSlots(entries: HistoryEntry[]): number {
  return Math.max(0, MAX_PINNED_ENTRIES - entries.filter(isPinned).length);
}

/**
 * Toggle one entry's pinned flag. Pinning past `MAX_PINNED_ENTRIES` is refused
 * — the same array is returned unchanged so callers can detect the no-op by
 * identity and explain the cap instead of silently dropping the request.
 * Unpinning is always allowed.
 */
export function togglePinned(entries: HistoryEntry[], id: string): HistoryEntry[] {
  const target = entries.find((entry) => entry.id === id);
  if (!target) return entries;
  if (!isPinned(target) && remainingPinSlots(entries) === 0) return entries;
  return entries.map((entry) =>
    entry.id === id ? { ...entry, pinned: !isPinned(entry) } : entry);
}

/** Remove every unpinned entry, keeping pinned ones in order. */
export function removeUnpinned(entries: HistoryEntry[]): HistoryEntry[] {
  return entries.filter(isPinned);
}

export function clearHistory(): void {
  localStorage.removeItem(STORAGE_KEY);
}

// ---------------------------------------------------------------------------
// Search and filtering
// ---------------------------------------------------------------------------

export type HistoryFilter = 'all' | 'recording' | 'file' | 'pinned';

export const HISTORY_FILTER_OPTIONS: { value: HistoryFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'recording', label: 'Mic' },
  { value: 'file', label: 'File' },
  { value: 'pinned', label: 'Pinned' },
];

export function entrySource(entry: HistoryEntry): HistorySource {
  return entry.source ?? 'recording';
}

/** Split a raw search box value into the tokens every match must contain. */
export function searchTokens(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

function matchesTokens(entry: HistoryEntry, tokens: string[]): boolean {
  if (tokens.length === 0) return true;
  const haystack = `${entry.text} ${entry.sourceName ?? ''}`.toLowerCase();
  return tokens.every((token) => haystack.includes(token));
}

/**
 * Apply the source/pinned chip and the search box. Order is preserved, so the
 * caller still owns presentation order.
 */
export function filterHistory(
  entries: HistoryEntry[],
  options: { query?: string; filter?: HistoryFilter } = {},
): HistoryEntry[] {
  const tokens = searchTokens(options.query ?? '');
  const filter = options.filter ?? 'all';
  return entries.filter((entry) => {
    if (filter === 'pinned' && !isPinned(entry)) return false;
    if ((filter === 'recording' || filter === 'file') && entrySource(entry) !== filter) return false;
    return matchesTokens(entry, tokens);
  });
}

/**
 * Presentation order: pinned entries first, then newest first inside each
 * group. Stable for equal timestamps (falls back to stored order).
 */
export function sortForDisplay(entries: HistoryEntry[]): HistoryEntry[] {
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => {
      const pinDelta = Number(isPinned(b.entry)) - Number(isPinned(a.entry));
      if (pinDelta !== 0) return pinDelta;
      const timeDelta = b.entry.timestamp - a.entry.timestamp;
      if (timeDelta !== 0) return timeDelta;
      return b.index - a.index;
    })
    .map(({ entry }) => entry);
}

export interface MatchSegment {
  text: string;
  match: boolean;
}

/** Bound on highlighted ranges per entry so a one-character query can't emit
 *  thousands of spans for a long transcript. */
const MAX_HIGHLIGHT_RANGES = 200;

/**
 * Split `text` into alternating plain/matched segments for every search token.
 * Overlapping and adjacent matches are merged, so highlighting "the there"
 * against "there" produces one range rather than two nested ones.
 */
export function matchSegments(text: string, query: string): MatchSegment[] {
  const tokens = searchTokens(query);
  if (tokens.length === 0 || text.length === 0) return [{ text, match: false }];

  const haystack = text.toLowerCase();
  const ranges: [number, number][] = [];
  for (const token of tokens) {
    let from = 0;
    while (ranges.length < MAX_HIGHLIGHT_RANGES) {
      const at = haystack.indexOf(token, from);
      if (at === -1) break;
      ranges.push([at, at + token.length]);
      from = at + token.length;
    }
  }
  if (ranges.length === 0) return [{ text, match: false }];

  ranges.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const merged: [number, number][] = [];
  for (const [start, end] of ranges) {
    const last = merged[merged.length - 1];
    if (last && start <= last[1]) {
      last[1] = Math.max(last[1], end);
    } else {
      merged.push([start, end]);
    }
  }

  const segments: MatchSegment[] = [];
  let cursor = 0;
  for (const [start, end] of merged) {
    if (start > cursor) segments.push({ text: text.slice(cursor, start), match: false });
    segments.push({ text: text.slice(start, end), match: true });
    cursor = end;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor), match: false });
  return segments;
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

export type HistoryExportFormat = 'markdown' | 'text' | 'json';

export const HISTORY_EXPORT_FORMATS: {
  value: HistoryExportFormat;
  label: string;
  extension: string;
}[] = [
  { value: 'markdown', label: 'Markdown', extension: 'md' },
  { value: 'text', label: 'Plain text', extension: 'txt' },
  { value: 'json', label: 'JSON', extension: 'json' },
];

export function exportExtension(format: HistoryExportFormat): string {
  return HISTORY_EXPORT_FORMATS.find((f) => f.value === format)?.extension ?? 'txt';
}

function pad(value: number): string {
  return String(value).padStart(2, '0');
}

export function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** Local `YYYY-MM-DD HH:MM:SS`, used in exports so they read the same on every
 *  machine regardless of locale formatting. */
export function formatExportTimestamp(timestamp: number): string {
  const d = new Date(timestamp);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} `
    + `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function formatDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  if (whole < 60) return `${whole}s`;
  return `${Math.floor(whole / 60)}m ${whole % 60}s`;
}

export function historyExportFileName(format: HistoryExportFormat, at: Date): string {
  const stamp = `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`
    + `-${pad(at.getHours())}${pad(at.getMinutes())}`;
  return `murmur-history-${stamp}.${exportExtension(format)}`;
}

function sourceLabel(entry: HistoryEntry): string {
  return entrySource(entry) === 'file' ? (entry.sourceName || 'File') : 'Mic';
}

/**
 * Render entries for export, newest first.
 *
 * Only what the user can already see is written out: text, timestamp,
 * duration, source and pin state. `teachingContext` (bundle ids and project
 * roots captured at recording start) is deliberately excluded — it is local
 * scope metadata for teaching, not part of a transcript the user shares.
 */
export function formatHistoryExport(
  entries: HistoryEntry[],
  format: HistoryExportFormat,
  exportedAt: Date,
): string {
  const ordered = sortForDisplay(entries);

  if (format === 'json') {
    return `${JSON.stringify({
      schema: 'murmur.history.v1',
      exportedAt: exportedAt.toISOString(),
      count: ordered.length,
      entries: ordered.map((entry) => ({
        id: entry.id,
        timestamp: entry.timestamp,
        durationSeconds: entry.duration,
        source: entrySource(entry),
        ...(entry.sourceName ? { sourceName: entry.sourceName } : {}),
        pinned: isPinned(entry),
        text: entry.text,
      })),
    }, null, 2)}\n`;
  }

  if (format === 'markdown') {
    const lines = [
      '# Murmur transcript history',
      '',
      `Exported ${formatExportTimestamp(exportedAt.getTime())} · ${ordered.length} `
        + `${ordered.length === 1 ? 'entry' : 'entries'}`,
      '',
    ];
    for (const entry of ordered) {
      lines.push(
        `## ${formatExportTimestamp(entry.timestamp)}`,
        '',
        `- Source: ${sourceLabel(entry)}`,
        `- Duration: ${formatDuration(entry.duration)}`,
        ...(isPinned(entry) ? ['- Pinned: yes'] : []),
        '',
        entry.text,
        '',
      );
    }
    return `${lines.join('\n').trimEnd()}\n`;
  }

  const blocks = ordered.map((entry) => {
    const meta = [
      formatExportTimestamp(entry.timestamp),
      sourceLabel(entry),
      formatDuration(entry.duration),
      ...(isPinned(entry) ? ['pinned'] : []),
    ].join(' · ');
    return `[${meta}]\n${entry.text}`;
  });
  return `${blocks.join('\n\n').trimEnd()}\n`;
}
