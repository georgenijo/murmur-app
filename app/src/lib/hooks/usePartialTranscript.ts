import { useEffect, useReducer } from 'react';
import { listen } from '@tauri-apps/api/event';

export const PARTIAL_TRANSCRIPT_CONTRACT_VERSION = 1 as const;

export interface RecordingSessionStartedPayload {
  contractVersion: typeof PARTIAL_TRANSCRIPT_CONTRACT_VERSION;
  recordingId: number;
}

export interface PartialTranscriptPayload extends RecordingSessionStartedPayload {
  text: string;
  chunkIndex: number;
  processedAudioMs: number;
}

export type PartialTranscriptClearReason =
  | 'cancelled'
  | 'error'
  | 'fallback'
  | 'finalized';

export interface PartialTranscriptClearedPayload extends RecordingSessionStartedPayload {
  reason: PartialTranscriptClearReason;
}

export interface PartialTranscriptState {
  activeRecordingId: number | null;
  text: string;
  chunkIndex: number;
  processedAudioMs: number;
  enabled: boolean;
  model: string;
}

export type PartialTranscriptAction =
  | { type: 'sessionStarted'; payload: RecordingSessionStartedPayload }
  | { type: 'partialReceived'; payload: PartialTranscriptPayload }
  | { type: 'sessionCleared'; payload: PartialTranscriptClearedPayload }
  | { type: 'clearActive' }
  | { type: 'settingsChanged'; enabled: boolean; model: string };

export function createPartialTranscriptState(
  enabled: boolean,
  model: string,
): PartialTranscriptState {
  return {
    activeRecordingId: null,
    text: '',
    chunkIndex: 0,
    processedAudioMs: 0,
    enabled,
    model,
  };
}

function clearSession(
  state: PartialTranscriptState,
  activeRecordingId: number | null = null,
): PartialTranscriptState {
  return {
    ...state,
    activeRecordingId,
    text: '',
    chunkIndex: 0,
    processedAudioMs: 0,
  };
}

export function partialTranscriptReducer(
  state: PartialTranscriptState,
  action: PartialTranscriptAction,
): PartialTranscriptState {
  switch (action.type) {
    case 'sessionStarted':
      return clearSession(state, action.payload.recordingId);
    case 'partialReceived': {
      const { payload } = action;
      if (
        !state.enabled ||
        payload.recordingId !== state.activeRecordingId ||
        payload.chunkIndex <= state.chunkIndex
      ) {
        return state;
      }
      return {
        ...state,
        text: payload.text,
        chunkIndex: payload.chunkIndex,
        processedAudioMs: payload.processedAudioMs,
      };
    }
    case 'sessionCleared':
      return action.payload.recordingId === state.activeRecordingId
        ? clearSession(state)
        : state;
    case 'clearActive':
      return clearSession(state);
    case 'settingsChanged': {
      if (action.model !== state.model) {
        return {
          ...clearSession(state),
          enabled: action.enabled,
          model: action.model,
        };
      }
      return {
        ...(action.enabled ? state : clearSession(state, state.activeRecordingId)),
        enabled: action.enabled,
      };
    }
  }
}

function hasCurrentContract(value: Record<string, unknown>): boolean {
  return value.contractVersion === PARTIAL_TRANSCRIPT_CONTRACT_VERSION;
}

export function isRecordingSessionStartedPayload(
  payload: unknown,
): payload is RecordingSessionStartedPayload {
  if (!payload || typeof payload !== 'object') return false;
  const value = payload as Record<string, unknown>;
  return hasCurrentContract(value)
    && typeof value.recordingId === 'number'
    && Number.isSafeInteger(value.recordingId)
    && value.recordingId > 0;
}

export function isPartialTranscriptPayload(
  payload: unknown,
): payload is PartialTranscriptPayload {
  if (!isRecordingSessionStartedPayload(payload)) return false;
  const value = payload as unknown as Record<string, unknown>;
  return typeof value.text === 'string'
    && typeof value.chunkIndex === 'number'
    && Number.isSafeInteger(value.chunkIndex)
    && value.chunkIndex > 0
    && typeof value.processedAudioMs === 'number'
    && Number.isFinite(value.processedAudioMs)
    && value.processedAudioMs >= 0;
}

export function isPartialTranscriptClearedPayload(
  payload: unknown,
): payload is PartialTranscriptClearedPayload {
  if (!isRecordingSessionStartedPayload(payload)) return false;
  const reason = (payload as PartialTranscriptClearedPayload).reason;
  return reason === 'cancelled'
    || reason === 'error'
    || reason === 'fallback'
    || reason === 'finalized';
}

export function usePartialTranscript(enabled: boolean, model: string) {
  const [state, dispatch] = useReducer(
    partialTranscriptReducer,
    createPartialTranscriptState(enabled, model),
  );

  useEffect(() => {
    dispatch({ type: 'settingsChanged', enabled, model });
  }, [enabled, model]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const register = (promise: Promise<() => void>) => {
      promise.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    };

    register(listen<unknown>('recording-session-started', (event) => {
      if (isRecordingSessionStartedPayload(event.payload)) {
        dispatch({ type: 'sessionStarted', payload: event.payload });
      }
    }));
    register(listen<unknown>('partial-transcript', (event) => {
      if (isPartialTranscriptPayload(event.payload)) {
        dispatch({ type: 'partialReceived', payload: event.payload });
      }
    }));
    register(listen<unknown>('partial-transcript-cleared', (event) => {
      if (isPartialTranscriptClearedPayload(event.payload)) {
        dispatch({ type: 'sessionCleared', payload: event.payload });
      }
    }));
    register(listen<unknown>('recording-cancelled', () => {
      dispatch({ type: 'clearActive' });
    }));
    register(listen<unknown>('transcription-complete', () => {
      dispatch({ type: 'clearActive' });
    }));
    register(listen<unknown>('recording-status-changed', (event) => {
      if (event.payload === 'idle') dispatch({ type: 'clearActive' });
    }));

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  return state;
}
