import { useEffect } from 'react';
import type { QueueItem } from '../lib/fileQueue';

interface FileTranscriptionToastsProps {
  queue: QueueItem[];
  error: string;
  onCancel: (id: string) => void;
  onDismiss: (id: string) => void;
}

function statusText(item: QueueItem): string {
  if (item.status === 'transcribing') return 'Transcribing…';
  if (item.status === 'done') return 'Done';
  if (item.status === 'error') return item.error || 'Could not transcribe';
  return 'Queued';
}

export function FileTranscriptionToasts({
  queue,
  error,
  onCancel,
  onDismiss,
}: FileTranscriptionToastsProps) {
  const visible = queue.slice(-3).reverse();

  useEffect(() => {
    const doneIds = queue.filter((item) => item.status === 'done').map((item) => item.id);
    if (doneIds.length === 0) return;
    const timer = setTimeout(() => doneIds.forEach((id) => onDismiss(id)), 3000);
    return () => clearTimeout(timer);
  }, [queue, onDismiss]);

  if (visible.length === 0 && !error) return null;

  return (
    <div className="pointer-events-none absolute bottom-4 right-4 z-30 flex w-[min(340px,calc(100vw-32px))] flex-col gap-2" aria-live="polite">
      {error && (
        <div className="dialog-toast pointer-events-auto border-error/30 p-3 text-xs text-error">
          {error}
        </div>
      )}
      {visible.map((item) => (
        <div key={item.id} className="dialog-toast pointer-events-auto overflow-hidden">
          <div className="flex items-start gap-3 p-3">
            <span className={`mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-[var(--ui-radius-control)] ${
              item.status === 'done' ? 'bg-success/10 text-success' : item.status === 'error' ? 'bg-error/10 text-error' : 'bg-surface-container-high text-on-surface'
            }`}>
              {item.status === 'transcribing' ? (
                <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary/25 border-t-primary" />
              ) : item.status === 'done' ? '✓' : item.status === 'error' ? '!' : '♪'}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-xs font-semibold text-on-surface" title={item.name}>{item.name}</span>
              <span className={`mt-0.5 block text-[11px] ${
                item.status === 'error' ? 'text-error' : item.status === 'done' ? 'text-success' : 'text-on-surface-variant'
              }`}>{statusText(item)}</span>
            </span>
            {item.status === 'transcribing' && (
              <button type="button" onClick={() => onCancel(item.id)} className="rounded-[var(--ui-radius-control)] px-1.5 py-0.5 text-[11px] font-semibold text-on-surface-variant hover:bg-surface-container hover:text-on-surface">
                Cancel
              </button>
            )}
            {(item.status === 'done' || item.status === 'error') && (
              <button type="button" onClick={() => onDismiss(item.id)} aria-label={`Dismiss ${item.name}`} className="rounded-[var(--ui-radius-control)] p-0.5 text-on-surface-variant hover:bg-surface-container hover:text-on-surface">×</button>
            )}
          </div>
          {item.status === 'transcribing' && (
            <div className="h-0.5 overflow-hidden bg-surface-container-high">
              <div className="model-download-indeterminate h-full bg-[linear-gradient(140deg,var(--murmur-primary),var(--murmur-primary-dim))]" />
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
