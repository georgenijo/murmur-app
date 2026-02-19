export type DictationStatus = 'idle' | 'recording' | 'processing';

export const VALID_STATUSES = ['idle', 'recording', 'processing'] as const;
export function isDictationStatus(v: unknown): v is DictationStatus {
  return typeof v === 'string' && (VALID_STATUSES as readonly string[]).includes(v);
}
