export type SmartDestinationStatus = 'current' | 'recent' | 'unavailable';

export interface SmartDestinationSelection {
  recordingId: number;
  status: SmartDestinationStatus;
  appName?: string;
}

const MAX_APP_NAME_LENGTH = 80;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f-\u009f]/;

function appNameFromPayload(value: unknown): string | undefined {
  if (
    typeof value !== 'string'
    || value.length === 0
    || value.length > MAX_APP_NAME_LENGTH
    || CONTROL_CHARACTER.test(value)
  ) return undefined;
  return value;
}

export function recordingIdFromPayload(value: unknown): number | null {
  if (!value || typeof value !== 'object') return null;
  const recordingId = (value as Record<string, unknown>).recordingId;
  return typeof recordingId === 'number'
    && Number.isSafeInteger(recordingId)
    && recordingId > 0
    ? recordingId
    : null;
}

export function smartDestinationSelectionFromPayload(
  value: unknown,
): SmartDestinationSelection | null {
  const recordingId = recordingIdFromPayload(value);
  if (recordingId === null) return null;
  const payload = value as Record<string, unknown>;
  if (
    payload.status !== 'current'
    && payload.status !== 'recent'
    && payload.status !== 'unavailable'
  ) return null;
  const selection: SmartDestinationSelection = {
    recordingId,
    status: payload.status,
  };
  if (selection.status !== 'unavailable') {
    const appName = appNameFromPayload(payload.appName);
    if (appName !== undefined) selection.appName = appName;
  }
  return selection;
}

export function smartDestinationLabel(
  status: SmartDestinationStatus,
  appName?: string | null,
): string {
  if (status !== 'unavailable' && appName) return `To ${appName}`;
  switch (status) {
    case 'current': return 'Current text field';
    case 'recent': return 'Recent text field';
    case 'unavailable': return 'Clipboard only';
  }
}
