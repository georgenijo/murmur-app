import { useState, useEffect, useRef, useCallback } from 'react';
import { startRecording, stopRecording } from '../dictation';
import type { DictationStatus } from '../types';

interface UseRecordingStateProps {
  addEntry: (text: string, duration: number) => void;
}

const VALID_STATUSES = ['idle', 'recording', 'processing'] as const;
function isDictationStatus(v: unknown): v is DictationStatus {
  return typeof v === 'string' && (VALID_STATUSES as readonly string[]).includes(v);
}

export function useRecordingState({ addEntry }: UseRecordingStateProps) {
  const [status, setStatus] = useState<DictationStatus>('idle');
  const [transcription, setTranscription] = useState('');
  const [error, setError] = useState('');
  const [recordingStartTime, setRecordingStartTime] = useState<number | null>(null);
  const [recordingDuration, setRecordingDuration] = useState(0);

  // Refs for stable callbacks (hotkey toggle reads current state)
  const statusRef = useRef(status);
  const recordingStartTimeRef = useRef(recordingStartTime);
  useEffect(() => { statusRef.current = status; }, [status]);
  const isStartingRef = useRef(false);

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
    const duration = recordingStartTimeRef.current
      ? Math.floor((Date.now() - recordingStartTimeRef.current) / 1000)
      : 0;
    try {
      setStatus('processing');
      const res = await stopRecording();
      if (res.text) {
        setTranscription(res.text);
        addEntry(res.text, duration);
      }
      if (res.type === 'error') setError(res.error || 'Unknown error');
      setStatus(isDictationStatus(res.state) ? res.state : 'idle');
    } catch (err) {
      setError(String(err));
      setStatus('idle');
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

  return {
    status,
    transcription,
    recordingDuration,
    error,
    setError,
    handleStart,
    handleStop,
    toggleRecording,
  };
}
