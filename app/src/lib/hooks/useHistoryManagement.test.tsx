import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { saveHistory, type HistoryEntry } from '../history';
import { useHistoryManagement } from './useHistoryManagement';

type HistoryState = ReturnType<typeof useHistoryManagement>;

function storedEntry(id: string, text: string): HistoryEntry {
  return { id, text, timestamp: 1, duration: 1, source: 'recording' };
}

describe('useHistoryManagement retention boundary', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: HistoryState;

  beforeEach(() => {
    localStorage.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function render(retainHistory: boolean) {
    function Harness({ retain }: { retain: boolean }) {
      current = useHistoryManagement(retain);
      return null;
    }
    await act(async () => root.render(<Harness retain={retainHistory} />));
  }

  it('keeps existing durable-cache entries while discarding new content when disabled', async () => {
    saveHistory([storedEntry('existing', 'keep me')]);
    await render(false);

    await act(async () => current.addEntry('discard me', 2));

    expect(current.historyEntries.map((entry) => entry.text)).toEqual(['keep me']);
    expect(JSON.parse(localStorage.getItem('dictation-history') ?? '[]')).toHaveLength(1);
  });

  it('persists new content after retention is enabled', async () => {
    await render(true);

    await act(async () => current.addEntry('keep me', 2));

    expect(current.historyEntries.map((entry) => entry.text)).toEqual(['keep me']);
    expect(JSON.parse(localStorage.getItem('dictation-history') ?? '[]')).toHaveLength(1);
  });

  it('persists pin changes across hook remounts and clears them with history', async () => {
    saveHistory([storedEntry('existing', 'keep me')]);
    await render(true);
    const target = current.historyEntries[0];
    await act(async () => current.togglePinned(target));
    expect(current.historyEntries[0].pinned).toBe(true);
    expect(JSON.parse(localStorage.getItem('dictation-history') ?? '[]')[0].pinned).toBe(true);

    await act(async () => root.unmount());
    container.remove();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await render(true);
    expect(current.historyEntries[0].pinned).toBe(true);
    await act(async () => current.clearHistory());
    expect(current.historyEntries).toEqual([]);
    expect(localStorage.getItem('dictation-history')).toBeNull();
  });

  it('deletes an exact pinned subset and frees its pin slot', async () => {
    saveHistory(Array.from({ length: 21 }, (_, index) => ({
      ...storedEntry(`entry-${index}`, `entry ${index}`),
      pinned: index < 20,
    })));
    await render(true);

    const removed = current.historyEntries[0];
    await act(async () => current.deleteEntries([removed]));

    expect(current.historyEntries.map((entry) => entry.id)).not.toContain('entry-0');
    expect(current.historyEntries.filter((entry) => entry.pinned)).toHaveLength(19);
    const newlyPinned = current.historyEntries.find((entry) => entry.id === 'entry-20')!;
    await act(async () => current.togglePinned(newlyPinned));
    expect(current.historyEntries.filter((entry) => entry.pinned)).toHaveLength(20);
    expect(JSON.parse(localStorage.getItem('dictation-history') ?? '[]')).toEqual(current.historyEntries);
  });
});
