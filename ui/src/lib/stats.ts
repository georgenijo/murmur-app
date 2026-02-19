export interface DictationStats {
  totalWords: number;
  totalRecordings: number;
  totalDurationSeconds: number;
  wpmSamples: number[];
}

const DEFAULT_STATS: DictationStats = {
  totalWords: 0,
  totalRecordings: 0,
  totalDurationSeconds: 0,
  wpmSamples: [],
};

const STORAGE_KEY = 'dictation-stats';
const MAX_WPM_SAMPLES = 100;

export function loadStats(): DictationStats {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<DictationStats>;
      return { ...DEFAULT_STATS, ...parsed };
    }
  } catch (e) {
    console.error('Failed to load stats:', e);
  }
  return { ...DEFAULT_STATS };
}

export function saveStats(stats: DictationStats): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(stats));
  } catch (e) {
    console.error('Failed to save stats:', e);
  }
}

export function updateStats(text: string, durationSeconds: number): void {
  const stats = loadStats();
  const wordCount = text.trim() === '' ? 0 : text.trim().split(/\s+/).length;

  const newSamples = [...stats.wpmSamples];
  if (durationSeconds > 0 && wordCount > 0) {
    newSamples.push((wordCount / durationSeconds) * 60);
    if (newSamples.length > MAX_WPM_SAMPLES) {
      newSamples.splice(0, newSamples.length - MAX_WPM_SAMPLES);
    }
  }

  saveStats({
    totalWords: stats.totalWords + wordCount,
    totalRecordings: stats.totalRecordings + 1,
    totalDurationSeconds: stats.totalDurationSeconds + durationSeconds,
    wpmSamples: newSamples,
  });
}

export function resetStats(): void {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch (e) {
    console.error('Failed to reset stats:', e);
  }
}

export function getWPM(stats: DictationStats): number {
  if (stats.wpmSamples.length === 0) return 0;
  const sum = stats.wpmSamples.reduce((a, b) => a + b, 0);
  return Math.round(sum / stats.wpmSamples.length);
}

export function getApproxTokens(stats: DictationStats): number {
  return Math.round(stats.totalWords * 1.3);
}
