import { useState, useEffect, useCallback, useRef } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { transcribeFile } from '../dictation';
import { flog } from '../log';

const AUDIO_EXTENSIONS = ['wav', 'mp3', 'm4a'];
const UNSUPPORTED_MESSAGE = 'Unsupported file type. Use WAV, MP3, or M4A.';

export type QueueItemStatus = 'pending' | 'processing' | 'complete' | 'error';

export interface FileQueueItem {
  id: string;
  path: string;
  name: string;
  status: QueueItemStatus;
  result?: string;
  error?: string;
  duration?: number;
}

function hasAudioExtension(path: string): boolean {
  const ext = path.split('.').pop()?.toLowerCase();
  return !!ext && AUDIO_EXTENSIONS.includes(ext);
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function partitionPaths(paths: string[]): { supported: string[]; unsupportedCount: number } {
  const supported: string[] = [];
  let unsupportedCount = 0;
  for (const path of paths) {
    if (hasAudioExtension(path)) {
      supported.push(path);
    } else {
      unsupportedCount += 1;
    }
  }
  return { supported, unsupportedCount };
}

function unsupportedWarning(count: number): string {
  if (count === 1) {
    return `Skipped 1 unsupported file. ${UNSUPPORTED_MESSAGE}`;
  }
  return `Skipped ${count} unsupported file(s). ${UNSUPPORTED_MESSAGE}`;
}

interface UseFileTranscriptionProps {
  /** Persist completed transcriptions to shared history (no WPM stats). */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

/**
 * Manages batch file transcription: queues paths, drains sequentially via Rust
 * `transcribe_file`, and wires window drag-and-drop (Tauri webview paths).
 */
export function useFileTranscription({ addEntry }: UseFileTranscriptionProps) {
  const [items, setItems] = useState<FileQueueItem[]>([]);
  const [batchWarning, setBatchWarning] = useState('');
  const [isDragging, setIsDragging] = useState(false);
  const [isDraining, setIsDraining] = useState(false);

  const itemsRef = useRef<FileQueueItem[]>([]);

  const setItemsSynced = useCallback((updater: (prev: FileQueueItem[]) => FileQueueItem[]) => {
    setItems((prev) => {
      const next = updater(prev);
      itemsRef.current = next;
      return next;
    });
  }, []);

  const batchGenerationRef = useRef(0);
  const drainingRef = useRef(false);

  const isProcessing = isDraining || items.some((i) => i.status === 'processing');

  const updateItem = useCallback(
    (id: string, patch: Partial<FileQueueItem>) => {
      setItemsSynced((prev) =>
        prev.map((item) => (item.id === id ? { ...item, ...patch } : item)),
      );
    },
    [setItemsSynced],
  );

  const drainQueue = useCallback(
    async (generation: number) => {
      if (drainingRef.current) return;
      drainingRef.current = true;
      setIsDraining(true);

      try {
        while (batchGenerationRef.current === generation) {
          const next = itemsRef.current.find((i) => i.status === 'pending');
          if (!next) break;

          updateItem(next.id, { status: 'processing', error: undefined, result: undefined });
          flog.info('file-transcribe', 'start', { name: next.name });

          const res = await transcribeFile(next.path);

          if (batchGenerationRef.current !== generation) {
            flog.info('file-transcribe', 'stale result ignored', { name: next.name });
            return;
          }

          if (res.type === 'error') {
            const err = res.error || 'Transcription failed';
            updateItem(next.id, { status: 'error', error: err });
            flog.warn('file-transcribe', 'error', { error: err });
            continue;
          }

          const text = res.text || '';
          updateItem(next.id, {
            status: 'complete',
            result: text,
            duration: res.duration ?? 0,
          });
          if (text.trim()) {
            addEntry(text, res.duration ?? 0, 'file', next.name);
          }
          flog.info('file-transcribe', 'complete', { textLen: text.length });
        }
      } finally {
        drainingRef.current = false;
        setIsDraining(false);
        // Another enqueue may have added pending items while we were finishing.
        if (
          batchGenerationRef.current === generation &&
          itemsRef.current.some((i) => i.status === 'pending')
        ) {
          void drainQueue(generation);
        }
      }
    },
    [addEntry, updateItem],
  );

  const enqueuePaths = useCallback(
    (paths: string[]) => {
      if (paths.length === 0) return;

      const { supported, unsupportedCount } = partitionPaths(paths);
      if (unsupportedCount > 0) {
        setBatchWarning(unsupportedWarning(unsupportedCount));
      }

      if (supported.length === 0) return;

      const activePaths = new Set(
        itemsRef.current
          .filter((i) => i.status === 'pending' || i.status === 'processing')
          .map((i) => i.path),
      );
      const toAdd = supported.filter((path) => !activePaths.has(path));
      if (toAdd.length === 0) return;

      const newItems: FileQueueItem[] = toAdd.map((path) => ({
        id: crypto.randomUUID(),
        path,
        name: baseName(path),
        status: 'pending',
      }));

      setItemsSynced((prev) => [...prev, ...newItems]);

      const generation = batchGenerationRef.current;
      void drainQueue(generation);
    },
    [drainQueue, setItemsSynced],
  );

  const reset = useCallback(() => {
    batchGenerationRef.current += 1;
    drainingRef.current = false;
    setIsDraining(false);
    itemsRef.current = [];
    setItems([]);
    setBatchWarning('');
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    try {
      getCurrentWebview()
        .onDragDropEvent((event) => {
          const payload = event.payload;
          if (payload.type === 'enter' || payload.type === 'over') {
            setIsDragging(true);
          } else if (payload.type === 'leave') {
            setIsDragging(false);
          } else if (payload.type === 'drop') {
            setIsDragging(false);
            if (payload.paths.length > 0) {
              enqueuePaths(payload.paths);
            }
          }
        })
        .then((fn) => {
          if (cancelled) fn();
          else unlisten = fn;
        })
        .catch((e) => {
          flog.warn('file-transcribe', 'drag-drop listener failed', { error: String(e) });
        });
    } catch (e) {
      flog.warn('file-transcribe', 'drag-drop unavailable', { error: String(e) });
    }

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enqueuePaths]);

  return {
    items,
    batchWarning,
    isDragging,
    isProcessing,
    enqueuePaths,
    reset,
  };
}
