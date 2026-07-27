import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearUnpinned: () => void;
  onClearAllHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  onTogglePin: (id: string) => void;
  focusSearchToken?: number;
}

export function TranscriptionView({
  historyEntries,
  onClearUnpinned,
  onClearAllHistory,
  onUpdateHistoryEntry,
  onTogglePin,
  focusSearchToken,
}: TranscriptionViewProps) {
  return (
    <div className="flex flex-1 flex-col overflow-hidden rounded-2xl bg-surface-container-low p-3">
      <HistoryPanel
        entries={historyEntries}
        onClearUnpinned={onClearUnpinned}
        onClearAll={onClearAllHistory}
        onUpdateEntry={onUpdateHistoryEntry}
        onTogglePin={onTogglePin}
        focusSearchToken={focusSearchToken}
      />
    </div>
  );
}
