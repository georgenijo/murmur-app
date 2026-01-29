export interface HistoryEntry {
  id: string;
  text: string;
  timestamp: number;
  duration: number; // recording duration in seconds
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

export function addHistoryEntry(entries: HistoryEntry[], text: string, duration: number): HistoryEntry[] {
  const newEntry: HistoryEntry = {
    id: Date.now().toString(),
    text,
    timestamp: Date.now(),
    duration,
  };
  return [...entries, newEntry].slice(-MAX_ENTRIES);
}

export function clearHistory(): void {
  localStorage.removeItem(STORAGE_KEY);
}

export function formatTimestamp(timestamp: number): string {
  const date = new Date(timestamp);
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}
