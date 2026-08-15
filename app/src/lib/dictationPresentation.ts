export type DictationPresentationStatusCode =
  | 'microphone_cleanup_in_progress'
  | 'microphone_initialization_failed'
  | 'microphone_cleanup_stalled'
  | 'microphone_interrupted';

export type DictationPresentationActionCode =
  | 'wait'
  | 'retry'
  | 'open_microphone_settings'
  | 'choose_microphone'
  | 'restart_app'
  | 'wait_for_partial_transcription';

export interface DictationPresentationPayload {
  recordingId: number;
  statusCode: DictationPresentationStatusCode;
  actionCode: DictationPresentationActionCode;
}

const PRESENTATION_PAIRS = new Set([
  'microphone_cleanup_in_progress:wait',
  'microphone_initialization_failed:retry',
  'microphone_initialization_failed:open_microphone_settings',
  'microphone_initialization_failed:choose_microphone',
  'microphone_cleanup_stalled:restart_app',
  'microphone_interrupted:retry',
  'microphone_interrupted:wait_for_partial_transcription',
]);

const AUDIO_FAILURE_KINDS = new Set([
  'permission_denied',
  'device_unavailable',
  'host_unavailable',
  'invalid_input',
  'resource_exhausted',
  'stream_invalidated',
  'unsupported_config',
  'backend_error',
  'protocol_error',
  'first_buffer_timeout',
  'initialization_timeout',
  'permission_prompt_timeout',
  'termination_unconfirmed',
  'worker_panicked',
  'signature_invalid',
]);

export function dictationPresentationFromPayload(
  value: unknown,
): DictationPresentationPayload | null {
  if (!value || typeof value !== 'object') return null;
  const payload = value as Record<string, unknown>;
  if (!Number.isSafeInteger(payload.recordingId) || (payload.recordingId as number) <= 0) {
    return null;
  }
  if (typeof payload.statusCode !== 'string' || typeof payload.actionCode !== 'string') {
    return null;
  }
  if (!PRESENTATION_PAIRS.has(`${payload.statusCode}:${payload.actionCode}`)) return null;
  return payload as unknown as DictationPresentationPayload;
}

export function initializationPresentationFromPayload(
  value: unknown,
): DictationPresentationPayload | null {
  const presentation = dictationPresentationFromPayload(value);
  if (!presentation || presentation.statusCode !== 'microphone_initialization_failed') {
    return null;
  }
  const payload = value as Record<string, unknown>;
  if (typeof payload.errorKind !== 'string' || !AUDIO_FAILURE_KINDS.has(payload.errorKind)) {
    return null;
  }
  const expectedAction = payload.errorKind === 'permission_denied'
    ? 'open_microphone_settings'
    : payload.errorKind === 'device_unavailable'
      ? 'choose_microphone'
      : 'retry';
  return presentation.actionCode === expectedAction ? presentation : null;
}

export function cleanupStalledPresentationFromPayload(
  value: unknown,
): DictationPresentationPayload | null {
  const presentation = dictationPresentationFromPayload(value);
  return presentation?.statusCode === 'microphone_cleanup_stalled'
    && presentation.actionCode === 'restart_app'
    ? presentation
    : null;
}

export function interruptedPresentationFromPayload(
  value: unknown,
): DictationPresentationPayload | null {
  const presentation = dictationPresentationFromPayload(value);
  if (!presentation || presentation.statusCode !== 'microphone_interrupted') return null;
  const autoTranscribe = (value as Record<string, unknown>).autoTranscribe;
  if (typeof autoTranscribe !== 'boolean') return null;
  const expectedAction = autoTranscribe ? 'wait_for_partial_transcription' : 'retry';
  return presentation.actionCode === expectedAction ? presentation : null;
}
