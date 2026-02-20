import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';
import type { DictationStatus } from '../lib/types';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  partialText?: string;
  status?: DictationStatus;
}

export function TranscriptionView({ historyEntries, onClearHistory, partialText, status }: TranscriptionViewProps) {
  const showPartial = status === 'processing' && partialText && partialText.length > 0;

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      {showPartial && (
        <div className="shrink-0 px-3 py-2 mb-2 bg-stone-100 dark:bg-stone-800 rounded-lg border border-stone-200 dark:border-stone-700">
          <p className="text-sm text-stone-400 dark:text-stone-500 italic leading-snug">
            {partialText}
          </p>
        </div>
      )}
      <HistoryPanel
        entries={historyEntries}
        onClearHistory={onClearHistory}
      />
    </div>
  );
}
