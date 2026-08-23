export type ModelDownloadPhase =
  | 'downloading'
  | 'installing'
  | 'preparing'
  | 'repairing'
  | 'initializing'
  | 'validating';

export interface ModelDownloadProgress {
  modelName?: string;
  attemptId?: number;
  received: number;
  total: number;
  phase?: ModelDownloadPhase;
  repeatedRepair?: boolean;
}

export function correlatedModelDownloadAttempt(
  progress: ModelDownloadProgress,
  modelName: string,
  activeAttemptId: number | null,
): number | undefined {
  if (
    progress.modelName !== modelName
    || !Number.isSafeInteger(progress.attemptId)
    || (progress.attemptId ?? 0) <= 0
  ) return undefined;
  if (activeAttemptId !== null && activeAttemptId !== progress.attemptId) return undefined;
  return progress.attemptId;
}

export function modelDownloadPercent(progress: ModelDownloadProgress): number | null {
  if ((progress.phase && progress.phase !== 'downloading') || progress.total <= 0) return null;
  return Math.min(100, Math.round((progress.received / progress.total) * 100));
}

export function modelDownloadLabel(progress: ModelDownloadProgress): string {
  switch (progress.phase) {
    case 'preparing':
      return 'Preparing secure installer...';
    case 'repairing':
      return progress.repeatedRepair
        ? 'Repairing incomplete install again...'
        : 'Repairing incomplete install...';
    case 'initializing':
      return 'Installing Core ML model...';
    case 'validating':
      return 'Validating installation...';
    case 'installing':
      return 'Installing...';
    default:
      return 'Downloading...';
  }
}
