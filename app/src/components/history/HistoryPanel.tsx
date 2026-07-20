import { useState } from 'react';
import { HistoryEntry, formatTimestamp, clearHistory } from '../../lib/history';
import { CorrectAndTeachDialog } from './CorrectAndTeachDialog';

interface HistoryPanelProps {
  entries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateEntry: (id: string, text: string) => void;
}

export function HistoryPanel({ entries, onClearHistory, onUpdateEntry }: HistoryPanelProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [teachingEntry, setTeachingEntry] = useState<HistoryEntry | null>(null);

  const handleCopy = async (entry: HistoryEntry) => {
    try {
      await navigator.clipboard.writeText(entry.text);
      setCopiedId(entry.id);
      setTimeout(() => setCopiedId(null), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  };

  const handleClear = () => {
    if (window.confirm('Are you sure you want to clear all history?')) {
      clearHistory();
      onClearHistory();
    }
  };

  const formatDuration = (seconds: number): string => {
    const wholeSeconds = Math.round(seconds);
    if (wholeSeconds < 60) return `${wholeSeconds}s`;
    const mins = Math.floor(wholeSeconds / 60);
    return `${mins}m ${wholeSeconds % 60}s`;
  };

  if (entries.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center text-on-surface-variant">
        <svg className="mb-3 h-12 w-12" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        <p className="text-sm">No transcription history yet</p>
        <p className="mt-1 text-xs">Your transcriptions will appear here</p>
      </div>
    );
  }

  const sortedEntries = [...entries].reverse();

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="flex-1 space-y-3 overflow-y-auto px-0.5 py-0.5">
        {sortedEntries.map((entry, index) => {
          const wordCount = entry.text.trim() ? entry.text.trim().split(/\s+/).length : 0;
          return (
            <article key={entry.id} className={`group w-full rounded-xl p-3.5 text-left shadow-sm transition-[box-shadow,background-color] hover:shadow-md ${copiedId === entry.id ? 'bg-emerald-50 dark:bg-emerald-950/40' : 'bg-surface-container-lowest hover:bg-surface-container-low'}`}>
              <div className="mb-1 flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="shrink-0 text-xs text-on-surface-variant">{formatTimestamp(entry.timestamp)}</span>
                  {(entry.source ?? 'recording') === 'file' ? (
                    <span title={entry.sourceName} className="inline-flex max-w-[180px] min-w-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" /></svg>
                      <span className="truncate">{entry.sourceName || 'File'}</span>
                    </span>
                  ) : (
                    <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                      <svg className="h-2.5 w-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11a7 7 0 01-14 0m7 7v3m-4 0h8m-4-6a3 3 0 01-3-3V5a3 3 0 016 0v4a3 3 0 01-3 3z" /></svg>
                      Mic
                    </span>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">{wordCount} {wordCount === 1 ? 'word' : 'words'}</span>
                  <span className="text-xs text-on-surface-variant">{formatDuration(entry.duration)}</span>
                  {copiedId === entry.id ? (
                    <span className="text-xs font-medium text-emerald-600 dark:text-emerald-400">Copied!</span>
                  ) : (
                    <button type="button" onClick={() => void handleCopy(entry)} aria-label={`Copy transcription from ${formatTimestamp(entry.timestamp)}`} className="rounded-md px-2 py-1 text-xs font-medium text-on-surface-variant hover:bg-surface-container hover:text-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Copy</button>
                  )}
                </div>
              </div>
              <p className="max-h-32 overflow-y-auto text-sm leading-relaxed text-on-surface">{entry.text}</p>
              {index === 0 && (
                <div className="mt-3 border-t border-outline-variant/20 pt-2">
                  <button type="button" onClick={() => setTeachingEntry(entry)} className="rounded-md px-2 py-1 text-xs font-semibold text-primary hover:bg-primary/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Correct and Teach</button>
                </div>
              )}
            </article>
          );
        })}
      </div>

      <div className="mt-3 shrink-0 pt-1">
        <button onClick={handleClear} className="w-full rounded-lg bg-surface-container-lowest px-3 py-2 text-sm font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary">Clear History</button>
      </div>

      {teachingEntry && (
        <CorrectAndTeachDialog
          entry={teachingEntry}
          onClose={() => setTeachingEntry(null)}
          onSaveCorrection={(text) => onUpdateEntry(teachingEntry.id, text)}
        />
      )}
    </div>
  );
}
