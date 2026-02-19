import { HistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';
import { DictationStatus } from '../lib/types';

interface TranscriptionViewProps {
  transcription: string;
  status: DictationStatus;
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
}

export function TranscriptionView({ transcription, status, historyEntries, onClearHistory }: TranscriptionViewProps) {
  return (
    <div className="flex-1 overflow-y-auto flex flex-col gap-3">
      <div className="bg-white dark:bg-stone-800 rounded-lg border border-stone-200 dark:border-stone-700 p-4 min-h-[80px]">
        {transcription ? (
          <p className="text-stone-900 dark:text-stone-100 text-sm leading-relaxed whitespace-pre-wrap">
            {transcription}
          </p>
        ) : (
          <p className="text-stone-400 dark:text-stone-500 text-sm italic">
            {status === 'recording'
              ? 'Listening...'
              : status === 'processing'
              ? 'Transcribing audio...'
              : 'Press the button below to start recording'}
          </p>
        )}
      </div>

      <HistoryPanel
        entries={historyEntries}
        onClearHistory={onClearHistory}
      />
    </div>
  );
}
