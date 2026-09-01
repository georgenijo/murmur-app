import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface DictationPartialPayload {
  recordingId: number;
  text: string;
}

/** Defensive bound. The 20s decode window keeps real partials far below this;
 *  a longer payload means something other than the partial ticker produced it,
 *  so it is dropped rather than rendered. */
const MAX_PARTIAL_CHARS = 4096;

function validPayload(value: unknown): value is DictationPartialPayload {
  const payload = value as Partial<DictationPartialPayload> | null;
  return typeof payload?.recordingId === 'number'
    && Number.isSafeInteger(payload.recordingId)
    && payload.recordingId > 0
    && typeof payload.text === 'string'
    && payload.text.length <= MAX_PARTIAL_CHARS;
}

function validRecordingId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

/**
 * Live partial text for the dictation preview window.
 *
 * Generation-gated the same way the Rust ticker is: a partial is only rendered
 * while it belongs to the newest recording seen on this window. The preview
 * This hook renders the current recording's words before it asks Rust to show
 * the native window. Rust validates ownership, performs the native show, and
 * hides the window when recording ends.
 *
 * It also drops the text the moment capture leaves `recording`. Rust hides the
 * window when the partial ticker exits, but that can trail the actual stop by
 * up to one tick (or a whole in-flight decode). Clearing here empties the card
 * immediately, so a stopped recording never leaves provisional words on screen
 * while the native hide catches up.
 */
export function useDictationPartial(): string {
  const [partial, setPartial] = useState<DictationPartialPayload | null>(null);
  const recordingIdRef = useRef(0);
  const shownRecordingIdRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    const track = (pending: Promise<() => void>) => {
      void pending
        .then((unlisten) => { cancelled ? unlisten() : unlistens.push(unlisten); })
        .catch(() => {});
    };

    track(listen<unknown>('dictation-generation-started', ({ payload }) => {
      const recordingId = (payload as { recordingId?: unknown } | null)?.recordingId;
      if (!validRecordingId(recordingId)) return;
      recordingIdRef.current = recordingId;
      shownRecordingIdRef.current = 0;
      setPartial(null);
    }));

    track(listen<unknown>('recording-status-changed', ({ payload }) => {
      if (payload !== 'recording') {
        shownRecordingIdRef.current = 0;
        setPartial(null);
      }
    }));

    track(listen<unknown>('dictation-partial', ({ payload }) => {
      if (!validPayload(payload) || payload.recordingId < recordingIdRef.current) return;
      // A partial can land before this window observes the generation event
      // (the card is only shown once words exist). Adopt the newer id rather
      // than dropping the very first line of the transcript.
      recordingIdRef.current = payload.recordingId;
      setPartial({ recordingId: payload.recordingId, text: payload.text.trim() });
    }));

    return () => {
      cancelled = true;
      unlistens.forEach((unlisten) => unlisten());
    };
  }, []);

  // The hidden webview must paint the card before AppKit shows its transparent
  // NSWindow. Showing first can commit an empty transparent frame that macOS
  // removes from the on-screen window list before the text event renders.
  useEffect(() => {
    if (!partial?.text || shownRecordingIdRef.current === partial.recordingId) return;
    shownRecordingIdRef.current = partial.recordingId;
    void invoke('show_dictation_preview', { recordingId: partial.recordingId }).catch(() => {
      if (shownRecordingIdRef.current === partial.recordingId) {
        shownRecordingIdRef.current = 0;
      }
    });
  }, [partial]);

  return partial?.text ?? '';
}
