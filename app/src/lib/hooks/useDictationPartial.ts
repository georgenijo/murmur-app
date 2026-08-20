import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';

interface DictationPartialPayload {
  recordingId: number;
  text: string;
}

/** Mirrors the Rust-side cap; a longer payload means something other than the
 *  partial ticker produced it, so it is dropped rather than rendered. */
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
 * window is shown and hidden by Rust, so this hook never decides visibility —
 * it only makes sure a card that is on screen is never showing another
 * recording's words.
 */
export function useDictationPartial(): string {
  const [partial, setPartial] = useState('');
  const recordingIdRef = useRef(0);

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
      setPartial('');
    }));

    track(listen<unknown>('dictation-partial', ({ payload }) => {
      if (!validPayload(payload) || payload.recordingId < recordingIdRef.current) return;
      // A partial can land before this window observes the generation event
      // (the card is only shown once words exist). Adopt the newer id rather
      // than dropping the very first line of the transcript.
      recordingIdRef.current = payload.recordingId;
      setPartial(payload.text.trim());
    }));

    return () => {
      cancelled = true;
      unlistens.forEach((unlisten) => unlisten());
    };
  }, []);

  return partial;
}
