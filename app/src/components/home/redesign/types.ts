import type { HistoryEntry } from '../../../lib/history';
import type { Settings } from '../../../lib/settings';
import type { DictationStatus } from '../../../lib/types';
import type { useMeetings } from '../../../lib/hooks/useMeetings';

/** Same contract HomeDashboard receives, so a variant can be promoted 1:1. */
export interface HomeRedesignProps {
  historyEntries: HistoryEntry[];
  onClearHistory: () => void;
  onUpdateHistoryEntry: (id: string, text: string) => void;
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
  onOpenSettings: (target: { page: string; editorTab?: 'vocabulary' | 'aliases' | 'knowledge' | 'transforms' | 'commands' | 'scan'; target?: string }) => void;
}
