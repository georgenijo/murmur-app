import { useMemo } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { HistoryEntry } from '../../lib/history';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';
import { loadStats } from '../../lib/stats';
import { HistoryPanel } from '../history/HistoryPanel';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeInsightsRail } from './HomeInsightsRail';

interface HomeDashboardProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  onToggleHistoryPinned: (entry: HistoryEntry) => void;
  focusSearchToken?: number;
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
}

export function HomeDashboard({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  onToggleHistoryPinned,
  focusSearchToken,
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

          <section className="home-history" aria-labelledby="recent-dictations-title">
            <HistoryPanel
              title="Your dictations"
              titleId="recent-dictations-title"
              entries={historyEntries}
              onClear={onClearHistory}
              onUpdateEntry={onUpdateHistoryEntry}
              onTogglePinned={onToggleHistoryPinned}
              pinnedCount={historyEntries.filter((entry) => entry.pinned === true).length}
              focusSearchToken={focusSearchToken}
              onTranscribeFile={onTranscribeFile}
            />
          </section>
        </div>

        <HomeInsightsRail stats={stats} onOpenInsights={onOpenInsights} />
      </div>
    </div>
  );
}
