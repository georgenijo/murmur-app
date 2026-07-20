import { useEffect, useReducer, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { flog } from '../log';
import { isDictationStatus } from '../types';
import type { DictationStatus } from '../types';

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
  status: DictationStatus;
  lifecycleEventGeneration: number;
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
  | { type: 'sessionStarted'; payload: RecordingSessionStartedPayload; eventGeneration?: number }
  | {
      type: 'sessionSnapshot';
      payload: ActiveRecordingSessionSnapshot;
      expectedEventGeneration: number;
    }
  | { type: 'statusChanged'; status: DictationStatus; eventGeneration?: number }
  | { type: 'partialReceived'; payload: PartialTranscriptPayload }
  | { type: 'sessionCleared'; payload: PartialTranscriptClearedPayload; eventGeneration?: number }
  | { type: 'settingsChanged'; enabled: boolean; model: string; eventGeneration?: number };

export function createPartialTranscriptState(
  enabled: boolean,
  model: string,
): PartialTranscriptState {
  return {
    status: 'idle',
    lifecycleEventGeneration: 0,
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

function observeLifecycleEvent(
  state: PartialTranscriptState,
  eventGeneration?: number,
): PartialTranscriptState {
  if (
    eventGeneration === undefined
    || eventGeneration <= state.lifecycleEventGeneration
  ) {
    return state;
  }
  return { ...state, lifecycleEventGeneration: eventGeneration };
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
    case 'sessionStarted': {
      const observed = observeLifecycleEvent(state, action.eventGeneration);
      if (
        observed.latestRecordingId !== null
        && action.payload.recordingId <= observed.latestRecordingId
      ) {
        return observed;
      }
      return {
        ...clearSession(observed, action.payload.recordingId),
        latestRecordingId: action.payload.recordingId,
      };
    }
    case 'sessionSnapshot': {
      if (action.expectedEventGeneration !== state.lifecycleEventGeneration) {
        return state;
      }
      const { recordingId, status } = action.payload;
      if (state.latestRecordingId !== null && recordingId < state.latestRecordingId) {
        return state;
      }
      if (recordingId === state.latestRecordingId) {
        return state.status === status ? state : { ...state, status };
      }
      return {
        ...clearSession(state, recordingId),
        latestRecordingId: recordingId,
        status,
      };
    }
    case 'statusChanged': {
      const observed = observeLifecycleEvent(state, action.eventGeneration);
      return observed.status === action.status
        ? observed
        : { ...observed, status: action.status };
    }
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
    case 'sessionCleared': {
      const observed = observeLifecycleEvent(state, action.eventGeneration);
      const withTombstone = (
        observed.latestRecordingId === null
        || action.payload.recordingId > observed.latestRecordingId
      )
        ? { ...observed, latestRecordingId: action.payload.recordingId }
        : observed;
      return action.payload.recordingId === withTombstone.activeRecordingId
        ? clearSession(withTombstone)
        : withTombstone;
    }
    case 'settingsChanged': {
      const observed = observeLifecycleEvent(state, action.eventGeneration);
      if (action.model !== observed.model) {
        return {
          ...clearSession(observed),
          enabled: action.enabled,
          model: action.model,
        };
      }
      return {
        ...(action.enabled
          ? observed
          : clearSession(observed, observed.activeRecordingId)),
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
  const lifecycleEventGenerationRef = useRef(0);
  const [state, dispatch] = useReducer(
    partialTranscriptReducer,
    createPartialTranscriptState(enabled, model),
  );

  useEffect(() => {
    const eventGeneration = ++lifecycleEventGenerationRef.current;
    dispatch({ type: 'settingsChanged', enabled, model, eventGeneration });
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
            const eventGeneration = ++lifecycleEventGenerationRef.current;
            flog.info('overlay-preview', 'recording session selected', {
              recordingId: event.payload.recordingId,
              source: 'event',
            });
            dispatch({ type: 'sessionStarted', payload: event.payload, eventGeneration });
          }
        }),
        listen<unknown>('recording-status-changed', (event) => {
          if (isDictationStatus(event.payload)) {
            const eventGeneration = ++lifecycleEventGenerationRef.current;
            dispatch({ type: 'statusChanged', status: event.payload, eventGeneration });
          }
        }),
        listen<unknown>('partial-transcript', (event) => {
          if (isPartialTranscriptPayload(event.payload)) {
            dispatch({ type: 'partialReceived', payload: event.payload });
          }
        }),
        listen<unknown>('partial-transcript-cleared', (event) => {
          if (isPartialTranscriptClearedPayload(event.payload)) {
            const eventGeneration = ++lifecycleEventGenerationRef.current;
            flog.info('overlay-preview', 'partial session clear received', {
              recordingId: event.payload.recordingId,
              reason: event.payload.reason,
            });
            dispatch({ type: 'sessionCleared', payload: event.payload, eventGeneration });
          }
        }),
        listen<unknown>('recording-cancelled', (event) => {
          if (isRecordingIdPayload(event.payload)) {
            const eventGeneration = ++lifecycleEventGenerationRef.current;
            dispatch({
              type: 'sessionCleared',
              eventGeneration,
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
            const eventGeneration = ++lifecycleEventGenerationRef.current;
            dispatch({
              type: 'sessionCleared',
              eventGeneration,
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
        const expectedEventGeneration = lifecycleEventGenerationRef.current;
        const snapshot = await invoke<unknown>('get_active_recording_session');
        const observedEventGeneration = lifecycleEventGenerationRef.current;
        if (
          !disposed
          && isActiveRecordingSessionSnapshot(snapshot)
          && observedEventGeneration === expectedEventGeneration
        ) {
          flog.info('overlay-preview', 'recording session selected', {
            recordingId: snapshot.recordingId,
            status: snapshot.status,
            source: 'readiness_snapshot',
          });
          dispatch({
            type: 'sessionSnapshot',
            payload: snapshot,
            expectedEventGeneration,
          });
        } else if (
          !disposed
          && isActiveRecordingSessionSnapshot(snapshot)
          && observedEventGeneration !== expectedEventGeneration
        ) {
          flog.info('overlay-preview', 'active session snapshot superseded', {
            recordingId: snapshot.recordingId,
            expectedEventGeneration,
            observedEventGeneration,
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
