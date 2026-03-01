export interface TranscriptionMetric {
  id: string;
  timestamp: number;
  recordingDurationSecs: number;
  inferenceMs: number;
  wordCount: number;
  model: string;
}

const STORAGE_KEY = 'transcription-metrics';
const MAX_ENTRIES = 100;

export function loadMetrics(): TranscriptionMetric[] {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      return JSON.parse(stored);
    }
  } catch (e) {
    console.error('Failed to load metrics:', e);
  }
  return [];
}

export function saveMetrics(entries: TranscriptionMetric[]): void {
  try {
    const trimmed = entries.slice(-MAX_ENTRIES);
    localStorage.setItem(STORAGE_KEY, JSON.stringify(trimmed));
  } catch (e) {
    console.error('Failed to save metrics:', e);
  }
}

export function addMetric(entry: Omit<TranscriptionMetric, 'id' | 'timestamp'>): TranscriptionMetric[] {
  const metrics = loadMetrics();
  const newEntry: TranscriptionMetric = {
    id: Date.now().toString(),
    timestamp: Date.now(),
    ...entry,
  };
  const updated = [...metrics, newEntry].slice(-MAX_ENTRIES);
  saveMetrics(updated);
  return updated;
}

export function clearMetrics(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch (e) {
    console.error('Failed to clear metrics:', e);
  }
}
