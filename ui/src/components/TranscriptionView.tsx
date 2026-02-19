import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
}

export function TranscriptionView({ historyEntries, onClearHistory }: TranscriptionViewProps) {
  return (
    <div className="flex-1 overflow-y-auto">
      <HistoryPanel
        entries={historyEntries}
        onClearHistory={onClearHistory}
      />
    </div>
  );
}
