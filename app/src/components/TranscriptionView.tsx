import { HistoryPanel, MeetingsPanel, QueryHistoryPanel } from './history';
import { HistoryEntry } from '../lib/history';
import { memo } from 'react';
import type { useMeetings } from '../lib/hooks/useMeetings';
import type { useQueryHistory } from '../lib/hooks/useQueryHistory';

export type HistoryWorkspace = 'transcripts' | 'meetings' | 'queries';

interface TranscriptionViewProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  focusSearchToken?: number;
  onTranscribeFile: () => void;
  workspace: HistoryWorkspace;
  onWorkspaceChange: (workspace: HistoryWorkspace) => void;
  meetings: ReturnType<typeof useMeetings>;
  queryHistory: ReturnType<typeof useQueryHistory>;
  queryHistoryActive: boolean;
  retainQueryHistory: boolean;
}

export const TranscriptionView = memo(function TranscriptionView({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  focusSearchToken,
  onTranscribeFile,
  workspace,
  onWorkspaceChange,
  meetings,
  queryHistory,
  queryHistoryActive,
  retainQueryHistory,
}: TranscriptionViewProps) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex shrink-0 justify-center border-b border-[var(--ui-hairline)] px-4 py-2">
        <div className="inline-flex rounded-[var(--ui-radius-pill)] bg-[var(--ui-tint-sunken)] p-0.5" role="tablist" aria-label="History workspace">
          {(['transcripts', 'meetings', 'queries'] as const).map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={workspace === value}
              onClick={() => onWorkspaceChange(value)}
              className={`rounded-[var(--ui-radius-pill)] px-4 py-1.5 text-xs font-semibold capitalize transition-colors ${
                workspace === value
                  ? 'bg-[var(--ui-tint-raised)] text-on-surface shadow-[var(--ui-shadow-1)]'
                  : 'text-on-surface-variant hover:text-on-surface'
              }`}
            >
              {value}
            </button>
          ))}
        </div>
      </div>
      {workspace === 'transcripts' ? (
        <HistoryPanel
          entries={historyEntries}
          onClear={onClearHistory}
          onUpdateEntry={onUpdateHistoryEntry}
          focusSearchToken={focusSearchToken}
          onTranscribeFile={onTranscribeFile}
        />
      ) : workspace === 'meetings' ? (
        <MeetingsPanel meetings={meetings} />
      ) : queryHistoryActive ? (
        <QueryHistoryPanel history={queryHistory} retentionEnabled={retainQueryHistory} />
      ) : null}
    </div>
  );
});
