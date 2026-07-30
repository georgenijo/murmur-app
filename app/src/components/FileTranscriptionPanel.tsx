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
      return <span className="text-xs text-on-surface-variant">Queued</span>;
    case 'transcribing':
      return (
        <span className="flex items-center gap-1.5 text-xs text-on-surface-variant">
          <svg className="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          Transcribing
        </span>
      );
    case 'done':
      return <span className="text-xs text-success">Done</span>;
    case 'error':
      return <span className="text-xs text-error">Error</span>;
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
            ? 'border-primary bg-surface-container'
            : 'border-outline-variant/40'
        }`}
      >
        <svg className="w-8 h-8 text-on-surface-variant" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
        </svg>
        <div className="text-sm text-on-surface-variant">
          Drag audio files here, or
        </div>
        <button
          onClick={handlePick}
          className="px-4 py-2 text-sm font-medium rounded-lg bg-surface-container-highest text-on-surface hover:bg-surface-container-high disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          Choose Files
        </button>
        <div className="text-xs text-on-surface-variant">WAV, MP3, or M4A — multiple files supported</div>
      </div>

      {/* Unsupported-type / dialog error (queue-level, not per-file) */}
      {error && (
        <div className="shrink-0 px-4 py-3 bg-error/10 border border-error/30 rounded-lg">
          <p className="text-error text-sm">{error}</p>
        </div>
      )}

      {/* Queue list with per-file status + results */}
      {hasQueue && (
        <div className="flex-1 flex flex-col overflow-hidden rounded-xl border border-outline-variant/40 bg-surface-container-lowest">
          <div className="shrink-0 flex items-center justify-between px-4 py-2 border-b border-outline-variant/40">
            <span className="text-xs font-medium text-on-surface-variant">
              {summary.finished
                ? `Finished — ${summary.done} done${summary.error > 0 ? `, ${summary.error} error${summary.error > 1 ? 's' : ''}` : ''}`
                : `Transcribing ${summary.done + summary.error} of ${summary.total}…`}
            </span>
            {summary.finished && !isRunning && (
              <button
                onClick={reset}
                className="text-xs font-medium text-on-surface-variant hover:text-on-surface transition-colors"
              >
                Clear
              </button>
            )}
          </div>
          <div className="flex-1 overflow-y-auto divide-y divide-outline-variant/40">
            {queue.map((item) => (
              <div key={item.id} className="px-4 py-3 flex flex-col gap-1.5">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-sm text-on-surface truncate" title={item.name}>
                    {item.name}
                  </span>
                  <div className="shrink-0 flex items-center gap-3">
                    <StatusBadge item={item} />
                    {item.status === 'done' && item.text && item.text.trim() && (
                      <button
                        onClick={() => handleCopy(item)}
                        className="text-xs font-medium text-on-surface-variant hover:text-on-surface transition-colors"
                      >
                        {copiedId === item.id ? (
                          <span className="text-success">Copied!</span>
                        ) : (
                          'Copy'
                        )}
                      </button>
                    )}
                  </div>
                </div>
                {item.status === 'done' && (
                  <p className="text-sm text-on-surface-variant whitespace-pre-wrap break-words">
                    {item.text && item.text.trim()
                      ? item.text
                      : <span className="text-on-surface-variant">No speech detected in this file.</span>}
                  </p>
                )}
                {item.status === 'error' && item.error && (
                  <p className="text-sm text-error break-words">{item.error}</p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
