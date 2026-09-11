import { invoke } from '@tauri-apps/api/core';
import { smartAutoMicrophoneRequest, type Settings, type SmartAutoMicrophoneRequest } from './settings';

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
    validForMs: number | null;
  }
  | {
    state: 'blocked';
    message: string;
    retryAfterMs: number | null;
  }
  | {
    state: 'probing';
    deviceId: string;
    phase: 'connecting' | 'verifying' | 'stopping';
    remainingMs: number;
  };

export type SmartAutoProbePolicy =
  | { enabled: false }
  | { enabled: true; request: SmartAutoMicrophoneRequest };

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
      || (value.validForMs !== null && (
        typeof value.validForMs !== 'number'
        || !Number.isSafeInteger(value.validForMs)
        || value.validForMs <= 0
        || value.validForMs > 120_000
      ))
      || (value.validForMs === null && (
        value.reason === 'current_verified' || value.reason === 'previous_verified_rollback'
      ))) return null;
    return {
      state: 'ready',
      deviceId: value.deviceId,
      reason: value.reason,
      validForMs: value.validForMs,
    };
  }
  if (value.state === 'blocked' && isBoundedText(value.message, 1024)) {
    const retryAfterMs = value.retryAfterMs ?? null;
    if (retryAfterMs !== null && (
      typeof retryAfterMs !== 'number'
      || !Number.isSafeInteger(retryAfterMs)
      || retryAfterMs < 1
      || retryAfterMs > 900_000
    )) return null;
    return { state: 'blocked', message: value.message, retryAfterMs };
  }
  if (value.state === 'probing') {
    const phase = value.phase;
    if (!isBoundedText(value.deviceId, 4096)
      || (phase !== 'connecting' && phase !== 'verifying' && phase !== 'stopping')
      || typeof value.remainingMs !== 'number'
      || !Number.isSafeInteger(value.remainingMs)
      || value.remainingMs < 0
      || value.remainingMs > 75_000) return null;
    return {
      state: 'probing',
      deviceId: value.deviceId,
      phase,
      remainingMs: value.remainingMs,
    };
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

export function smartAutoProbePolicy(
  settings: Pick<Settings,
    'disabled'
    | 'microphone'
    | 'smartAutoMicrophoneEnabled'
    | 'smartAutoProbeEnabled'
    | 'smartAutoApprovedDeviceIds'
    | 'smartAutoPreferredDeviceIds'
    | 'smartAutoAllowContinuity'>,
): SmartAutoProbePolicy {
  const request = smartAutoMicrophoneRequest(settings);
  return !settings.disabled && settings.microphone === 'system_default' && settings.smartAutoProbeEnabled && request
    ? { enabled: true, request }
    : { enabled: false };
}

export async function configureSmartAutoProbe(policy: SmartAutoProbePolicy): Promise<number> {
  const generation = await invoke<unknown>('configure_smart_auto_probe', { policy });
  if (typeof generation !== 'number' || !Number.isSafeInteger(generation) || generation < 0) {
    throw new Error('Murmur returned an unsupported Smart Auto probe generation.');
  }
  return generation;
}

export async function retrySmartAutoProbe(): Promise<number> {
  const generation = await invoke<unknown>('retry_smart_auto_probe');
  if (typeof generation !== 'number' || !Number.isSafeInteger(generation) || generation < 0) {
    throw new Error('Murmur returned an unsupported Smart Auto probe generation.');
  }
  return generation;
}

export function smartAutoProbePhaseLabel(phase: 'connecting' | 'verifying' | 'stopping'): string {
  switch (phase) {
    case 'connecting': return 'connecting';
    case 'verifying': return 'checking signal';
    case 'stopping': return 'finishing';
  }
}

export function smartAutoMicrophoneReasonLabel(reason: SmartAutoMicrophoneReadyReason): string {
  switch (reason) {
    case 'current_verified': return 'already active with recent signal';
    case 'previous_verified_rollback': return 'previously active with recent signal';
    case 'preferred_approved': return 'your preferred included microphone';
    case 'approved_macos_default': return 'an included microphone that matches the macOS default';
    case 'approved_external_fallback': return 'the first available included external microphone';
    case 'approved_continuity_fallback': return 'the available included iPhone microphone';
  }
}
