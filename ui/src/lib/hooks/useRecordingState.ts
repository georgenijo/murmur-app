import { useState, useEffect, useRef, useCallback } from 'react';
import { startRecording, stopRecording } from '../dictation';
import type { DictationStatus } from '../types';

interface UseRecordingStateProps {
  addEntry: (text: string, duration: number) => void;
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
  useEffect(() => { recordingStartTimeRef.current = recordingStartTime; }, [recordingStartTime]);

  // Recording duration timer
  useEffect(() => {
    let interval: ReturnType<typeof setInterval>;
    if (status === 'recording' && recordingStartTime) {
      interval = setInterval(() => {
        setRecordingDuration(Math.floor((Date.now() - recordingStartTime) / 1000));
      }, 100);
    } else {
      setRecordingDuration(0);
    }
    return () => clearInterval(interval);
  }, [status, recordingStartTime]);

  const handleStart = useCallback(async () => {
    try {
      setRecordingStartTime(Date.now());
      setError('');
      const res = await startRecording();
      if (res.state) setStatus(res.state as DictationStatus);
      if (res.type === 'error') {
        setError(res.error || 'Unknown error');
        setRecordingStartTime(null);
      }
    } catch (err) {
      setError(String(err));
      setRecordingStartTime(null);
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
      setStatus((res.state as DictationStatus) || 'idle');
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
