import type { MicPermissionStatus } from '../../lib/dictation';
import { defaultButtonKeyDown } from '../../lib/defaultButton';
import { useMemo } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { HistoryEntry } from '../../lib/history';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';
import type { SmartDestinationStatus } from '../../lib/smartDestination';
import { loadStats } from '../../lib/stats';
import { HistoryPanel } from '../history/HistoryPanel';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeInsightsRail } from './HomeInsightsRail';

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
  microphonePermission?: MicPermissionStatus;
  initialized: boolean;
  recordingDuration: number;
  audioLevel: number;
  smartDestinationStatus?: SmartDestinationStatus | null;
  smartDestinationAppName?: string | null;
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
  onDeleteHistoryEntries,
  onUpdateHistoryEntry,
  onToggleHistoryPinned,
  focusSearchToken,
  teachLatestToken,
  onTeachLatestHandled,
  onTranscribeFile,
  status,
  initialized,
  microphonePermission,
  recordingDuration,
  audioLevel,
  smartDestinationStatus = null,
  smartDestinationAppName = null,
  settings,
  meetings,
  statsVersion,
  onRecord,
  onStop,
  onOpenInsights,
}: HomeDashboardProps) {
  const stats = useMemo(() => loadStats(), [statsVersion]);

  return (
    <div onKeyDown={defaultButtonKeyDown} className="home-dashboard">
      <div className="home-dashboard-main">
        <div className="home-dashboard-top">
          <HomeRecordingBar
            status={status}
            initialized={initialized}
            microphonePermission={microphonePermission}
            recordingDuration={recordingDuration}
            audioLevel={audioLevel}
            smartDestinationStatus={smartDestinationStatus}
            smartDestinationAppName={smartDestinationAppName}
            triggerKey={settings.doubleTapKey}
            recordingMode={settings.recordingMode}
            meetingPhase={meetings.status.phase}
            onRecord={onRecord}
            onStop={onStop}
            footer={historyEntries.length > 0 ? <HomeInsightsRail stats={stats} onOpenInsights={onOpenInsights} /> : undefined}
          />
        </div>

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
    </div>
  );
}
