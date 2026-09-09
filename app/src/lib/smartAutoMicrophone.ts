import { invoke } from '@tauri-apps/api/core';
import type { SmartAutoMicrophoneRequest } from './settings';

export type SmartAutoMicrophoneReadyReason =
  | 'current_verified'
  | 'previous_verified_rollback'
  | 'preferred_approved'
  | 'approved_macos_default'
  | 'approved_external_fallback'
  | 'approved_continuity_fallback';

export type SmartAutoMicrophoneStatus =
  | {
    state: 'ready';
    deviceId: string;
    reason: SmartAutoMicrophoneReadyReason;
    validForMs: number;
  }
  | {
    state: 'blocked';
    message: string;
  };

const READY_REASONS = new Set<string>([
  'current_verified',
  'previous_verified_rollback',
  'preferred_approved',
  'approved_macos_default',
  'approved_external_fallback',
  'approved_continuity_fallback',
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isBoundedText(value: unknown, maxLength: number): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= maxLength;
}

function isReadyReason(value: unknown): value is SmartAutoMicrophoneReadyReason {
  return typeof value === 'string' && READY_REASONS.has(value);
}

export function parseSmartAutoMicrophoneStatus(value: unknown): SmartAutoMicrophoneStatus | null {
  if (!isRecord(value)) return null;
  if (value.state === 'ready') {
    if (!isBoundedText(value.deviceId, 4096)
      || !isReadyReason(value.reason)
      || typeof value.validForMs !== 'number'
      || !Number.isSafeInteger(value.validForMs)
      || value.validForMs <= 0
      || value.validForMs > 120_000) return null;
    return {
      state: 'ready',
      deviceId: value.deviceId,
      reason: value.reason,
      validForMs: value.validForMs,
    };
  }
  if (value.state === 'blocked' && isBoundedText(value.message, 1024)) {
    return { state: 'blocked', message: value.message };
  }
  return null;
}

export async function getSmartAutoMicrophoneStatus(
  smartAuto: SmartAutoMicrophoneRequest,
): Promise<SmartAutoMicrophoneStatus> {
  const status = parseSmartAutoMicrophoneStatus(
    await invoke<unknown>('get_smart_auto_microphone_status', { smartAuto }),
  );
  if (!status) throw new Error('Murmur returned an unsupported Smart Auto microphone status.');
  return status;
}

export function smartAutoMicrophoneReasonLabel(reason: SmartAutoMicrophoneReadyReason): string {
  switch (reason) {
    case 'current_verified': return 'current verified microphone retained';
    case 'previous_verified_rollback': return 'previous verified microphone restored';
    case 'preferred_approved': return 'preferred approved microphone';
    case 'approved_macos_default': return 'approved macOS default';
    case 'approved_external_fallback': return 'approved external fallback';
    case 'approved_continuity_fallback': return 'approved iPhone fallback';
  }
}
