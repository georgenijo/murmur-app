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
});
