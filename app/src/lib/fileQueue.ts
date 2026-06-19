//! Pure queue-state helpers for multi-file transcription.
//!
//! The hook (`useFileTranscription`) drives the actual Rust calls; this module
//! holds only the side-effect-free list math so it can be unit-tested in
//! isolation (no Tauri, no React). A queue is an ordered list of items, each
//! carrying its own per-file status so one file's failure never aborts the rest.

const AUDIO_EXTENSIONS = ['wav', 'mp3', 'm4a'] as const;

export const UNSUPPORTED_MESSAGE = 'Unsupported file type. Use WAV, MP3, or M4A.';

export type QueueItemStatus = 'queued' | 'transcribing' | 'done' | 'error';

export interface QueueItem {
  /** Stable id for React keys and targeted updates (path + ordinal). */
  id: string;
  /** Absolute filesystem path passed to the Rust `transcribe_file` command. */
  path: string;
  /** Display name (basename of the path). */
  name: string;
  status: QueueItemStatus;
  /** Transcribed text once `done` (empty string allowed = no speech). */
  text?: string;
  /** Failure reason once `error`. */
  error?: string;
}

/** True when `path` ends in a supported audio extension (case-insensitive). */
export function hasAudioExtension(path: string): boolean {
  const ext = path.split('.').pop()?.toLowerCase();
  return !!ext && (AUDIO_EXTENSIONS as readonly string[]).includes(ext);
}

/** Last path segment, tolerant of both `/` and `\` separators. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/**
 * Build queue items for the audio paths in `paths`, skipping non-audio files and
 * any path already present in `existing` (dedupe across repeated drops/picks).
 * Ids are derived from the path plus a monotonic ordinal so duplicate basenames
 * across directories still get distinct React keys.
 */
export function buildQueueItems(paths: string[], existing: QueueItem[] = []): QueueItem[] {
  const seen = new Set(existing.map((i) => i.path));
  const items: QueueItem[] = [];
  let ordinal = existing.length;
  for (const path of paths) {
    if (!hasAudioExtension(path)) continue;
    if (seen.has(path)) continue;
    seen.add(path);
    items.push({
      id: `${ordinal}:${path}`,
      path,
      name: baseName(path),
      status: 'queued',
    });
    ordinal += 1;
  }
  return items;
}

/** Return a new queue with the item at `id` patched (immutable update). */
export function updateItem(
  queue: QueueItem[],
  id: string,
  patch: Partial<QueueItem>,
): QueueItem[] {
  return queue.map((item) => (item.id === id ? { ...item, ...patch } : item));
}

/** The first still-`queued` item, or `null` when the queue is fully processed. */
export function nextQueued(queue: QueueItem[]): QueueItem | null {
  return queue.find((item) => item.status === 'queued') ?? null;
}

/** Counts by status, useful for the summary line ("3 of 5 done, 1 error"). */
export interface QueueSummary {
  total: number;
  queued: number;
  transcribing: number;
  done: number;
  error: number;
  /** True when nothing is left queued or in-flight. */
  finished: boolean;
}

export function summarize(queue: QueueItem[]): QueueSummary {
  const counts = { queued: 0, transcribing: 0, done: 0, error: 0 };
  for (const item of queue) counts[item.status] += 1;
  return {
    total: queue.length,
    ...counts,
    finished: queue.length > 0 && counts.queued === 0 && counts.transcribing === 0,
  };
}

/** True when at least one supported audio file appears in `paths`. */
export function hasAnyAudio(paths: string[]): boolean {
  return paths.some(hasAudioExtension);
}
