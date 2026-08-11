import { invoke } from '@tauri-apps/api/core';

export type MicrophonePreviewPhase = 'idle' | 'connecting' | 'active' | 'stopping' | 'error';
export type MicrophoneSignalClassification =
  | 'no_signal'
  | 'too_quiet'
  | 'signal_detected'
  | 'clipping';

export interface MicrophonePreviewStatus {
  previewId: number | null;
  state: MicrophonePreviewPhase;
  stillConnecting: boolean;
  errorKind: string | null;
  message: string | null;
}

export interface MicrophonePreviewLevel {
  previewId: number;
  rms: number;
  peak: number;
  classification: MicrophoneSignalClassification;
}

export type MicrophonePreviewVadDecision = 'speech_detected' | 'no_speech' | 'unavailable';

export interface MicrophonePreviewVad {
  previewId: number;
  sensitivity: number;
  decision: MicrophonePreviewVadDecision;
}

export const IDLE_MICROPHONE_PREVIEW: MicrophonePreviewStatus = {
  previewId: null,
  state: 'idle',
  stillConnecting: false,
  errorKind: null,
  message: null,
};

export function getMicrophonePreviewStatus(): Promise<MicrophonePreviewStatus> {
  return invoke('get_microphone_preview_status');
}

export function startMicrophonePreview(
  deviceId: string,
  vadSensitivity: number,
): Promise<MicrophonePreviewStatus> {
  return invoke('start_microphone_preview', { deviceId, vadSensitivity });
}

export function updateMicrophonePreviewVadSensitivity(
  previewId: number,
  vadSensitivity: number,
): Promise<boolean> {
  return invoke('update_microphone_preview_vad_sensitivity', { previewId, vadSensitivity });
}

export function stopMicrophonePreview(previewId: number): Promise<MicrophonePreviewStatus> {
  return invoke('stop_microphone_preview', { previewId });
}

export function cancelMicrophonePreview(previewId?: number | null): Promise<boolean> {
  return invoke('cancel_microphone_preview', { previewId: previewId ?? null });
}

export function microphoneLevelPercent(rms: number): number {
  if (!Number.isFinite(rms)) return 0;
  // Speech RMS is normally a small fraction of full scale. A mildly
  // nonlinear display keeps quiet speech visible without hiding headroom.
  return Math.round(Math.min(1, Math.sqrt(Math.max(0, rms))) * 100);
}

export function microphonePeakPercent(peak: number): number {
  if (!Number.isFinite(peak)) return 0;
  return Math.round(Math.min(1, Math.max(0, peak)) * 100);
}

/**
 * Move a displayed meter value toward the latest audio sample without exposing
 * every callback-sized jump. The exponential response is independent of the
 * display refresh rate and uses a faster attack than release so speech still
 * feels responsive while the bar settles smoothly.
 */
export function smoothMicrophoneMeterValue(
  current: number,
  target: number,
  elapsedMs: number,
  attackMs = 75,
  releaseMs = 300,
): number {
  const safeCurrent = Number.isFinite(current) ? Math.min(100, Math.max(0, current)) : 0;
  const safeTarget = Number.isFinite(target) ? Math.min(100, Math.max(0, target)) : 0;
  const safeElapsed = Number.isFinite(elapsedMs) ? Math.min(100, Math.max(0, elapsedMs)) : 0;
  if (safeElapsed === 0 || safeCurrent === safeTarget) return safeCurrent;

  const responseMs = safeTarget > safeCurrent ? attackMs : releaseMs;
  const alpha = 1 - Math.exp(-safeElapsed / Math.max(1, responseMs));
  return safeCurrent + ((safeTarget - safeCurrent) * alpha);
}

export function microphoneClassificationLabel(
  classification: MicrophoneSignalClassification,
): string {
  switch (classification) {
    case 'no_signal': return 'No signal';
    case 'too_quiet': return 'Too quiet';
    case 'signal_detected': return 'Signal detected';
    case 'clipping': return 'Clipping';
  }
}
