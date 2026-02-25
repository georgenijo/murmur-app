import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { startRecording, stopRecording } from '../dictation';
import { isDictationStatus } from '../types';
import type { DictationStatus } from '../types';
import { updateStats } from '../stats';
import { flog } from '../log';

interface UseRecordingStateProps {
  addEntry: (text: string, duration: number) => void;
}

export function useRecordingState({ addEntry }: UseRecordingStateProps) {
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
  const recordingStartTimeRef = useRef(recordingStartTime);
  useEffect(() => { statusRef.current = status; }, [status]);
  const isStartingRef = useRef(false);
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
        setStatus(event.payload);
        // When recording starts from the overlay, handleStart doesn't run in this window.
        // Seed recordingStartTime so the duration timer ticks.
        if (event.payload === 'recording' && !recordingStartTimeRef.current) {
          const now = Date.now();
          recordingStartTimeRef.current = now;
          setRecordingStartTime(now);
        }
        // When recording stops, clear recordingStartTime.
        if (event.payload === 'idle' || event.payload === 'processing') {
          recordingStartTimeRef.current = null;
          setRecordingStartTime(null);
        }
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
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

  // Sync transcription results from Rust — picks up text when recording was
  // initiated from the overlay (where handleStop doesn't run in this window).
  // Skip if isStoppingRef is true — handleStop is active and will handle it.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<{ text: string; duration: number }>('transcription-complete', (event) => {
      flog.info('recording', 'transcription-complete event', {
        textLen: event.payload.text?.length, duration: event.payload.duration,
        isStopping: isStoppingRef.current,
      });
      // Single source of truth for history entries — always handle here,
      // never in handleStop, to avoid race-condition duplicates.
      const { text, duration } = event.payload;
      if (text) {
        setTranscription(text);
        addEntry(text, duration);
        updateStats(text, duration);
        setStatsVersion(v => v + 1);
      }
    }).then((fn) => {
      if (cancelled) { fn(); } else { unlisten = fn; }
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [addEntry]);

  const handleStart = useCallback(async () => {
    flog.info('recording', 'handleStart called', {
      isStarting: isStartingRef.current, status: statusRef.current,
    });
    if (isStartingRef.current) return;
    isStartingRef.current = true;
    try {
      const now = Date.now();
      flog.info('recording', 'setting recordingStartTime', { value: now });
      recordingStartTimeRef.current = now;
      setRecordingStartTime(now);
      setError('');
      const res = await startRecording();
      if (isDictationStatus(res.state)) setStatus(res.state);
      if (res.type === 'error') {
        setError(res.error || 'Unknown error');
        setRecordingStartTime(null);
        recordingStartTimeRef.current = null;
      }
    } catch (err) {
      setError(String(err));
      setRecordingStartTime(null);
      recordingStartTimeRef.current = null;
    } finally {
      isStartingRef.current = false;
    }
  }, []);

  const handleStop = useCallback(async () => {
    flog.info('recording', 'handleStop called', {
      isStopping: isStoppingRef.current, status: statusRef.current,
      recordingStartTime: recordingStartTimeRef.current,
    });
    if (isStoppingRef.current) return;
    isStoppingRef.current = true;
    const duration = recordingStartTimeRef.current
      ? Math.floor((Date.now() - recordingStartTimeRef.current) / 1000)
      : 0;
    flog.info('recording', 'computed duration', { duration });
    try {
      setStatus('processing');
      const res = await stopRecording();
      if (res.text) {
        setTranscription(res.text);
        // addEntry/updateStats handled by transcription-complete event listener
        // to avoid race-condition duplicates.
      }
      if (res.type === 'error') setError(res.error || 'Unknown error');
      setStatus(isDictationStatus(res.state) ? res.state : 'idle');
    } catch (err) {
      setError(String(err));
      setStatus('idle');
    } finally {
      isStoppingRef.current = false;
    }
  }, [addEntry]);

  // Stable toggle for hotkey use — reads status from ref
  const toggleRecording = useCallback(async () => {
    flog.info('recording', 'toggleRecording', { status: statusRef.current });
    if (statusRef.current === 'processing') return;
    if (statusRef.current === 'recording') {
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
    handleStop,
    toggleRecording,
    audioLevel,
    lockedMode,
    toggleLockedMode,
    statsVersion,
  };
}
