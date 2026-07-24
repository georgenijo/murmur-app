import { invoke } from '@tauri-apps/api/core';

export interface TransformPhaseV1 {
  phase: string;
  outcome: string;
  durationMs: number | null;
  errorCode: string | null;
}

export interface TransformAttemptV1 {
  schemaVersion: 1;
  transformPassId: number;
  startedAtMs: number;
  finishedAtMs: number | null;
  outcome: string;
  selectionSource: string | null;
  selectionResult: string | null;
  selectionSizeBucket: string | null;
  rangeAvailable: boolean | null;
  boundsAvailable: boolean | null;
  instructionConfidenceBucket: string | null;
  modelWarmState: string | null;
  outputTokenCount: number | null;
  finishReason: string | null;
  processExitCode: number | null;
  processExitSignal: number | null;
  phases: TransformPhaseV1[];
}

export interface TransformAttemptListV1 {
  schemaVersion: 1;
  attempts: TransformAttemptV1[];
}

export interface CaptureArmStatusV1 {
  armed: boolean;
  expiresAtMs: number | null;
}

export interface DiagnosticCaptureSummaryV1 {
  captureId: string;
  transformPassId: number;
  capturedAtMs: number;
  expiresAtMs: number;
  outcome: string;
}

export interface DiagnosticCaptureV1 extends DiagnosticCaptureSummaryV1 {
  schemaVersion: 1;
  selection: string | null;
  instruction: string | null;
  output: string | null;
  phases: TransformPhaseV1[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNullable<T>(value: unknown, check: (candidate: unknown) => candidate is T): value is T | null {
  return value === null || check(value);
}

const isString = (value: unknown): value is string => typeof value === 'string';
const isNumber = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value);
const isBoolean = (value: unknown): value is boolean => typeof value === 'boolean';

export function isTransformPhaseV1(value: unknown): value is TransformPhaseV1 {
  return isRecord(value)
    && isString(value.phase)
    && isString(value.outcome)
    && isNullable(value.durationMs, isNumber)
    && isNullable(value.errorCode, isString);
}

export function isTransformAttemptV1(value: unknown): value is TransformAttemptV1 {
  return isRecord(value)
    && value.schemaVersion === 1
    && isNumber(value.transformPassId)
    && isNumber(value.startedAtMs)
    && isNullable(value.finishedAtMs, isNumber)
    && isString(value.outcome)
    && isNullable(value.selectionSource, isString)
    && isNullable(value.selectionResult, isString)
    && isNullable(value.selectionSizeBucket, isString)
    && isNullable(value.rangeAvailable, isBoolean)
    && isNullable(value.boundsAvailable, isBoolean)
    && isNullable(value.instructionConfidenceBucket, isString)
    && isNullable(value.modelWarmState, isString)
    && isNullable(value.outputTokenCount, isNumber)
    && isNullable(value.finishReason, isString)
    && isNullable(value.processExitCode, isNumber)
    && isNullable(value.processExitSignal, isNumber)
    && Array.isArray(value.phases)
    && value.phases.every(isTransformPhaseV1);
}

export function isDiagnosticCaptureV1(value: unknown): value is DiagnosticCaptureV1 {
  return isRecord(value)
    && value.schemaVersion === 1
    && isString(value.captureId)
    && isNumber(value.transformPassId)
    && isNumber(value.capturedAtMs)
    && isNumber(value.expiresAtMs)
    && isString(value.outcome)
    && isNullable(value.selection, isString)
    && isNullable(value.instruction, isString)
    && isNullable(value.output, isString)
    && Array.isArray(value.phases)
    && value.phases.every(isTransformPhaseV1);
}

export async function listTransformAttempts(): Promise<TransformAttemptV1[]> {
  const value: unknown = await invoke('list_transform_attempts', { limit: 100 });
  if (!isRecord(value)
    || value.schemaVersion !== 1
    || !Array.isArray(value.attempts)
    || !value.attempts.every(isTransformAttemptV1)) {
    throw new Error('Transform attempt diagnostics returned an unsupported format.');
  }
  return value.attempts;
}

export async function getCaptureArmStatus(): Promise<CaptureArmStatusV1> {
  const value: unknown = await invoke('get_transform_diagnostic_capture_status');
  if (!isRecord(value)
    || typeof value.armed !== 'boolean'
    || !isNullable(value.expiresAtMs, isNumber)) {
    throw new Error('Diagnostic capture status returned an unsupported format.');
  }
  return { armed: value.armed, expiresAtMs: value.expiresAtMs };
}

export async function armNextTransformCapture(): Promise<CaptureArmStatusV1> {
  const value: unknown = await invoke('arm_next_transform_diagnostic_capture');
  if (!isRecord(value)
    || value.armed !== true
    || !isNullable(value.expiresAtMs, isNumber)) {
    throw new Error('Diagnostic capture could not be armed.');
  }
  return { armed: true, expiresAtMs: value.expiresAtMs };
}

export async function listTransformCaptures(): Promise<DiagnosticCaptureSummaryV1[]> {
  const value: unknown = await invoke('list_transform_diagnostic_captures');
  if (!Array.isArray(value) || !value.every(candidate =>
    isRecord(candidate)
      && isString(candidate.captureId)
      && isNumber(candidate.transformPassId)
      && isNumber(candidate.capturedAtMs)
      && isNumber(candidate.expiresAtMs)
      && isString(candidate.outcome))) {
    throw new Error('Diagnostic capture list returned an unsupported format.');
  }
  return value as DiagnosticCaptureSummaryV1[];
}

export async function getTransformCapture(captureId: string): Promise<DiagnosticCaptureV1 | null> {
  const value: unknown = await invoke('get_transform_diagnostic_capture', { captureId });
  if (value === null) return null;
  if (!isDiagnosticCaptureV1(value)) {
    throw new Error('Diagnostic capture returned an unsupported format.');
  }
  return value;
}

export async function deleteTransformCapture(captureId: string): Promise<void> {
  await invoke('delete_transform_diagnostic_capture', { captureId });
}
