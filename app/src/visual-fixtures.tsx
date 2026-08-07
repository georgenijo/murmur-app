import React from 'react';
import ReactDOM from 'react-dom/client';
import { mockIPC } from '@tauri-apps/api/mocks';
import { MainHeader } from './components/MainHeader';
import { FooterStats } from './components/FooterStats';
import { HistoryPanel } from './components/history/HistoryPanel';
import { SettingsPanel } from './components/settings/SettingsPanel';
import { UpdateIndicator } from './components/UpdateIndicator';
import { DEFAULT_SETTINGS } from './lib/settings';
import type { DictationStatus } from './lib/types';
import './styles.css';

const query = new URLSearchParams(window.location.search);
const requestedState = query.get('state') ?? 'idle';
const appearance = query.get('appearance') === 'dark' ? 'dark' : 'light';
const status: DictationStatus = requestedState === 'recording'
  ? 'recording'
  : requestedState === 'processing'
    ? 'processing'
    : requestedState === 'update-recovering'
      ? 'recovering'
      : 'idle';

document.documentElement.dataset.appearance = appearance;

mockIPC((command) => {
  if (command === 'get_model_runtime_catalog' || command.startsWith('list_')) return [];
  if (command.includes('version')) return '0.27.0';
  return null;
}, { shouldMockEvents: true });

const entries = [
  {
    id: 'older',
    text: 'History owns the window.',
    timestamp: Date.UTC(2026, 7, 6, 14, 37),
    duration: 2,
    source: 'recording' as const,
  },
  {
    id: 'file',
    text: 'Imported audio uses the same spacing rhythm without taking over the workspace.',
    timestamp: Date.UTC(2026, 7, 6, 14, 47),
    duration: 38,
    source: 'file' as const,
    sourceName: 'design-review.wav',
  },
  {
    id: 'newest',
    text: 'The compact transcript keeps its metadata aligned and its actions quiet.',
    timestamp: Date.UTC(2026, 7, 6, 15, 26),
    duration: 8,
    source: 'recording' as const,
  },
];

function VisualFixture() {
  const settingsOpen = requestedState === 'settings';

  return (
    <div
      data-appearance={appearance}
      data-visual-ready="true"
      className="flex h-[720px] w-[880px] flex-col overflow-hidden bg-background text-on-surface"
    >
      <MainHeader
        status={status}
        initialized
        recordingDuration={12}
        recordingMode={requestedState === 'update-recovering' ? 'both' : 'hold_down'}
        onRecord={() => {}}
        onStop={() => {}}
        onOpenSettings={() => {}}
        settingsOpen={settingsOpen}
        triggerKey="shift_l"
        mode={settingsOpen ? 'settings' : 'main'}
        updateIndicator={requestedState === 'update-recovering' ? (
          <UpdateIndicator
            status={{
              phase: 'available',
              version: 'v0.27.1',
              notes: '',
              isForced: false,
            }}
            onOpen={() => {}}
            onRetryCheck={() => {}}
          />
        ) : undefined}
      />
      {settingsOpen ? (
        <SettingsPanel
          isOpen
          onClose={() => {}}
          settings={DEFAULT_SETTINGS}
          onUpdateSettings={() => {}}
          status="idle"
          onResetStats={() => {}}
          onRerunSetup={() => {}}
          accessibilityGranted
          onCheckForUpdate={async () => {}}
          updateStatus={{ phase: 'idle' }}
          configureError={null}
        />
      ) : (
        <>
          <HistoryPanel
            entries={entries}
            onClear={() => {}}
            onUpdateEntry={() => {}}
          />
          <FooterStats statsVersion={0} />
        </>
      )}
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <VisualFixture />
  </React.StrictMode>,
);
