import type { DictationStatus } from './types';

export const HOTKEY_MISS_FLASH_MS = 500;

export interface HotkeyTapRejectedPayload {
  reason: string;
  mode: string;
}

export function isHotkeyTapRejectedPayload(value: unknown): value is HotkeyTapRejectedPayload {
  if (!value || typeof value !== 'object') return false;
  const payload = value as Record<string, unknown>;
  return typeof payload.reason === 'string' && typeof payload.mode === 'string';
}

export function shouldShowHotkeyMissFeedback(
  enabled: boolean,
  status: DictationStatus,
  payload: HotkeyTapRejectedPayload,
): boolean {
  return enabled
    && status === 'idle'
    && payload.reason === 'second_tap_expired'
    && (payload.mode === 'double_tap' || payload.mode === 'both');
}
