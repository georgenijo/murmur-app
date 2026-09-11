import { useCallback, useEffect, useRef, useState } from 'react';
import { emitTo, listen } from '@tauri-apps/api/event';
import {
  dismissMeetingSuggestion,
  getMeetingSuggestion,
  isMeetingSuggestionPayload,
  type MeetingSuggestion,
} from '../meetingSuggestions';

export interface MeetingSuggestionController {
  suggestion: MeetingSuggestion | null;
  busy: boolean;
  error: string | null;
  accept: () => Promise<void>;
  dismiss: () => Promise<void>;
  clear: () => void;
}

export function useMeetingSuggestion(): MeetingSuggestionController {
  const [suggestion, setSuggestion] = useState<MeetingSuggestion | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const suggestionRef = useRef<MeetingSuggestion | null>(null);
  const actionTokenRef = useRef<string | null>(null);

  const applySuggestion = useCallback((next: MeetingSuggestion | null) => {
    suggestionRef.current = next;
    setSuggestion(next);
    setError(null);
  }, []);

  useEffect(() => {
    let disposed = false;
    let eventGeneration = 0;
    let unlisten: (() => void) | null = null;

    void listen<unknown>('meeting-suggestion-changed', (event) => {
      if (disposed || !isMeetingSuggestionPayload(event.payload)) return;
      eventGeneration += 1;
      applySuggestion(event.payload);
    }).then(async (stopListening) => {
      if (disposed) {
        stopListening();
        return;
      }
      unlisten = stopListening;
      const snapshotGeneration = eventGeneration;
      try {
        const initial = await getMeetingSuggestion();
        if (
          !disposed
          && eventGeneration === snapshotGeneration
          && isMeetingSuggestionPayload(initial)
        ) {
          applySuggestion(initial);
        }
      } catch {
        if (!disposed && eventGeneration === snapshotGeneration) {
          setError('Meeting suggestions are temporarily unavailable.');
        }
      }
    }).catch(() => {
      if (!disposed) setError('Meeting suggestions are temporarily unavailable.');
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [applySuggestion]);

  const accept = useCallback(async () => {
    const current = suggestionRef.current;
    if (!current || actionTokenRef.current === current.token) return;
    actionTokenRef.current = current.token;
    setBusy(true);
    setError(null);
    try {
      await emitTo('main', 'accept-meeting-suggestion', { token: current.token });
      if (suggestionRef.current?.token === current.token) applySuggestion(null);
    } catch {
      setError('Notetaker could not receive that suggestion. Try again.');
    } finally {
      if (actionTokenRef.current === current.token) {
        actionTokenRef.current = null;
        setBusy(false);
      }
    }
  }, [applySuggestion]);

  const dismiss = useCallback(async () => {
    const current = suggestionRef.current;
    if (!current || actionTokenRef.current === current.token) return;
    actionTokenRef.current = current.token;
    setBusy(true);
    setError(null);
    try {
      await dismissMeetingSuggestion(current.token);
      if (suggestionRef.current?.token === current.token) applySuggestion(null);
    } catch {
      setError('That suggestion could not be dismissed. Try again.');
    } finally {
      if (actionTokenRef.current === current.token) {
        actionTokenRef.current = null;
        setBusy(false);
      }
    }
  }, [applySuggestion]);

  const clear = useCallback(() => applySuggestion(null), [applySuggestion]);

  return {
    suggestion,
    busy,
    error,
    accept,
    dismiss,
    clear,
  };
}
