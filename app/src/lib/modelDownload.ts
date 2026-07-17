export type ModelDownloadPhase = 'downloading' | 'installing';

export interface ModelDownloadProgress {
  received: number;
  total: number;
  phase?: ModelDownloadPhase;
}

export function modelDownloadPercent(progress: ModelDownloadProgress): number | null {
  if (progress.phase === 'installing' || progress.total <= 0) return null;
  return Math.min(100, Math.round((progress.received / progress.total) * 100));
}

export function modelDownloadLabel(progress: ModelDownloadProgress): string {
  return progress.phase === 'installing' ? 'Installing...' : 'Downloading...';
}
