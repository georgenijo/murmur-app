import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { isMeetingSuggestionToken } from '../meetingSuggestions';

type StartSuggestedMeeting = (suggestionToken: string) => Promise<void>;

function suggestionToken(payload: unknown): string | null {
  if (typeof payload !== 'object' || payload === null) return null;
  if (!('token' in payload)) return null;
  return isMeetingSuggestionToken(payload.token) ? payload.token : null;
}

export function useMeetingSuggestionAcceptance(
  startMeeting: StartSuggestedMeeting,
  onError: (message: string) => void,
): void {
  const startRef = useRef(startMeeting);
  const errorRef = useRef(onError);
  const pendingTokensRef = useRef(new Set<string>());
  startRef.current = startMeeting;
  errorRef.current = onError;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<unknown>('accept-meeting-suggestion', (event) => {
      const token = suggestionToken(event.payload);
      if (!token || pendingTokensRef.current.has(token)) return;
      pendingTokensRef.current.add(token);
      void startRef.current(token).catch(() => {
        if (disposed) return;
        errorRef.current('Notetaker could not start from that Calendar suggestion. Start it manually to continue.');
        void invoke('show_main_window').catch(() => {});
      }).finally(() => {
        pendingTokensRef.current.delete(token);
      });
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch(() => {
      if (!disposed) errorRef.current('Meeting suggestion controls are unavailable. Restart Murmur to try again.');
    });
    return () => {
      disposed = true;
      unlisten?.();
      pendingTokensRef.current.clear();
    };
  }, []);
}
