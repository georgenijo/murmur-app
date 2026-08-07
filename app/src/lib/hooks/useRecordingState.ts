import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cancelRecording, startRecording, stopRecording } from '../dictation';
import { isDictationStatus } from '../types';
import type { DictationStatus } from '../types';
import { updateStats } from '../stats';
import { flog } from '../log';
import type { TeachingContext } from '../correctAndTeach';
import type { HistoryInterruption } from '../history';

interface UseRecordingStateProps {
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string, teachingContext?: TeachingContext, interruption?: HistoryInterruption) => void;
  microphone: string;
  microphoneFallbackToDefault?: boolean;
}

export function useRecordingState({
  addEntry,
  microphone,
  microphoneFallbackToDefault = false,
}: UseRecordingStateProps) {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [transcription, setTranscription] = useState('');
  const [error, setError] = useState('');
  const [recordingStartTime, setRecordingStartTime] = useState<number | null>(null);
  const [recordingDuration, setRecordingDuration] = useState(0);
  const [audioLevel, setAudioLevel] = useState(0);
  const [lockedMode, setLockedMode] = useState(false);
  const [statsVersion, setStatsVersion] = useState(0);

  // Refs for stable callbacks (hotkey toggle reads current state)
  const statusRef = useRef(status);
  const microphoneRef = useRef(microphone);
  const microphoneFallbackRef = useRef(microphoneFallbackToDefault);
  const recordingStartTimeRef = useRef(recordingStartTime);
  useEffect(() => { statusRef.current = status; }, [status]);
  useEffect(() => { microphoneRef.current = microphone; }, [microphone]);
  useEffect(() => {
    microphoneFallbackRef.current = microphoneFallbackToDefault;
  }, [microphoneFallbackToDefault]);
  const isStartingRef = useRef(false);
  const startOperationRef = useRef<Promise<void> | null>(null);
  const isStoppingRef = useRef(false);

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
        flog.info('recording', 'status event: ' + event.payload, {
          prevStatus: statusRef.current,
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
    listen<{ error?: unknown }>('recording-initialization-failed', (event) => {
      const message = typeof event.payload?.error === 'string'
        ? event.payload.error
        : 'Microphone initialization failed.';
      setError(message);
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    listen('recording-recovery-stalled', () => {
      setError(
        'Murmur is still waiting for macOS audio to finish stopping the microphone. '
        + 'Restarting Murmur clears this stop, but macOS audio may still need time to recover.',
      );
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    listen<{ autoTranscribe?: boolean }>('recording-interrupted', (event) => {
      setError(event.payload?.autoTranscribe
        ? 'Microphone capture was interrupted. Murmur is transcribing the audio received so far.'
        : 'Microphone capture was interrupted before enough audio was received.');
    }).then((fn) => {
      if (cancelled) fn(); else unlistens.push(fn);
    });
    return () => {
      cancelled = true;
      unlistens.forEach((fn) => fn());
    };
  }, []);

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
      setError(event.payload);
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
      pasteErrorTimerRef.current = setTimeout(() => setError(''), 5000);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => {
      cancelled = true;
      unlisten?.();
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
    };
  }, []);

  // Listen for file-output (save transcript/audio) failures and surface a hint.
  // Reuses the same auto-clearing error banner as auto-paste failures.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<string>('file-output-failed', (event) => {
      setError(event.payload);
      if (pasteErrorTimerRef.current) clearTimeout(pasteErrorTimerRef.current);
      pasteErrorTimerRef.current = setTimeout(() => setError(''), 5000);
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  // Sync transcription results from Rust — picks up text when recording was
  // initiated from the overlay (where handleStop doesn't run in this window).
  // Skip if isStoppingRef is true — handleStop is active and will handle it.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ text: string; duration: number; teachingContext?: TeachingContext; interrupted?: HistoryInterruption }>('transcription-complete', (event) => {
      flog.info('recording', 'transcription-complete event', {
        textLen: event.payload.text?.length, duration: event.payload.duration,
        isStopping: isStoppingRef.current,
      });
      // Single source of truth for history entries — always handle here,
      // never in handleStop, to avoid race-condition duplicates.
      const { text, duration, teachingContext, interrupted } = event.payload;
      if (text) {
        setTranscription(text);
        addEntry(text, duration, 'recording', undefined, teachingContext, interrupted);
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
      try {
        setError('');
        const res = await startRecording(
          microphoneRef.current,
          origin,
          microphoneFallbackRef.current,
        );
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
          if (res.type === 'error') setError(res.error || 'Unknown error');
          else if (res.type === 'busy_benchmarking') setError('Wait for the benchmark to finish.');
          else if (res.type === 'busy_transcribing_file') setError('Wait for the file transcription to finish.');
          else if (res.type === 'audio_recovering') {
            setError('Microphone cleanup is still in progress. Try again when Murmur is ready.');
          }
        }
      } catch (err) {
        statusRef.current = 'idle';
        setStatus('idle');
        setError(String(err));
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
  }, []);

  const handleStart = useCallback(() => beginRecording('toggle'), [beginRecording]);
  const handleHoldStart = useCallback(() => beginRecording('hold'), [beginRecording]);

  const handleStop = useCallback(async () => {
    flog.info('recording', 'handleStop called', {
      isStopping: isStoppingRef.current, status: statusRef.current,
      recordingStartTime: recordingStartTimeRef.current,
    });
    if (isStoppingRef.current) return;
    isStoppingRef.current = true;
    try {
      if (statusRef.current === 'starting') {
        flog.info('recording', 'handleStop cancelling audio initialization');
        await cancelRecording();
        return;
      }
      if (statusRef.current === 'recovering') {
        setError('Microphone cleanup is still in progress. Try again when Murmur is ready.');
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
      if (res.type === 'error') setError(res.error || 'Unknown error');
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
      setError(String(err));
      statusRef.current = 'idle';
      setStatus('idle');
    } finally {
      isStoppingRef.current = false;
    }
  }, []);

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
    error,
    setError,
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
