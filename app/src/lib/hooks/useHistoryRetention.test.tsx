import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { saveHistory, type HistoryEntry, type HistoryRetentionDays } from '../history';
import { HISTORY_RETENTION_SWEEP_MS, useHistoryManagement } from './useHistoryManagement';

type HistoryState = ReturnType<typeof useHistoryManagement>;
const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.UTC(2026, 10, 27, 12);

function stored(id: string, timestamp: number, pinned = false): HistoryEntry {
  return { schemaVersion: 2, id, text: id, timestamp, duration: 1, source: 'recording', pinned };
}

function storedIds(): string[] {
  return (JSON.parse(localStorage.getItem('dictation-history') ?? '[]') as HistoryEntry[]).map((e) => e.id);
}

describe('useHistoryManagement automatic deletion', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: HistoryState;

  function Harness({ days }: { days: HistoryRetentionDays }) {
    current = useHistoryManagement(true, days);
    return null;
  }
  const render = (days: HistoryRetentionDays) => act(async () => root.render(<Harness days={days} />));

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    localStorage.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  it('leaves history untouched by default', async () => {
    saveHistory([stored('ancient', NOW - 500 * DAY)]);
    function HarnessDefault() {
      current = useHistoryManagement(true);
      return null;
    }
    await act(async () => root.render(<HarnessDefault />));
    await act(async () => { vi.advanceTimersByTime(3 * HISTORY_RETENTION_SWEEP_MS); });
    expect(current.historyEntries.map((e) => e.id)).toEqual(['ancient']);
    expect(storedIds()).toEqual(['ancient']);
  });

  it('sweeps on enable, keeps pins, and persists the result', async () => {
    saveHistory([stored('old', NOW - 10 * DAY), stored('old-pinned', NOW - 10 * DAY, true), stored('new', NOW - DAY)]);
    await render(0);
    expect(current.countExpired(7)).toBe(1);
    expect(current.historyEntries).toHaveLength(3);

    await render(7);
    expect(current.historyEntries.map((e) => e.id)).toEqual(['old-pinned', 'new']);
    expect(storedIds()).toEqual(['old-pinned', 'new']);
    expect(current.countExpired(7)).toBe(0);
  });

  it('re-sweeps hourly while the app stays open', async () => {
    saveHistory([stored('aging', NOW - DAY + 30 * 60 * 1000)]);
    await render(1);
    expect(current.historyEntries).toHaveLength(1);

    await act(async () => { vi.advanceTimersByTime(HISTORY_RETENTION_SWEEP_MS); });
    expect(current.historyEntries).toHaveLength(0);
    expect(storedIds()).toEqual([]);
  });

  it('stops sweeping after switching back to Never', async () => {
    saveHistory([stored('aging', NOW - DAY + 30 * 60 * 1000)]);
    await render(1);
    await render(0);
    await act(async () => { vi.advanceTimersByTime(5 * HISTORY_RETENTION_SWEEP_MS); });
    expect(current.historyEntries.map((e) => e.id)).toEqual(['aging']);
  });
});
