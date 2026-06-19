import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useFileTranscription } from '../lib/hooks/useFileTranscription';
import type { QueueItem } from '../lib/hooks/useFileTranscription';
import { flog } from '../lib/log';

interface FileTranscriptionPanelProps {
  /** Persist completed transcriptions to shared history. */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

/** Per-file status pill in the queue list. */
function StatusBadge({ item }: { item: QueueItem }) {
  switch (item.status) {
    case 'queued':
      return <span className="text-xs text-stone-400 dark:text-stone-500">Queued</span>;
    case 'transcribing':
      return (
        <span className="flex items-center gap-1.5 text-xs text-stone-600 dark:text-stone-300">
          <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          Transcribing
        </span>
      );
    case 'done':
      return <span className="text-xs text-emerald-600 dark:text-emerald-400">Done</span>;
    case 'error':
      return <span className="text-xs text-red-600 dark:text-red-400">Error</span>;
  }
}

export function FileTranscriptionPanel({ addEntry }: FileTranscriptionPanelProps) {
  const { queue, summary, error, isDragging, isRunning, enqueue, reset } = useFileTranscription({ addEntry });
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const handlePick = async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'm4a'] }],
      });
      // With `multiple: true` the dialog returns string[] | null.
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length > 0) enqueue(paths);
    } catch (e) {
      flog.warn('file-transcribe', 'file dialog failed', { error: String(e) });
    }
  };

  const handleCopy = async (item: QueueItem) => {
    if (!item.text) return;
    try {
      await navigator.clipboard.writeText(item.text);
      setCopiedId(item.id);
      setTimeout(() => setCopiedId((id) => (id === item.id ? null : id)), 2000);
    } catch (e) {
      console.error('Failed to copy:', e);
    }
  };

  const hasQueue = queue.length > 0;

  return (
    <div className="flex-1 flex flex-col overflow-hidden gap-4">
      {/* Drop zone + multi-select picker */}
      <div
        className={`shrink-0 rounded-xl border-2 border-dashed p-8 flex flex-col items-center justify-center text-center gap-3 transition-colors ${
          isDragging
            ? 'border-stone-500 bg-stone-100 dark:border-stone-400 dark:bg-stone-800'
            : 'border-stone-300 dark:border-stone-700'
        }`}
      >
        <svg className="w-8 h-8 text-stone-400 dark:text-stone-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
        </svg>
        <div className="text-sm text-stone-600 dark:text-stone-400">
          Drag audio files here, or
        </div>
        <button
          onClick={handlePick}
          className="px-4 py-2 text-sm font-medium rounded-lg bg-stone-800 text-white hover:bg-stone-700 disabled:opacity-50 disabled:cursor-not-allowed dark:bg-stone-200 dark:text-stone-900 dark:hover:bg-stone-100 transition-colors"
        >
          Choose Files
        </button>
        <div className="text-xs text-stone-400 dark:text-stone-500">WAV, MP3, or M4A — multiple files supported</div>
      </div>

      {/* Unsupported-type / dialog error (queue-level, not per-file) */}
      {error && (
        <div className="shrink-0 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
          <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
        </div>
      )}

      {/* Queue list with per-file status + results */}
      {hasQueue && (
        <div className="flex-1 flex flex-col overflow-hidden rounded-xl border border-stone-200 dark:border-stone-700 bg-white dark:bg-stone-800">
          <div className="shrink-0 flex items-center justify-between px-4 py-2 border-b border-stone-200 dark:border-stone-700">
            <span className="text-xs font-medium text-stone-500 dark:text-stone-400">
              {summary.finished
                ? `Finished — ${summary.done} done${summary.error > 0 ? `, ${summary.error} error${summary.error > 1 ? 's' : ''}` : ''}`
                : `Transcribing ${summary.done + summary.error} of ${summary.total}…`}
            </span>
            {summary.finished && !isRunning && (
              <button
                onClick={reset}
                className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 transition-colors"
              >
                Clear
              </button>
            )}
          </div>
          <div className="flex-1 overflow-y-auto divide-y divide-stone-100 dark:divide-stone-700/60">
            {queue.map((item) => (
              <div key={item.id} className="px-4 py-3 flex flex-col gap-1.5">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-sm text-stone-800 dark:text-stone-200 truncate" title={item.name}>
                    {item.name}
                  </span>
                  <div className="shrink-0 flex items-center gap-3">
                    <StatusBadge item={item} />
                    {item.status === 'done' && item.text && item.text.trim() && (
                      <button
                        onClick={() => handleCopy(item)}
                        className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 transition-colors"
                      >
                        {copiedId === item.id ? (
                          <span className="text-emerald-600 dark:text-emerald-400">Copied!</span>
                        ) : (
                          'Copy'
                        )}
                      </button>
                    )}
                  </div>
                </div>
                {item.status === 'done' && (
                  <p className="text-sm text-stone-600 dark:text-stone-300 whitespace-pre-wrap break-words">
                    {item.text && item.text.trim()
                      ? item.text
                      : <span className="text-stone-400 dark:text-stone-500">No speech detected in this file.</span>}
                  </p>
                )}
                {item.status === 'error' && item.error && (
                  <p className="text-sm text-red-600 dark:text-red-400 break-words">{item.error}</p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
