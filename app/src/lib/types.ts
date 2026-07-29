export type DictationStatus = 'idle' | 'starting' | 'recording' | 'recovering' | 'processing';

export const VALID_STATUSES = ['idle', 'starting', 'recording', 'recovering', 'processing'] as const;
export function isDictationStatus(v: unknown): v is DictationStatus {
  return typeof v === 'string' && (VALID_STATUSES as readonly string[]).includes(v);
}
