import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
}

export function TranscriptionView({ historyEntries, onClearHistory, onUpdateHistoryEntry }: TranscriptionViewProps) {
  return (
    <div className="flex flex-1 flex-col overflow-hidden rounded-2xl bg-surface-container-low p-3">
      <HistoryPanel
        entries={historyEntries}
        onClearHistory={onClearHistory}
        onUpdateEntry={onUpdateHistoryEntry}
      />
    </div>
  );
}
