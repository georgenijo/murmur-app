import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelRecording, startRecording, stopRecording } from '../dictation';
import type { SmartAutoMicrophoneRequest } from '../settings';
import { isDictationStatus } from '../types';
import type { DictationStatus } from '../types';
import { updateStats } from '../stats';
import { flog } from '../log';
import type { TeachingContext } from '../correctAndTeach';
import type { HistoryInterruption, HistoryRecordingContext } from '../history';
import {
  cleanupStalledPresentationFromPayload,
  initializationPresentationFromPayload,
  interruptedPresentationFromPayload,
} from '../dictationPresentation';

interface UseRecordingStateProps {
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string, teachingContext?: TeachingContext, interruption?: HistoryInterruption, details?: { rawText: string; recording: HistoryRecordingContext }) => void;
  microphone: string;
  smartAuto?: SmartAutoMicrophoneRequest | null;
}

type RecordingErrorKind = 'cleanup' | 'other';

interface RecordingErrorPresentation {
  id: number;
  message: string;
  kind: RecordingErrorKind;
  recordingId?: number;
}

interface PresentErrorOptions {
  kind?: RecordingErrorKind;
  recordingId?: number;
  producerEpoch?: number;
}

export function useRecordingState({ addEntry, microphone, smartAuto = null }: UseRecordingStateProps) {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [transcription, setTranscription] = useState('');
  const [errorPresentation, setErrorPresentation] = useState<RecordingErrorPresentation | null>(null);
  const [recordingStartTime, setRecordingStartTime] = useState<number | null>(null);
  const [recordingDuration, setRecordingDuration] = useState(0);
  const [audioLevel, setAudioLevel] = useState(0);
  const [lockedMode, setLockedMode] = useState(false);
  const [statsVersion, setStatsVersion] = useState(0);

  // Refs for stable callbacks (hotkey toggle reads current state)
  const statusRef = useRef(status);
  const microphoneRef = useRef(microphone);
  const smartAutoRef = useRef(smartAuto);
  const recordingStartTimeRef = useRef(recordingStartTime);
  const latestRecordingGenerationRef = useRef(0);
  const currentErrorRef = useRef<RecordingErrorPresentation | null>(null);
  const nextErrorIdRef = useRef(0);
  const errorProducerEpochRef = useRef(0);
  const dismissedErrorRef = useRef<{ message: string; recordingId?: number } | null>(null);
  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { microphoneRef.current = microphone; }, [microphone]);
  useEffect(() => { smartAutoRef.current = smartAuto; }, [smartAuto]);
  const isStartingRef = useRef(false);
  const startOperationRef = useRef<Promise<void> | null>(null);
  const isStoppingRef = useRef(false);

  const presentError = useCallback((message: string, options: PresentErrorOptions = {}) => {
    if (!message) return null;
    if (
      options.producerEpoch !== undefined
      && options.producerEpoch !== errorProducerEpochRef.current
    ) {
      return null;
    }
    if (
      options.recordingId !== undefined
      && options.recordingId < latestRecordingGenerationRef.current
    ) {
      return null;
    }
    const dismissed = dismissedErrorRef.current;
    if (
      dismissed?.message === message
      && dismissed.recordingId === options.recordingId
    ) {
      return null;
    }

    errorProducerEpochRef.current += 1;
    const presentation: RecordingErrorPresentation = {
      id: ++nextErrorIdRef.current,
      message,
      kind: options.kind ?? 'other',
      recordingId: options.recordingId,
    };
    currentErrorRef.current = presentation;
    setErrorPresentation(presentation);
    return presentation.id;
  }, []);

  const clearError = useCallback((expectedId?: number) => {
    if (expectedId !== undefined && currentErrorRef.current?.id !== expectedId) return;
    errorProducerEpochRef.current += 1;
    currentErrorRef.current = null;
    setErrorPresentation(null);
  }, []);

  const dismissError = useCallback(() => {
    const current = currentErrorRef.current;
    if (current) {
      dismissedErrorRef.current = {
        message: current.message,
        recordingId: current.recordingId,
      };
    }
    clearError();
  }, [clearError]);

  const beginErrorProducer = useCallback(() => {
    dismissedErrorRef.current = null;
    clearError();
    return errorProducerEpochRef.current;
  }, [clearError]);

  // Recording duration timer
  useEffect(() => {
    let interval: ReturnType<typeof setInterval>;
    if (status === 'recording' && recordingStartTime) {
      interval = setInterval(() => {
        setRecordingDuration(Math.floor((Date.now() - recordingStartTime) / 1000));
      }, 1000);
    } else {
      setRecordingDuration(0);
    }
    return () => clearInterval(interval);
  }, [status, recordingStartTime]);

  // Sync status from Rust events — keeps main window in sync when overlay controls recording
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('recording-status-changed', (event) => {
      if (isDictationStatus(event.payload)) {
        const previousStatus = statusRef.current;
        flog.info('recording', 'status event: ' + event.payload, {
          prevStatus: previousStatus,
          recordingStartTime: recordingStartTimeRef.current,
          isStopping: isStoppingRef.current,
        });
        // Update the ref synchronously. Hotkey events can arrive before React
        // commits the state update, and transition decisions read this ref.
        statusRef.current = event.payload;
        setStatus(event.payload);
        // When recording starts from the overlay, handleStart doesn't run in this window.
        // Seed recordingStartTime so the duration timer ticks.
        if (event.payload === 'recording' && !recordingStartTimeRef.current) {
          const now = Date.now();
          recordingStartTimeRef.current = now;
          setRecordingStartTime(now);
        }
        // When recording stops, clear recordingStartTime.
        if (
          event.payload === 'idle'
          || event.payload === 'starting'
          || event.payload === 'recovering'
          || event.payload === 'processing'
        ) {
          recordingStartTimeRef.current = null;
          setRecordingStartTime(null);
        }
        // If idle arrived externally (e.g. Escape cancel), unblock handleStop
        // so the next recording cycle can stop normally.
        if (event.payload === 'idle') {
          isStoppingRef.current = false;
          if (previousStatus === 'recovering') {
            // Recovery reaching Idle is authoritative. Invalidate any invoke
            // response that was produced before this transition and clear only
            // the cleanup notice associated with that lifecycle. A distinct,
            // newer error remains visible.
            errorProducerEpochRef.current += 1;
            if (currentErrorRef.current?.kind === 'cleanup') {
              currentErrorRef.current = null;
              setErrorPresentation(null);
            }
          }
        }
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Audio initialization failures are terminal for the attempt. Cancellation
  // briefly reports Recovering before Idle while detached Core Audio cleanup
  // continues asynchronously, so keep the error visible across that transition.
  useEffect(() => {
    let cancelled = false;
    const unlistens: (() => void)[] = [];
    const acceptPresentationGeneration = (recordingId: number) => {
      if (recordingId < latestRecordingGenerationRef.current) return false;
      if (recordingId > latestRecordingGenerationRef.current) {
        latestRecordingGenerationRef.current = recordingId;
        dismissedErrorRef.current = null;
        clearError();
      }
      return true;
    };
    listen<unknown>('dictation-generation-started', (event) => {
      const payload = event.payload as Record<string, unknown> | null;
      const recordingId = payload?.recordingId;
      if (
        typeof recordingId === 'number'
        && Number.isSafeInteger(recordingId)
        && recordingId > latestRecordingGenerationRef.current
      ) {
        latestRecordingGenerationRef.current = recordingId;
        dismissedErrorRef.current = null;
        clearError();
      }
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    listen<unknown>('recording-initialization-failed', (event) => {
      const presentation = initializationPresentationFromPayload(event.payload);
      if (!presentation || !acceptPresentationGeneration(presentation.recordingId)) return;
      const payload = event.payload as Record<string, unknown>;
      const message = typeof payload.error === 'string'
        ? payload.error
        : 'Microphone initialization failed.';
      presentError(message, { recordingId: presentation.recordingId });
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    listen<unknown>('recording-recovery-stalled', (event) => {
      const presentation = cleanupStalledPresentationFromPayload(event.payload);
      if (!presentation || !acceptPresentationGeneration(presentation.recordingId)) return;
      presentError(
        'Murmur is still waiting for macOS audio to finish stopping the microphone. '
        + 'Restarting Murmur clears this stop, but macOS audio may still need time to recover.',
        { kind: 'cleanup', recordingId: presentation.recordingId },
      );
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    listen<unknown>('recording-interrupted', (event) => {
      const presentation = interruptedPresentationFromPayload(event.payload);
      if (!presentation || !acceptPresentationGeneration(presentation.recordingId)) return;
      const payload = event.payload as Record<string, unknown>;
      presentError(payload.autoTranscribe === true
        ? 'Microphone capture was interrupted. Murmur is transcribing the audio received so far.'
        : 'Microphone capture was interrupted before enough audio was received.', {
        recordingId: presentation.recordingId,
      });
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
    };
  }, [clearError, presentError]);

  // Subscribe to live audio level for waveform visualisation
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<number>('audio-level', (event) => {
      setAudioLevel(event.payload);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Listen for auto-paste failures and surface a hint to the user
  const pasteErrorTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('auto-paste-failed', (event) => {
      const errorId = presentError(event.payload);
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
      if (errorId !== null) {
        pasteErrorTimerRef.current = setTimeout(() => clearError(errorId), 5000);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      unlisten?.();
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
    };
  }, [clearError, presentError]);

  // Listen for file-output (save transcript/audio) failures and surface a hint.
  // Reuses the same auto-clearing error banner as auto-paste failures.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('file-output-failed', (event) => {
      const errorId = presentError(event.payload);
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
      if (errorId !== null) {
        pasteErrorTimerRef.current = setTimeout(() => clearError(errorId), 5000);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [clearError, presentError]);

  // Sync transcription results from Rust — picks up text when recording was
  // initiated from the overlay (where handleStop doesn't run in this window).
  // Skip if isStoppingRef is true — handleStop is active and will handle it.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ text: string; rawText?: string; duration: number; teachingContext?: TeachingContext; interrupted?: HistoryInterruption; recording?: HistoryRecordingContext }>('transcription-complete', (event) => {
      flog.info('recording', 'transcription-complete event', {
        textLen: event.payload.text?.length, duration: event.payload.duration,
        isStopping: isStoppingRef.current,
      });
      // Single source of truth for history entries — always handle here,
      // never in handleStop, to avoid race-condition duplicates.
      const { text, rawText, duration, teachingContext, interrupted, recording } = event.payload;
      if (text) {
        setTranscription(text);
        const details = typeof rawText === 'string' && recording
          ? { rawText, recording }
          : undefined;
        if (details) {
          addEntry(text, duration, 'recording', undefined, teachingContext, interrupted, details);
        } else {
          addEntry(text, duration, 'recording', undefined, teachingContext, interrupted);
        }
        updateStats(text, duration);
        setStatsVersion(v => v + 1);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [addEntry]);

  const beginRecording = useCallback(async (origin: 'toggle' | 'hold') => {
    flog.info('recording', 'handleStart called', {
      isStarting: isStartingRef.current, status: statusRef.current, origin,
    });
    if (startOperationRef.current) {
      await startOperationRef.current;
      return;
    }
    isStartingRef.current = true;
    const operation = (async () => {
      const producerEpoch = beginErrorProducer();
      try {
        const res = await startRecording(microphoneRef.current, origin, smartAutoRef.current);
        // These two responses may describe a transform-owned supervisor
        // attempt while dictation itself is Idle. Lifecycle events remain the
        // authority; never let a later invoke response overwrite them.
        const responseOwnsDictationState = res.type !== 'audio_recovering'
          && res.type !== 'already_starting';
        if (responseOwnsDictationState && isDictationStatus(res.state)) {
          // A fast device can emit Recording before the invoke promise resolves.
          // Never let the older `recording_starting` response move that newer
          // lifecycle event backwards.
          const staleStartingResponse = res.state === 'starting'
            && statusRef.current !== 'idle'
            && statusRef.current !== 'starting';
          if (!staleStartingResponse) {
            statusRef.current = res.state;
            setStatus(res.state);
          }
        }
        if (res.type !== 'recording_starting') {
          if (res.type === 'error') presentError(res.error || 'Unknown error', { producerEpoch });
          else if (res.type === 'busy_benchmarking') presentError('Wait for the benchmark to finish.', { producerEpoch });
          else if (res.type === 'busy_transcribing_file') presentError('Wait for the file transcription to finish.', { producerEpoch });
          else if (res.type === 'busy_meeting') presentError('Stop the meeting capture before starting dictation.', { producerEpoch });
          // Without these three the press is a silent no-op: Rust refuses the
          // start but nothing tells the user why.
          else if (res.type === 'busy_querying') presentError('Finish speaking your voice query before starting dictation.', { producerEpoch });
          else if (res.type === 'busy_transforming') presentError('Wait for the transform to finish.', { producerEpoch });
          else if (res.type === 'busy_recording_corpus') presentError('Finish the corpus recording before starting dictation.', { producerEpoch });
          else if (res.type === 'audio_recovering') {
            presentError('Microphone cleanup is still in progress. Try again when Murmur is ready.', {
              kind: 'cleanup',
              producerEpoch,
            });
          }
        }
      } catch (err) {
        statusRef.current = 'idle';
        setStatus('idle');
        presentError(String(err), { producerEpoch });
        setRecordingStartTime(null);
        recordingStartTimeRef.current = null;
      } finally {
        isStartingRef.current = false;
      }
    })();
    startOperationRef.current = operation;
    try {
      await operation;
    } finally {
      if (startOperationRef.current === operation) {
        startOperationRef.current = null;
      }
    }
  }, [beginErrorProducer, presentError]);

  const handleStart = useCallback(() => beginRecording('toggle'), [beginRecording]);
  const handleHoldStart = useCallback(() => beginRecording('hold'), [beginRecording]);

  const handleStop = useCallback(async () => {
    flog.info('recording', 'handleStop called', {
      isStopping: isStoppingRef.current, status: statusRef.current,
      recordingStartTime: recordingStartTimeRef.current,
    });
    if (isStoppingRef.current) return;
    isStoppingRef.current = true;
    const producerEpoch = errorProducerEpochRef.current;
    try {
      if (statusRef.current === 'starting') {
        flog.info('recording', 'handleStop cancelling audio initialization');
        await cancelRecording();
        return;
      }
      if (statusRef.current === 'recovering') {
        presentError('Microphone cleanup is still in progress. Try again when Murmur is ready.', {
          kind: 'cleanup',
        });
        return;
      }
      if (statusRef.current !== 'recording') {
        flog.info('recording', 'handleStop ignored', {
          status: statusRef.current,
        });
        return;
      }
      const duration = recordingStartTimeRef.current
        ? Math.floor((Date.now() - recordingStartTimeRef.current) / 1000)
        : 0;
      flog.info('recording', 'computed duration', { duration });
      statusRef.current = 'processing';
      setStatus('processing');
      const res = await stopRecording();
      if (res.text) {
        setTranscription(res.text);
        // addEntry/updateStats handled by transcription-complete event listener
        // to avoid race-condition duplicates.
      }
      if (res.type === 'error') presentError(res.error || 'Unknown error', { producerEpoch });
      // Only update status from the return value if we're still in
      // processing. If cancel already set us to idle (or a new recording
      // started), don't clobber the current state with a stale result.
      // Event handlers update statusRef synchronously, so this check cannot
      // lag behind React rendering.
      const newStatus = isDictationStatus(res.state) ? res.state : 'idle';
      if (statusRef.current === 'processing') {
        statusRef.current = newStatus;
        setStatus(newStatus);
      }
    } catch (err) {
      presentError(String(err), { producerEpoch });
      statusRef.current = 'idle';
      setStatus('idle');
    } finally {
      isStoppingRef.current = false;
    }
  }, [presentError]);

  // Stable toggle for hotkey use — reads status from ref
  const toggleRecording = useCallback(async () => {
    flog.info('recording', 'toggleRecording', { status: statusRef.current });
    if (statusRef.current === 'processing' || statusRef.current === 'recovering') return;
    if (statusRef.current === 'recording' || statusRef.current === 'starting') {
      await handleStop();
    } else {
      await handleStart();
    }
  }, [handleStart, handleStop]);

  // Side effects must live outside the setLockedMode updater to avoid double-firing in StrictMode
  const toggleLockedMode = useCallback(async () => {
    const next = !lockedMode;
    setLockedMode(next);
    if (next && statusRef.current !== 'recording') {
      await handleStart();
    } else if (!next && statusRef.current === 'recording') {
      await handleStop();
    }
  }, [lockedMode, handleStart, handleStop]);

  return {
    status,
    transcription,
    recordingDuration,
    error: errorPresentation?.message ?? '',
    dismissError,
    handleStart,
    handleHoldStart,
    handleStop,
    toggleRecording,
    audioLevel,
    lockedMode,
    toggleLockedMode,
    statsVersion,
  };
}
