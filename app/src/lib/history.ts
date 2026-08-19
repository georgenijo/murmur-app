/** Where a history entry's text came from. */
import type { TeachingContext } from './correctAndTeach';
import { clearDurableBlob, HISTORY_STORE, saveDurableBlob } from './durableUserData';

export type HistorySource = 'recording' | 'file';

export interface HistoryInterruption {
  reason: string;
  deliveredSamples: number;
  durationMs: number;
}

export type HistoryStageOutcome = 'applied' | 'skipped' | 'fallback' | 'failed';

export interface HistoryStageResult {
  stage: string;
  outcome: HistoryStageOutcome;
  changed: boolean;
}

export interface HistoryRecordingContext {
  recordingId: number;
  modelId: string;
  /** Reserved for the reusable Mode model. Legacy/global dictation has no id. */
  modeId: string | null;
  /** The resolved profile label at recording start; absent for global settings. */
  profileId: string | null;
  stages: HistoryStageResult[];
}

export interface HistoryEntry {
  /** Absent only on in-memory legacy fixtures before migration. */
  schemaVersion?: 2;
  id: string;
  /** Final text delivered to the clipboard and shown in History. */
  text: string;
  /** Exact backend recognition before transforms. Absent on migrated entries. */
  rawText?: string;
  timestamp: number;
  duration: number; // recording duration in seconds
  /** Origin of the entry. Absent on entries saved before this field existed
   *  (treated as 'recording' when displayed). */
  source?: HistorySource;
  /** For file transcriptions, the source file's base name (for display). */
  sourceName?: string;
  /** Local recording-start scope metadata used only for explicit teaching. */
  teachingContext?: TeachingContext;
  /** Capture ended unexpectedly; the retained prefix was still transcribed. */
  interruption?: HistoryInterruption;
  /** Present for live dictations recorded with the v2 completion contract. */
  recording?: HistoryRecordingContext;
  /** Provenance for a new entry derived by reformatting retained raw text. */
  derived?: { sourceEntryId: string; modeId: string; createdAt: number; stages: HistoryStageResult[] };
}

export function addDerivedHistoryEntry(
  entries: HistoryEntry[], source: HistoryEntry, text: string, modeId: string,
  stages: HistoryStageResult[],
): HistoryEntry[] {
  const createdAt = Date.now();
  return trimHistory([...entries, {
    schemaVersion: 2,
    id: nextEntryId(),
    text,
    rawText: source.rawText,
    timestamp: createdAt,
    duration: source.duration,
    source: source.source ?? 'recording',
    ...(source.sourceName ? { sourceName: source.sourceName } : {}),
    derived: { sourceEntryId: source.id, modeId, createdAt, stages },
  }]);
}

/** Rolling cap on stored entries. */
const MAX_ENTRIES = 200;

/**
 * Drop the oldest entries beyond the cap, preserving the stored oldest-first
 * order. Index-based: ids are millisecond timestamps and two entries created
 * in the same millisecond can collide.
 */
export function trimHistory(entries: HistoryEntry[]): HistoryEntry[] {
  return entries.slice(-MAX_ENTRIES);
}

export function loadHistory(): HistoryEntry[] {
  try {
    const stored = localStorage.getItem(HISTORY_STORE.storageKey);
    if (stored) {
      const parsed = JSON.parse(stored);
      if (Array.isArray(parsed)) return trimHistory(parsed.map(migrateHistoryEntry));
    }
  } catch (e) {
    console.error('Failed to load history:', e);
  }
  return [];
}

function migrateHistoryEntry(value: Record<string, unknown>): HistoryEntry {
  if (value.schemaVersion === 2) return value as unknown as HistoryEntry;
  // Deliberately do not copy legacy delivered `text` into `rawText`: that
  // relationship cannot be reconstructed after the fact.
  return { ...value, schemaVersion: 2 } as unknown as HistoryEntry;
}

export function saveHistory(entries: HistoryEntry[]): void {
  try {
    const blob = JSON.stringify(trimHistory(entries));
    saveDurableBlob(HISTORY_STORE, blob);
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
  interruption?: HistoryInterruption,
  details?: { rawText: string; recording: HistoryRecordingContext },
): HistoryEntry[] {
  const newEntry: HistoryEntry = {
    schemaVersion: 2,
    id: nextEntryId(),
    text,
    timestamp: Date.now(),
    duration,
    source,
    ...(sourceName ? { sourceName } : {}),
    ...(teachingContext ? { teachingContext } : {}),
    ...(interruption ? { interruption } : {}),
    ...(details ? { rawText: details.rawText, recording: details.recording } : {}),
  };
  return trimHistory([...entries, newEntry]);
}

/**
 * Monotonic suffix so two entries created inside the same millisecond can't
 * share an id (which would make React keys and entry updates ambiguous).
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

export function clearHistory(): void {
  clearDurableBlob(HISTORY_STORE);
}

// ---------------------------------------------------------------------------
// Search and filtering
// ---------------------------------------------------------------------------

export type HistoryFilter = 'all' | 'recording' | 'file';

export const HISTORY_FILTER_OPTIONS: { value: HistoryFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'recording', label: 'Mic' },
  { value: 'file', label: 'File' },
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
 * Apply the source chip and the search box. Order is preserved, so the
 * caller still owns presentation order.
 */
export function filterHistory(
  entries: HistoryEntry[],
  options: { query?: string; filter?: HistoryFilter } = {},
): HistoryEntry[] {
  const tokens = searchTokens(options.query ?? '');
  const filter = options.filter ?? 'all';
  return entries.filter((entry) => {
    if ((filter === 'recording' || filter === 'file') && entrySource(entry) !== filter) return false;
    return matchesTokens(entry, tokens);
  });
}

/**
 * Presentation order: newest first. Stable for equal timestamps (falls back
 * to stored order).
 */
export function sortForDisplay(entries: HistoryEntry[]): HistoryEntry[] {
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => {
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
 * duration and source. `teachingContext` (bundle ids and project
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
      schema: 'murmur.history.v2',
      exportedAt: exportedAt.toISOString(),
      count: ordered.length,
      entries: ordered.map((entry) => ({
        schemaVersion: entry.schemaVersion ?? 2,
        id: entry.id,
        timestamp: entry.timestamp,
        durationSeconds: entry.duration,
        source: entrySource(entry),
        ...(entry.sourceName ? { sourceName: entry.sourceName } : {}),
        text: entry.text,
        ...(entry.rawText !== undefined ? { rawText: entry.rawText } : {}),
        ...(entry.recording ? { recording: entry.recording } : {}),
        ...(entry.derived ? { derived: entry.derived } : {}),
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
    ].join(' · ');
    return `[${meta}]\n${entry.text}`;
  });
  return `${blocks.join('\n\n').trimEnd()}\n`;
}
