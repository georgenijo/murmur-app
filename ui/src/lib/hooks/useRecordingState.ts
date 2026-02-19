import { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { startRecording, stopRecording } from '../dictation';
import { isDictationStatus } from '../types';
import type { DictationStatus } from '../types';
import { updateStats } from '../stats';

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
        setStatus(event.payload);
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

  const handleStart = useCallback(async () => {
    if (isStartingRef.current) return;
    isStartingRef.current = true;
    try {
      recordingStartTimeRef.current = Date.now();
      setRecordingStartTime(Date.now());
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
    if (isStoppingRef.current) return;
    isStoppingRef.current = true;
    const duration = recordingStartTimeRef.current
      ? Math.floor((Date.now() - recordingStartTimeRef.current) / 1000)
      : 0;
    try {
      setStatus('processing');
      const res = await stopRecording();
      if (res.text) {
        setTranscription(res.text);
        addEntry(res.text, duration);
        updateStats(res.text, duration);
        setStatsVersion(v => v + 1);
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
