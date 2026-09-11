import { useMemo } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { HistoryEntry } from '../../lib/history';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';
import { loadStats } from '../../lib/stats';
import { HistoryPanel } from '../history/HistoryPanel';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeInsightsRail } from './HomeInsightsRail';
import { DiscoveryChecklist } from './DiscoveryChecklist';
import type { ChecklistItemId } from '../../lib/discovery';

interface HomeDashboardProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onDeleteHistoryEntries: (entries: readonly HistoryEntry[]) => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  onToggleHistoryPinned: (entry: HistoryEntry) => void;
  focusSearchToken?: number;
  teachLatestToken?: number;
  onTeachLatestHandled?: () => void;
  onTranscribeFile: () => void;
  status: DictationStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel: number;
  settings: Settings;
  meetings: ReturnType<typeof useMeetings>;
  statsVersion: number;
  onRecord: () => void;
  onStop: () => void;
  onOpenInsights: () => void;
  discovery?: {
    completed: readonly ChecklistItemId[];
    onAction: (id: ChecklistItemId) => void;
    onDismiss: () => void;
  };
}

export function HomeDashboard({
  historyEntries,
  onClearHistory,
  onDeleteHistoryEntries,
  onUpdateHistoryEntry,
  onToggleHistoryPinned,
  focusSearchToken,
  teachLatestToken,
  onTeachLatestHandled,
  onTranscribeFile,
  status,
  initialized,
  recordingDuration,
  audioLevel,
  settings,
  meetings,
  statsVersion,
  onRecord,
  onStop,
  onOpenInsights,
  discovery,
}: HomeDashboardProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);

  return (
    <div className="home-dashboard">
      <div className="home-dashboard-grid">
        <div className="home-dashboard-main">
          <HomeRecordingBar
            status={status}
            initialized={initialized}
            recordingDuration={recordingDuration}
            audioLevel={audioLevel}
            triggerKey={settings.doubleTapKey}
            recordingMode={settings.recordingMode}
            meetingPhase={meetings.status.phase}
            onRecord={onRecord}
            onStop={onStop}
          />

          {discovery && (
            <DiscoveryChecklist
              completed={discovery.completed}
              onAction={discovery.onAction}
              onDismiss={discovery.onDismiss}
            />
          )}

          <section className="home-history" aria-labelledby="recent-dictations-title">
            <HistoryPanel
              title="Your dictations"
              titleId="recent-dictations-title"
              entries={historyEntries}
              onClear={onClearHistory}
              onDeleteEntries={onDeleteHistoryEntries}
              onUpdateEntry={onUpdateHistoryEntry}
              onTogglePinned={onToggleHistoryPinned}
              pinnedCount={historyEntries.filter((entry) => entry.pinned === true).length}
              focusSearchToken={focusSearchToken}
              teachLatestToken={teachLatestToken}
              onTeachLatestHandled={onTeachLatestHandled}
              onTranscribeFile={onTranscribeFile}
            />
          </section>
        </div>

        <HomeInsightsRail stats={stats} onOpenInsights={onOpenInsights} />
      </div>
    </div>
  );
}
