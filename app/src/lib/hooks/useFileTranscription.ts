import { useState, useEffect, useCallback, useRef } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { transcribeFile } from '../dictation';
import { flog } from '../log';
import {
  QueueItem,
  UNSUPPORTED_MESSAGE,
  buildQueueItems,
  updateItem,
  nextQueued,
  summarize,
  hasAnyAudio,
} from '../fileQueue';

export type { QueueItem, QueueItemStatus } from '../fileQueue';

interface UseFileTranscriptionProps {
  /** Persist completed transcriptions to shared history (no WPM stats). */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

/**
 * Manages the multi-file transcription flow: maintains a queue of audio files,
 * invokes the Rust `transcribe_file` command for each one **sequentially** (one
 * at a time keeps memory/CPU sane — the backend is shared with live dictation),
 * tracks per-file status, and appends each completed result to shared history.
 *
 * A single file's failure is recorded on its own item and does not abort the
 * remaining queue. Drag-and-drop via the Tauri webview yields real filesystem
 * paths (the plain HTML5 drop event does not, for security reasons); the
 * multi-select file picker in the panel supplies paths the same way.
 */
export function useFileTranscription({ addEntry }: UseFileTranscriptionProps) {
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [error, setError] = useState('');
  const [isDragging, setIsDragging] = useState(false);
  const [isRunning, setIsRunning] = useState(false);

  // Refs mirror state so the once-registered drag-drop listener and the async
  // drain loop always see current values without re-subscribing.
  const runningRef = useRef(false);
  runningRef.current = isRunning;
  const queueRef = useRef(queue);
  queueRef.current = queue;
  const addEntryRef = useRef(addEntry);
  addEntryRef.current = addEntry;

  // Sequentially process every still-`queued` item. Re-entrancy is guarded by
  // `runningRef`; the loop re-reads `queueRef` each pass so items enqueued mid-run
  // are picked up. Per-file errors are captured on the item, never thrown.
  const drain = useCallback(async () => {
    if (runningRef.current) return;
    runningRef.current = true;
    setIsRunning(true);

    try {
      // eslint-disable-next-line no-constant-condition
      while (true) {
        const item = nextQueued(queueRef.current);
        if (!item) break;

        setQueue((q) => updateItem(q, item.id, { status: 'transcribing' }));
        flog.info('file-transcribe', 'start', { name: item.name });

        const res = await transcribeFile(item.path);

        if (res.type === 'error') {
          const message = res.error || 'Transcription failed';
          setQueue((q) => updateItem(q, item.id, { status: 'error', error: message }));
          flog.warn('file-transcribe', 'error', { error: message });
          continue;
        }

        const text = res.text || '';
        setQueue((q) => updateItem(q, item.id, { status: 'done', text }));
        if (text.trim()) {
          addEntryRef.current(text, res.duration ?? 0, 'file', item.name);
        }
        flog.info('file-transcribe', 'complete', { textLen: text.length });
      }
    } finally {
      runningRef.current = false;
      setIsRunning(false);
    }
  }, []);

  // Add audio paths to the queue (deduped) and kick off the drain loop. Reports
  // an unsupported-type error only when *none* of the dropped/picked files are
  // audio, so a mixed selection still queues the valid files.
  const enqueue = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    if (!hasAnyAudio(paths)) {
      setError(UNSUPPORTED_MESSAGE);
      return;
    }
    setError('');
    setQueue((prev) => {
      const added = buildQueueItems(paths, prev);
      const next = added.length > 0 ? [...prev, ...added] : prev;
      queueRef.current = next;
      return next;
    });
    void drain();
  }, [drain]);

  /** Backwards-compatible single-path entry point (wraps `enqueue`). */
  const transcribe = useCallback((path: string) => {
    enqueue([path]);
  }, [enqueue]);

  const reset = useCallback(() => {
    if (runningRef.current) return;
    setQueue([]);
    setError('');
  }, []);

  // Drag-and-drop via the Tauri webview — provides absolute file paths. Drag-drop
  // is an optional convenience; if the listener can't be registered the picker
  // button still works, so failures degrade gracefully rather than break the UI.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    try {
      getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === 'enter' || payload.type === 'over') {
          setIsDragging(true);
        } else if (payload.type === 'leave') {
          setIsDragging(false);
        } else if (payload.type === 'drop') {
          setIsDragging(false);
          if (payload.paths.length > 0) {
            enqueue(payload.paths);
          }
        }
      }).then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      }).catch((e) => {
        flog.warn('file-transcribe', 'drag-drop listener failed', { error: String(e) });
      });
    } catch (e) {
      flog.warn('file-transcribe', 'drag-drop unavailable', { error: String(e) });
    }

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enqueue]);

  const summary = summarize(queue);

  return {
    queue,
    summary,
    error,
    isDragging,
    isRunning,
    enqueue,
    transcribe,
    reset,
  };
}
