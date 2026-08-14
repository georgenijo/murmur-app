import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '../../lib/settings';
import type { AudioInputInventoryV1 } from '../../lib/audioDevices';

const mocks = vi.hoisted(() => ({
  start: vi.fn(async () => {}),
  stop: vi.fn(),
  summary: vi.fn(async () => ({ corpusDirectory: '/private/corpus', recordings: [] })),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('../../lib/corpusRecorder', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/corpusRecorder')>();
  return {
    ...actual,
    startCorpusRecording: mocks.start,
    stopCorpusRecording: mocks.stop,
    getCorpusSummary: mocks.summary,
    cancelCorpusRecording: vi.fn(async () => true),
    openCorpusFolder: vi.fn(async () => {}),
  };
});

import { CorpusRecorder, corpusMicrophoneAvailability } from './CorpusRecorder';

const available = {
  schemaVersion: 1 as const,
  revision: 1,
  status: 'available' as const,
  devices: [{ id: 'uid-1', name: 'Studio Mic' }],
  defaultInputId: 'uid-1',
  errorCode: null,
};

describe('CorpusRecorder microphone inventory consumer', () => {
  afterEach(() => vi.clearAllMocks());
  it('allows a selected ID only with current authoritative membership', () => {
    expect(corpusMicrophoneAvailability(available, 'uid-1').deviceSelectable).toBe(true);
    expect(corpusMicrophoneAvailability(available, 'missing').deviceSelectable).toBe(false);
  });

  it('retains stale names for display but exposes no selectable devices', () => {
    const stale = { ...available, status: 'stale' as const, errorCode: 'refreshPending' as const };
    const result = corpusMicrophoneAvailability(stale, 'uid-1');
    expect(result.displayDevices).toEqual(available.devices);
    expect(result.selectableDevices).toEqual([]);
    expect(result.deviceSelectable).toBe(false);
  });

  it('does not claim System Default is usable without authoritative topology', () => {
    expect(corpusMicrophoneAvailability(null, 'system_default').deviceSelectable).toBe(false);
    expect(corpusMicrophoneAvailability(available, 'system_default').deviceSelectable).toBe(true);
  });

  it('disables every saved-take recording path when inventory becomes stale', async () => {
    const recording = {
      entryId: 'entry-1', promptIndex: 1, promptId: 'open-project-dashboard',
      label: 'Short command', reference: 'Open the project dashboard.', take: 1,
      selected: true, fileName: 'take.wav', sha256: 'abc', recordedAt: '2026-08-14T00:00:00Z',
      sampleRate: 16_000, durationMs: 1000, peak: 0.4, rms: 0.1,
      clippingPercent: 0, deviceLabel: 'System Default', qualityWarnings: [],
    };
    mocks.stop.mockResolvedValue({ corpusDirectory: '/private/corpus', recording });
    const container = document.createElement('div');
    document.body.appendChild(container);
    const root = createRoot(container);
    const onBusyChange = vi.fn();
    const render = (audioInventory: AudioInputInventoryV1) => root.render(createElement(CorpusRecorder, {
      status: 'idle',
      benchmarkRunning: false,
      fileTranscribing: false,
      settings: DEFAULT_SETTINGS,
      audioInventory,
      onUpdateSettings: vi.fn(),
      onBusyChange,
    }));

    await act(async () => { render(available); await Promise.resolve(); });
    const record = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Record') as HTMLButtonElement;
    await act(async () => { record.click(); await Promise.resolve(); });
    const stop = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Stop & Save') as HTMLButtonElement;
    await act(async () => { stop.click(); await Promise.resolve(); });

    const stale = { ...available, revision: 2, status: 'stale' as const, errorCode: 'refreshPending' as const };
    await act(async () => { render(stale); await Promise.resolve(); });
    const primary = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Record Another Take') as HTMLButtonElement;
    const secondary = Array.from(container.querySelectorAll('button')).find((button) => button.textContent === 'Record another take') as HTMLButtonElement;
    expect(primary.disabled).toBe(true);
    expect(secondary.disabled).toBe(true);
    primary.click();
    secondary.click();
    expect(mocks.start).toHaveBeenCalledOnce();

    await act(async () => root.unmount());
    container.remove();
  });
});
