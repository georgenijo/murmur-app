import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useFileTranscription, type FileQueueItem } from '../lib/hooks/useFileTranscription';
import { flog } from '../lib/log';

interface FileTranscriptionPanelProps {
  /** Persist completed transcriptions to shared history. */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

function formatCopyAll(items: FileQueueItem[]): string {
  return items
    .filter((i) => i.status === 'complete')
    .map((i) => {
      const text = i.result?.trim() || 'No speech detected in this file.';
      return `${i.name}\n\n${text}`;
    })
    .join('\n\n');
}

function StatusIcon({ status }: { status: FileQueueItem['status'] }) {
  if (status === 'processing') {
    return (
      <svg className="w-4 h-4 animate-spin shrink-0 text-stone-500" fill="none" viewBox="0 0 24 24">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
    );
  }
  if (status === 'complete') {
    return (
      <svg className="w-4 h-4 shrink-0 text-emerald-600 dark:text-emerald-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
      </svg>
    );
  }
  if (status === 'error') {
    return (
      <svg className="w-4 h-4 shrink-0 text-red-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
      </svg>
    );
  }
  return (
    <span className="w-4 h-4 shrink-0 rounded-full border-2 border-stone-300 dark:border-stone-600" />
  );
}

function statusLabel(status: FileQueueItem['status']): string {
  switch (status) {
    case 'pending':
      return 'Queued';
    case 'processing':
      return 'Transcribing…';
    case 'complete':
      return 'Done';
    case 'error':
      return 'Failed';
  }
}

export function FileTranscriptionPanel({ addEntry }: FileTranscriptionPanelProps) {
  const { items, batchWarning, isDragging, isProcessing, enqueuePaths, reset } =
    useFileTranscription({ addEntry });
  const [copiedIds, setCopiedIds] = useState<Set<string>>(new Set());
  const [copyAllDone, setCopyAllDone] = useState(false);

  const completeItems = items.filter((i) => i.status === 'complete');
  const hasQueue = items.length > 0;

  const handlePick = async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'm4a'] }],
      });
      if (selected === null) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      enqueuePaths(paths);
    } catch (e) {
      flog.warn('file-transcribe', 'file dialog failed', { error: String(e) });
    }
  };

  const handleCopyItem = async (item: FileQueueItem) => {
    const text = item.result?.trim() || '';
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopiedIds((prev) => new Set(prev).add(item.id));
      setTimeout(() => {
        setCopiedIds((prev) => {
          const next = new Set(prev);
          next.delete(item.id);
          return next;
        });
      }, 2000);
    } catch (e) {
      console.error('Failed to copy:', e);
    }
  };

  const handleCopyAll = async () => {
    const text = formatCopyAll(items);
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopyAllDone(true);
      setTimeout(() => setCopyAllDone(false), 2000);
    } catch (e) {
      console.error('Failed to copy:', e);
    }
  };

  const handleReset = () => {
    reset();
    setCopiedIds(new Set());
    setCopyAllDone(false);
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden gap-4">
      {/* Drop zone + picker */}
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
          className="px-4 py-2 text-sm font-medium rounded-lg bg-stone-800 text-white hover:bg-stone-700 dark:bg-stone-200 dark:text-stone-900 dark:hover:bg-stone-100 transition-colors"
        >
          Choose Files
        </button>
        <div className="text-xs text-stone-400 dark:text-stone-500">WAV, MP3, or M4A</div>
      </div>

      {batchWarning && (
        <div className="shrink-0 px-4 py-3 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg">
          <p className="text-amber-800 dark:text-amber-300 text-sm">{batchWarning}</p>
        </div>
      )}

      {hasQueue && (
        <div className="flex-1 flex flex-col overflow-hidden rounded-xl border border-stone-200 dark:border-stone-700 bg-white dark:bg-stone-800 min-h-0">
          <div className="shrink-0 flex items-center justify-between px-4 py-2 border-b border-stone-200 dark:border-stone-700 gap-2">
            <span className="text-xs font-medium text-stone-500 dark:text-stone-400">
              {isProcessing
                ? `Transcribing (${items.filter((i) => i.status === 'complete' || i.status === 'error').length}/${items.length})`
                : `${items.length} file${items.length === 1 ? '' : 's'}`}
            </span>
            <div className="flex items-center gap-3">
              {completeItems.length > 0 && (
                <button
                  onClick={handleCopyAll}
                  className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 transition-colors"
                >
                  {copyAllDone ? (
                    <span className="text-emerald-600 dark:text-emerald-400">Copied all!</span>
                  ) : (
                    'Copy all'
                  )}
                </button>
              )}
              <button
                onClick={handleReset}
                className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 transition-colors"
              >
                Clear
              </button>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto divide-y divide-stone-200 dark:divide-stone-700">
            {items.map((item) => (
              <div key={item.id} className="px-4 py-3 flex flex-col gap-2">
                <div className="flex items-start gap-2">
                  <StatusIcon status={item.status} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium text-stone-800 dark:text-stone-200 truncate">
                        {item.name}
                      </span>
                      <span className="text-xs text-stone-400 dark:text-stone-500 shrink-0">
                        {statusLabel(item.status)}
                      </span>
                    </div>
                    {item.status === 'error' && item.error && (
                      <p className="mt-1 text-xs text-red-600 dark:text-red-400">{item.error}</p>
                    )}
                  </div>
                  {item.status === 'complete' && (
                    <button
                      onClick={() => handleCopyItem(item)}
                      disabled={!item.result?.trim()}
                      className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 disabled:opacity-50 shrink-0 transition-colors"
                    >
                      {copiedIds.has(item.id) ? (
                        <span className="text-emerald-600 dark:text-emerald-400">Copied!</span>
                      ) : (
                        'Copy'
                      )}
                    </button>
                  )}
                </div>
                {item.status === 'complete' && (
                  <div className="pl-6 text-sm text-stone-700 dark:text-stone-300 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">
                    {item.result?.trim() || (
                      <span className="text-stone-400 dark:text-stone-500">No speech detected in this file.</span>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
