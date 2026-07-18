import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
}

export function TranscriptionView({ historyEntries, onClearHistory }: TranscriptionViewProps) {
  return (
    <div className="flex flex-1 flex-col overflow-hidden rounded-2xl bg-surface-container-low p-3">
      <HistoryPanel
        entries={historyEntries}
        onClearHistory={onClearHistory}
      />
    </div>
  );
}
