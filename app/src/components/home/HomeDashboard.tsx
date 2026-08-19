import { useEffect, useMemo, useState } from 'react';
import type { useMeetings } from '../../lib/hooks/useMeetings';
import type { HistoryEntry, HistoryStageResult } from '../../lib/history';
import type { Settings } from '../../lib/settings';
import type { DictationStatus } from '../../lib/types';
import { loadStats } from '../../lib/stats';
import { derivePersonalization, getUsageOverview } from '../../lib/homeDashboard';
import { HistoryPanel } from '../history/HistoryPanel';
import { HomeRecordingBar } from './HomeRecordingBar';
import { HomeInsightsRail } from './HomeInsightsRail';

const TIP_DISMISSED_KEY = 'murmur-home-styles-tip-dismissed';

interface SettingsTarget {
  page: string;
  editorTab?: 'vocabulary' | 'aliases' | 'knowledge' | 'transforms' | 'commands' | 'scan';
  target?: string;
}

interface HomeDashboardProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
  onAddDerivedHistoryEntry?: (source: HistoryEntry, text: string, modeId: string, stages: HistoryStageResult[]) => void;
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
  onOpenSettings: (target: SettingsTarget) => void;
}

function loadTipDismissed(): boolean {
  try { return localStorage.getItem(TIP_DISMISSED_KEY) === 'true'; }
  catch { return false; }
}

export function HomeDashboard({
  historyEntries,
  onClearHistory,
  onUpdateHistoryEntry,
  onAddDerivedHistoryEntry = () => {},
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
  onOpenSettings,
}: HomeDashboardProps) {
  const [tipDismissed, setTipDismissed] = useState(loadTipDismissed);
  const stats = useMemo(() => loadStats(), [statsVersion]);
  const usage = useMemo(() => getUsageOverview(stats), [stats]);
  const personalization = useMemo(
    () => derivePersonalization(settings, stats),
    [settings.vocabularyEntries, settings.appProfiles, stats],
  );
  const hasConfiguredStyle = settings.appProfiles.some((profile) => profile.writingStyle !== null);
  const showStylesTip = !tipDismissed && !hasConfiguredStyle;

  useEffect(() => {
    if (hasConfiguredStyle) setTipDismissed(true);
  }, [hasConfiguredStyle]);

  const dismissTip = () => {
    setTipDismissed(true);
    try { localStorage.setItem(TIP_DISMISSED_KEY, 'true'); } catch { /* presentation-only */ }
  };

  return (
    <div className="home-dashboard">
      <header className="home-dashboard-heading">
        <div>
          <h1>Ready when you are</h1>
          <p>{usage.recordingsThisMonth.toLocaleString()} {usage.recordingsThisMonth === 1 ? 'dictation' : 'dictations'} this month · everything processed locally</p>
        </div>
      </header>

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
            onTranscribeFile={onTranscribeFile}
          />

          {showStylesTip && (
            <section className="home-styles-tip" aria-label="Set up app styles">
              <span className="home-tip-icon" aria-hidden="true">⌁</span>
              <p><strong>Make Murmur sound like you.</strong> Set a writing style for each app.</p>
              <button type="button" onClick={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })}>Set up styles</button>
              <button type="button" onClick={dismissTip} aria-label="Dismiss styles tip" className="home-tip-dismiss">×</button>
            </section>
          )}

          <section className="home-history" aria-labelledby="recent-dictations-title">
            <div className="home-history-heading">
              <h2 id="recent-dictations-title">Recent dictations</h2>
              <span>{historyEntries.length} {historyEntries.length === 1 ? 'entry' : 'entries'}</span>
            </div>
            <HistoryPanel
              entries={historyEntries}
              onClear={onClearHistory}
              onUpdateEntry={onUpdateHistoryEntry}
              modes={settings.modes}
              onAddDerived={onAddDerivedHistoryEntry}
              focusSearchToken={focusSearchToken}
              onTranscribeFile={onTranscribeFile}
            />
          </section>
        </div>

        <HomeInsightsRail
          stats={stats}
          personalization={personalization}
          onOpenInsights={onOpenInsights}
          onOpenVocabulary={() => onOpenSettings({ page: 'text', editorTab: 'aliases' })}
          onOpenStyles={() => onOpenSettings({ page: 'delivery', target: 'app-overrides' })}
        />
      </div>
    </div>
  );
}
