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
}

const STORAGE_KEY = 'dictation-history';
const MAX_ENTRIES = 50;

export function loadHistory(): HistoryEntry[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return JSON.parse(stored);
    }
  } catch (e) {
    console.error('Failed to load history:', e);
  }
  return [];
}

export function saveHistory(entries: HistoryEntry[]): void {
  try {
    // Keep only the last MAX_ENTRIES
    const trimmed = entries.slice(-MAX_ENTRIES);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(trimmed));
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
    id: Date.now().toString(),
    text,
    timestamp: Date.now(),
    duration,
    source,
    ...(sourceName ? { sourceName } : {}),
    ...(teachingContext ? { teachingContext } : {}),
  };
  return [...entries, newEntry].slice(-MAX_ENTRIES);
}

export function updateHistoryEntry(
  entries: HistoryEntry[],
  id: string,
  text: string,
): HistoryEntry[] {
  return entries.map((entry) => entry.id === id ? { ...entry, text } : entry);
}

export function clearHistory(): void {
  localStorage.removeItem(STORAGE_KEY);
}

export function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}
