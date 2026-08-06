import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  focusSearchToken?: number;
  onTranscribeFile: () => void;
}

export function TranscriptionView({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  focusSearchToken,
  onTranscribeFile,
}: TranscriptionViewProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <HistoryPanel
        entries={historyEntries}
        onClear={onClearHistory}
        onUpdateEntry={onUpdateHistoryEntry}
        focusSearchToken={focusSearchToken}
        onTranscribeFile={onTranscribeFile}
      />
    </div>
  );
}
