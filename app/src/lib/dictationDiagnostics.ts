import { invoke } from '@tauri-apps/api/core';

export type DictationCaptureArmStatusV1 =
  | { state: 'unarmed' }
  | { state: 'armed'; expiresAtMs: number }
  | { state: 'capturing'; recordingId: number };

export interface DictationCaptureSummaryV1 {
  captureId: string;
  recordingId: number;
  capturedAtMs: number;
  expiresAtMs: number;
  outcome: string;
  hasContent: boolean;
}

export interface BoundedPrivateTextV1 {
  text: string;
  truncated: boolean;
}

export type DictationCaptureResultV1 =
  | {
    kind: 'success';
    rawText: BoundedPrivateTextV1;
    finalText: BoundedPrivateTextV1;
    modelId: string;
    totalMs: number;
  }
  | { kind: 'noContent'; outcome: string; errorCode: string };

export interface DictationCaptureV1 {
  schemaVersion: 1;
  captureId: string;
  recordingId: number;
  capturedAtMs: number;
  expiresAtMs: number;
  result: DictationCaptureResultV1;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isString(value: unknown): value is string {
  return typeof value === 'string';
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isBoundedText(value: unknown): value is BoundedPrivateTextV1 {
  return isRecord(value)
    && isString(value.text)
    && typeof value.truncated === 'boolean';
}

export function isDictationCaptureArmStatusV1(
  value: unknown,
): value is DictationCaptureArmStatusV1 {
  if (!isRecord(value) || !isString(value.state)) return false;
  if (value.state === 'unarmed') return true;
  if (value.state === 'armed') return isNumber(value.expiresAtMs);
  if (value.state === 'capturing') return isNumber(value.recordingId);
  return false;
}

export function isDictationCaptureSummaryV1(
  value: unknown,
): value is DictationCaptureSummaryV1 {
  return isRecord(value)
    && isString(value.captureId)
    && isNumber(value.recordingId)
    && isNumber(value.capturedAtMs)
    && isNumber(value.expiresAtMs)
    && isString(value.outcome)
    && typeof value.hasContent === 'boolean';
}

function isDictationCaptureResultV1(value: unknown): value is DictationCaptureResultV1 {
  if (!isRecord(value) || !isString(value.kind)) return false;
  if (value.kind === 'success') {
    return isBoundedText(value.rawText)
      && isBoundedText(value.finalText)
      && isString(value.modelId)
      && isNumber(value.totalMs);
  }
  return value.kind === 'noContent'
    && isString(value.outcome)
    && isString(value.errorCode);
}

export function isDictationCaptureV1(value: unknown): value is DictationCaptureV1 {
  return isRecord(value)
    && value.schemaVersion === 1
    && isString(value.captureId)
    && isNumber(value.recordingId)
    && isNumber(value.capturedAtMs)
    && isNumber(value.expiresAtMs)
    && isDictationCaptureResultV1(value.result);
}

async function readArmStatus(command: string): Promise<DictationCaptureArmStatusV1> {
  const value: unknown = await invoke(command);
  if (!isDictationCaptureArmStatusV1(value)) {
    throw new Error('Private capture status returned an unsupported format.');
  }
  return value;
}

export function getDictationCaptureStatus(): Promise<DictationCaptureArmStatusV1> {
  return readArmStatus('get_dictation_diagnostic_capture_status');
}

export function armNextDictationCapture(): Promise<DictationCaptureArmStatusV1> {
  return readArmStatus('arm_next_dictation_diagnostic_capture');
}

export function disarmNextDictationCapture(): Promise<DictationCaptureArmStatusV1> {
  return readArmStatus('disarm_next_dictation_diagnostic_capture');
}

export async function listDictationCaptures(): Promise<DictationCaptureSummaryV1[]> {
  const value: unknown = await invoke('list_dictation_diagnostic_captures');
  if (!Array.isArray(value) || !value.every(isDictationCaptureSummaryV1)) {
    throw new Error('Private capture list returned an unsupported format.');
  }
  return value;
}

export async function getDictationCapture(captureId: string): Promise<DictationCaptureV1 | null> {
  const value: unknown = await invoke('get_dictation_diagnostic_capture', { captureId });
  if (value === null) return null;
  if (!isDictationCaptureV1(value)) {
    throw new Error('Private capture returned an unsupported format.');
  }
  return value;
}

export async function deleteDictationCapture(captureId: string): Promise<void> {
  await invoke('delete_dictation_diagnostic_capture', { captureId });
}

export async function uploadDictationCapture(captureId: string): Promise<void> {
  await invoke('upload_dictation_diagnostic_capture', { captureId });
}
