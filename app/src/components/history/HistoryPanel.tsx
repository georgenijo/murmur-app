import { useState } from 'react';
import { HistoryEntry, formatTimestamp, clearHistory } from '../../lib/history';

interface HistoryPanelProps {
  entries: HistoryEntry[];
  onClearHistory: () => void;
}

export function HistoryPanel({ entries, onClearHistory }: HistoryPanelProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);

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
    if (wholeSeconds < 60) {
      return `${wholeSeconds}s`;
    }
    const mins = Math.floor(wholeSeconds / 60);
    const secs = wholeSeconds % 60;
    return `${mins}m ${secs}s`;
  };

  if (entries.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-on-surface-variant">
        <svg
          className="w-12 h-12 mb-3"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
          />
        </svg>
        <p className="text-sm">No transcription history yet</p>
        <p className="text-xs mt-1">Your transcriptions will appear here</p>
      </div>
    );
  }

  // Show entries in reverse chronological order (newest first)
  const sortedEntries = [...entries].reverse();

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {/* Scrollable list */}
      <div className="flex-1 overflow-y-auto space-y-3 px-0.5 py-0.5">
        {sortedEntries.map((entry) => {
          const wordCount = entry.text.trim() ? entry.text.trim().split(/\s+/).length : 0;
          return (
            <button
              key={entry.id}
              onClick={() => handleCopy(entry)}
              aria-label={`Copy transcription from ${formatTimestamp(entry.timestamp)}`}
              className={`group w-full rounded-xl p-3.5 text-left shadow-sm transition-[box-shadow,background-color] hover:shadow-md focus:outline-none focus-visible:ring-2 focus-visible:ring-primary ${
                copiedId === entry.id
                  ? 'bg-emerald-50 dark:bg-emerald-950/40'
                  : 'bg-surface-container-lowest hover:bg-surface-container-low'
              }`}
            >
            {/* Header row with timestamp and duration */}
            <div className="flex items-center justify-between mb-1 gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <span className="shrink-0 text-xs text-on-surface-variant">
                  {formatTimestamp(entry.timestamp)}
                </span>
                {(entry.source ?? 'recording') === 'file' ? (
                  <span
                    title={entry.sourceName}
                    className="inline-flex max-w-[180px] min-w-0 items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
                  >
                    <svg className="w-2.5 h-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
                    </svg>
                    <span className="truncate">{entry.sourceName || 'File'}</span>
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                    <svg className="w-2.5 h-2.5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 11a7 7 0 01-14 0m7 7v3m-4 0h8m-4-6a3 3 0 01-3-3V5a3 3 0 016 0v4a3 3 0 01-3 3z" />
                    </svg>
                    Mic
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2 shrink-0">
                <span className="rounded-full bg-surface-container px-2 py-0.5 text-[10px] font-medium text-on-surface-variant">
                  {wordCount} {wordCount === 1 ? 'word' : 'words'}
                </span>
                <span className="text-xs text-on-surface-variant">
                  {formatDuration(entry.duration)}
                </span>
                {copiedId === entry.id ? (
                  <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium">
                    Copied!
                  </span>
                ) : (
                  <svg
                    className="h-3.5 w-3.5 text-on-surface-variant opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                    />
                  </svg>
                )}
              </div>
            </div>
            {/* Text content */}
            <p className="max-h-32 overflow-y-auto text-sm leading-relaxed text-on-surface">
              {entry.text}
            </p>
            </button>
          );
        })}
      </div>

      {/* Clear history button */}
      <div className="mt-3 shrink-0 pt-1">
        <button
          onClick={handleClear}
          className="w-full rounded-lg bg-surface-container-lowest px-3 py-2 text-sm font-medium text-on-surface-variant shadow-sm transition-colors hover:bg-surface-container hover:text-error focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          Clear History
        </button>
      </div>
    </div>
  );
}
