import { useEffect, useReducer } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';

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
  latestRecordingId: number | null;
  text: string;
  chunkIndex: number;
  processedAudioMs: number;
  enabled: boolean;
  model: string;
  acceptedEventCount: number;
  rejectedEventCount: number;
  lastEventDecision: PartialTranscriptEventDecision | null;
  lastEventRecordingId: number | null;
  lastEventChunkIndex: number;
}

export type PartialTranscriptEventDecision =
  | 'accepted'
  | 'disabled'
  | 'no_active_session'
  | 'recording_id_mismatch'
  | 'out_of_order';

export interface ActiveRecordingSessionSnapshot {
  recordingId: number;
  status: 'recording' | 'processing';
}

export type PartialTranscriptAction =
  | { type: 'sessionStarted'; payload: RecordingSessionStartedPayload }
  | { type: 'partialReceived'; payload: PartialTranscriptPayload }
  | { type: 'sessionCleared'; payload: PartialTranscriptClearedPayload }
  | { type: 'settingsChanged'; enabled: boolean; model: string };

export function createPartialTranscriptState(
  enabled: boolean,
  model: string,
): PartialTranscriptState {
  return {
    activeRecordingId: null,
    latestRecordingId: null,
    text: '',
    chunkIndex: 0,
    processedAudioMs: 0,
    enabled,
    model,
    acceptedEventCount: 0,
    rejectedEventCount: 0,
    lastEventDecision: null,
    lastEventRecordingId: null,
    lastEventChunkIndex: 0,
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
      if (
        state.latestRecordingId !== null
        && action.payload.recordingId <= state.latestRecordingId
      ) {
        return state;
      }
      return {
        ...clearSession(state, action.payload.recordingId),
        latestRecordingId: action.payload.recordingId,
      };
    case 'partialReceived': {
      const { payload } = action;
      const decision = classifyPartialTranscriptEvent(state, payload);
      if (decision !== 'accepted') {
        return {
          ...state,
          rejectedEventCount: state.rejectedEventCount + 1,
          lastEventDecision: decision,
          lastEventRecordingId: payload.recordingId,
          lastEventChunkIndex: payload.chunkIndex,
        };
      }
      return {
        ...state,
        text: payload.text,
        chunkIndex: payload.chunkIndex,
        processedAudioMs: payload.processedAudioMs,
        acceptedEventCount: state.acceptedEventCount + 1,
        lastEventDecision: decision,
        lastEventRecordingId: payload.recordingId,
        lastEventChunkIndex: payload.chunkIndex,
      };
    }
    case 'sessionCleared':
      return action.payload.recordingId === state.activeRecordingId
        ? clearSession(state)
        : state;
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

export function classifyPartialTranscriptEvent(
  state: PartialTranscriptState,
  payload: PartialTranscriptPayload,
): PartialTranscriptEventDecision {
  if (!state.enabled) return 'disabled';
  if (state.activeRecordingId === null) return 'no_active_session';
  if (payload.recordingId !== state.activeRecordingId) return 'recording_id_mismatch';
  if (payload.chunkIndex <= state.chunkIndex) return 'out_of_order';
  return 'accepted';
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

export function isActiveRecordingSessionSnapshot(
  payload: unknown,
): payload is ActiveRecordingSessionSnapshot {
  if (!payload || typeof payload !== 'object') return false;
  const value = payload as Record<string, unknown>;
  return typeof value.recordingId === 'number'
    && Number.isSafeInteger(value.recordingId)
    && value.recordingId > 0
    && (value.status === 'recording' || value.status === 'processing');
}

export function isRecordingIdPayload(
  payload: unknown,
): payload is { recordingId: number } {
  if (!payload || typeof payload !== 'object') return false;
  const recordingId = (payload as Record<string, unknown>).recordingId;
  return typeof recordingId === 'number'
    && Number.isSafeInteger(recordingId)
    && recordingId > 0;
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
    if (!state.lastEventDecision) return;
    flog.info('overlay-preview', 'partial event handled', {
      decision: state.lastEventDecision,
      recordingId: state.lastEventRecordingId,
      activeRecordingId: state.activeRecordingId,
      recordingIdMatched: state.lastEventRecordingId === state.activeRecordingId,
      chunkIndex: state.lastEventChunkIndex,
      acceptedEventCount: state.acceptedEventCount,
      rejectedEventCount: state.rejectedEventCount,
    });
  }, [
    state.acceptedEventCount,
    state.rejectedEventCount,
  ]);

  useEffect(() => {
    let disposed = false;
    let unlisteners: Array<() => void> = [];

    const setupListeners = async () => {
      const registered = await Promise.all([
        listen<unknown>('recording-session-started', (event) => {
          if (isRecordingSessionStartedPayload(event.payload)) {
            flog.info('overlay-preview', 'recording session selected', {
              recordingId: event.payload.recordingId,
              source: 'event',
            });
            dispatch({ type: 'sessionStarted', payload: event.payload });
          }
        }),
        listen<unknown>('partial-transcript', (event) => {
          if (isPartialTranscriptPayload(event.payload)) {
            dispatch({ type: 'partialReceived', payload: event.payload });
          }
        }),
        listen<unknown>('partial-transcript-cleared', (event) => {
          if (isPartialTranscriptClearedPayload(event.payload)) {
            flog.info('overlay-preview', 'partial session clear received', {
              recordingId: event.payload.recordingId,
              reason: event.payload.reason,
            });
            dispatch({ type: 'sessionCleared', payload: event.payload });
          }
        }),
        listen<unknown>('recording-cancelled', (event) => {
          if (isRecordingIdPayload(event.payload)) {
            dispatch({
              type: 'sessionCleared',
              payload: {
                contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
                recordingId: event.payload.recordingId,
                reason: 'cancelled',
              },
            });
          }
        }),
        listen<unknown>('transcription-complete', (event) => {
          if (isRecordingIdPayload(event.payload)) {
            dispatch({
              type: 'sessionCleared',
              payload: {
                contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
                recordingId: event.payload.recordingId,
                reason: 'finalized',
              },
            });
          }
        }),
      ]);

      if (disposed) {
        registered.forEach((unlisten) => unlisten());
        return;
      }
      unlisteners = registered;
      flog.info('overlay-preview', 'listeners ready', { listenerCount: registered.length });

      try {
        const snapshot = await invoke<unknown>('get_active_recording_session');
        if (!disposed && isActiveRecordingSessionSnapshot(snapshot)) {
          flog.info('overlay-preview', 'recording session selected', {
            recordingId: snapshot.recordingId,
            status: snapshot.status,
            source: 'readiness_snapshot',
          });
          dispatch({
            type: 'sessionStarted',
            payload: {
              contractVersion: PARTIAL_TRANSCRIPT_CONTRACT_VERSION,
              recordingId: snapshot.recordingId,
            },
          });
        }
      } catch (error) {
        flog.warn('overlay-preview', 'active session reconciliation failed', {
          error: String(error),
        });
      }
    };

    setupListeners().catch((error) => {
      flog.error('overlay-preview', 'listener setup failed', { error: String(error) });
    });

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  return state;
}
