import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  focusSearchToken?: number;
}

export function TranscriptionView({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  focusSearchToken,
}: TranscriptionViewProps) {
  return (
    <div className="flex flex-1 flex-col overflow-hidden rounded-2xl bg-surface-container-low p-3">
      <HistoryPanel
        entries={historyEntries}
        onClear={onClearHistory}
        onUpdateEntry={onUpdateHistoryEntry}
        focusSearchToken={focusSearchToken}
      />
    </div>
  );
}
