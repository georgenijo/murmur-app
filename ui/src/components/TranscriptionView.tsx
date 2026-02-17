import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';
import { TabType } from './TabNavigation';

interface TranscriptionViewProps {
  activeTab: TabType;
  transcription: string;
  status: string;
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
}

export function TranscriptionView({ activeTab, transcription, status, historyEntries, onClearHistory }: TranscriptionViewProps) {
  return (
    <div className="flex-1 overflow-y-auto bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-4 flex flex-col">
      {activeTab === 'current' ? (
        transcription ? (
          <p className="text-gray-900 dark:text-gray-100 text-sm leading-relaxed whitespace-pre-wrap">
            {transcription}
          </p>
        ) : (
          <p className="text-gray-400 dark:text-gray-500 text-sm italic">
            {status === 'recording'
              ? 'Listening...'
              : status === 'processing'
              ? 'Transcribing audio...'
              : 'Press the button below to start recording'}
          </p>
        )
      ) : (
        <HistoryPanel
          entries={historyEntries}
          onClearHistory={onClearHistory}
        />
      )}
    </div>
  );
}
