import { useEffect, useMemo, useState } from 'react';
import type { VocabScanStats, VocabScanStatus, WalkerRow } from '../../lib/hooks/useVocabScan';
import type { Settings } from '../../lib/settings';
import { vocabularyPrompt } from '../../lib/settings';
import { KnowledgeManager } from './KnowledgeManager';
import { TransformsManager } from './TransformsManager';
import { VocabScanStrip } from './VocabScanStrip';
import { VocabularyAliasesEditor } from './VocabularyAliasesEditor';
import { VoiceCommandsManager } from './VoiceCommandsManager';

export type SettingsEditorTab =
  | 'vocabulary'
  | 'aliases'
  | 'knowledge'
  | 'transforms'
  | 'commands'
  | 'scan';

const EDITOR_LABELS: Record<SettingsEditorTab, string> = {
  vocabulary: 'Vocabulary',
  aliases: 'Aliases',
  knowledge: 'Knowledge',
  transforms: 'Saved Transforms',
  commands: 'Voice Commands',
  scan: 'Project Scan',
};

interface SettingsEditorsWindowProps {
  initialTab: SettingsEditorTab;
  settings: Settings;
  onUpdateSettings: (updates: Partial<Settings>) => void;
  scanStatus: VocabScanStatus;
  scanWalker: WalkerRow[];
  scanStats: VocabScanStats;
  onChooseCodeFolder: () => void;
  onClearCodeFolder: () => void;
  onScan: () => void;
  onCancelScan: () => void;
  onBack: () => void;
  backLabel?: string;
}

function ScannedVocabulary({ settings }: { settings: Settings }) {
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<'frequency' | 'alpha'>('frequency');
  const summary = settings.codeVocabLastScan;
  const terms = useMemo(() => {
    const filtered = (summary?.rankedTerms ?? []).filter((term) =>
      term.term.toLowerCase().includes(query.trim().toLowerCase()));
    return sort === 'alpha'
      ? [...filtered].sort((left, right) => left.term.localeCompare(right.term))
      : filtered;
  }, [query, sort, summary?.rankedTerms]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="mb-4">
        <h2 className="text-base font-semibold text-on-surface">Scanned vocabulary</h2>
        <p className="mt-1 max-w-2xl text-xs leading-relaxed text-on-surface-variant">
          Identifiers from the latest project scan, ranked by frequency. The highest-ranked terms feed the model prompt; every retained term remains available to local correction.
        </p>
      </div>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <label className="relative min-w-[260px] flex-1">
          <span className="sr-only">Filter terms</span>
          <span aria-hidden="true" className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-on-surface-variant">⌕</span>
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter terms…" className="h-10 w-full rounded-xl border border-outline-variant bg-surface-container-lowest pl-9 pr-3 text-sm outline-none focus:border-primary" />
        </label>
        <select value={sort} onChange={(event) => setSort(event.target.value as 'frequency' | 'alpha')} className="h-10 rounded-xl border border-outline-variant bg-surface-container-high px-3 text-xs text-on-surface">
          <option value="frequency">Sort: frequency</option>
          <option value="alpha">Sort: A–Z</option>
        </select>
        <span className="text-xs text-on-surface-variant">
          {summary ? `${summary.whisperCount} feed the model · ${summary.terms} total` : 'No project scan yet'}
        </span>
      </div>
      <div className="settings-card min-h-0 flex-1 overflow-hidden">
        {!summary ? (
          <div className="grid h-full min-h-48 place-items-center px-6 text-center text-sm text-on-surface-variant">
            Run a Project Scan to populate local developer vocabulary.
          </div>
        ) : (
          <>
            <div className="grid grid-cols-[48px_minmax(0,1fr)_90px_180px] gap-3 border-b border-outline-variant/20 bg-surface-container-low px-4 py-2.5 text-[10px] font-bold uppercase tracking-[0.12em] text-on-surface-variant">
              <span>#</span><span>Term</span><span className="text-right">Freq</span><span>Feeds</span>
            </div>
            <div className="h-full max-h-[460px] overflow-y-auto">
              {terms.map((term) => {
                const rank = summary.rankedTerms.findIndex((candidate) => candidate.term === term.term) + 1;
                return (
                  <div key={term.term} className="grid grid-cols-[48px_minmax(0,1fr)_90px_180px] items-center gap-3 border-b border-outline-variant/15 px-4 py-3 text-sm last:border-b-0">
                    <span className="text-on-surface-variant">{rank}</span>
                    <code className="truncate font-mono text-on-surface">{term.term}</code>
                    <span className="text-right tabular-nums text-on-surface-variant">{term.freq}×</span>
                    <span className="rounded-lg bg-surface-container-high px-3 py-1.5 text-xs text-on-surface">
                      {rank <= summary.whisperCount ? 'Model prompt' : 'Correction only'}
                    </span>
                  </div>
                );
              })}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export function SettingsEditorsWindow({
  initialTab,
  settings,
  onUpdateSettings,
  scanStatus,
  scanWalker,
  scanStats,
  onChooseCodeFolder,
  onClearCodeFolder,
  onScan,
  onCancelScan,
  onBack,
  backLabel = 'Back to Text settings',
}: SettingsEditorsWindowProps) {
  useEffect(() => {
    const handleEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      event.preventDefault();
      onBack();
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [onBack]);

  return (
    <section aria-labelledby="settings-editor-title" className="flex min-h-0 flex-1 flex-col">
      <div className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2">
        <button
          type="button"
          onClick={onBack}
          className="settings-back-btn focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
        >
          <span aria-hidden="true">‹</span>
          {backLabel}
        </button>
        <div>
          <p className="settings-eyebrow">Text editor</p>
          <h1 id="settings-editor-title" className="text-base font-semibold text-on-surface">
            {EDITOR_LABELS[initialTab]}
          </h1>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        {initialTab === 'vocabulary' && <ScannedVocabulary settings={settings} />}
        {initialTab === 'aliases' && (
          <VocabularyAliasesEditor
            entries={settings.vocabularyEntries}
            voiceCommands={settings.voiceCommands}
            onChange={(vocabularyEntries) => onUpdateSettings({
              vocabularyEntries,
              customVocabulary: vocabularyPrompt(vocabularyEntries),
            })}
          />
        )}
        {initialTab === 'knowledge' && <KnowledgeManager active profiles={settings.appProfiles} />}
        {initialTab === 'transforms' && <TransformsManager active />}
        {initialTab === 'commands' && (
          <VoiceCommandsManager
            active
            globallyEnabled={settings.voiceCommandsEnabled}
            profiles={settings.appProfiles}
          />
        )}
        {initialTab === 'scan' && (
          <section>
            <h2 className="text-base font-semibold text-on-surface">Project Scan</h2>
            <p className="mt-1 text-xs text-on-surface-variant">Scan one local source folder for identifiers. Dependency and build directories remain excluded.</p>
            <div className="settings-card mt-4 p-4">
              <p className="break-all rounded-lg bg-surface-container-low px-3 py-2 text-xs text-on-surface">
                {settings.codeVocabFolder || 'No project folder selected'}
              </p>
              <div className="mt-3 flex gap-2">
                <button type="button" onClick={onChooseCodeFolder} className="rounded-(--ui-radius-pill) bg-primary shadow-(--ui-shadow-accent) px-3 py-2 text-xs font-semibold text-on-primary">Choose Folder</button>
                {settings.codeVocabFolder && <button type="button" onClick={onClearCodeFolder} className="rounded-lg bg-surface-container-high px-3 py-2 text-xs font-semibold text-on-surface-variant">Clear</button>}
              </div>
              <VocabScanStrip
                status={scanStatus}
                walker={scanWalker}
                stats={scanStats}
                folder={settings.codeVocabFolder}
                onScan={onScan}
                onCancel={onCancelScan}
              />
            </div>
          </section>
        )}
      </div>
    </section>
  );
}
