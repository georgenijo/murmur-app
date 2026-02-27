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
    if (seconds < 60) {
      return `${seconds}s`;
    }
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}m ${secs}s`;
  };

  if (entries.length === 0) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center text-stone-400 dark:text-stone-500">
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
      <div className="flex-1 overflow-y-auto space-y-2">
        {sortedEntries.map((entry) => (
          <button
            key={entry.id}
            onClick={() => handleCopy(entry)}
            className={`w-full text-left p-3 rounded-lg border transition-all ${
              copiedId === entry.id
                ? 'bg-emerald-50 dark:bg-emerald-900/20 border-emerald-300 dark:border-emerald-700'
                : 'bg-stone-50 dark:bg-stone-700/50 border-stone-200 dark:border-stone-600 hover:bg-stone-100 dark:hover:bg-stone-700'
            }`}
          >
            {/* Header row with timestamp and duration */}
            <div className="flex items-center justify-between mb-1">
              <span className="text-xs text-stone-500 dark:text-stone-400">
                {formatTimestamp(entry.timestamp)}
              </span>
              <div className="flex items-center gap-2">
                <span className="text-xs text-stone-400 dark:text-stone-500">
                  {formatDuration(entry.duration)}
                </span>
                {copiedId === entry.id ? (
                  <span className="text-xs text-emerald-600 dark:text-emerald-400 font-medium">
                    Copied!
                  </span>
                ) : (
                  <svg
                    className="w-3.5 h-3.5 text-stone-400 dark:text-stone-500"
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
            <p className="text-sm text-stone-900 dark:text-stone-100 overflow-y-auto max-h-32">
              {entry.text}
            </p>
          </button>
        ))}
      </div>

      {/* Clear history button */}
      <div className="shrink-0 pt-3 border-t border-stone-200 dark:border-stone-700 mt-3">
        <button
          onClick={handleClear}
          className="w-full px-3 py-2 text-sm text-stone-500 dark:text-stone-400 hover:bg-stone-100 dark:hover:bg-stone-700 rounded-lg transition-colors"
        >
          Clear History
        </button>
      </div>
    </div>
  );
}
