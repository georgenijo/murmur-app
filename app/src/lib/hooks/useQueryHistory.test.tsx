import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { QueryHistoryPageV1 } from '../queryHistory';

type Listener = (event: { payload: unknown }) => void;
const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  clear: vi.fn(),
  listeners: new Map<string, Listener>(),
}));

vi.mock('../queryHistory', async importOriginal => ({
  ...(await importOriginal<typeof import('../queryHistory')>()),
  listQueryHistory: mocks.list,
  clearQueryHistory: mocks.clear,
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, listener: Listener) => {
    mocks.listeners.set(name, listener);
    return () => mocks.listeners.delete(name);
  }),
}));

import { useQueryHistory } from './useQueryHistory';

const emptyPage = (providerOffset = 0): QueryHistoryPageV1 => ({
  schemaVersion: 1,
  entries: [],
  total: 0,
  offset: providerOffset,
  hasMore: false,
});

describe('useQueryHistory', () => {
  let container: HTMLDivElement;
  let root: Root;
  let latest: ReturnType<typeof useQueryHistory>;

  function Harness({ active }: { active: boolean }) {
    latest = useQueryHistory(active);
    return null;
  }

  beforeEach(() => {
    mocks.list.mockReset().mockResolvedValue(emptyPage());
    mocks.clear.mockReset().mockResolvedValue(undefined);
    mocks.listeners.clear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('does not retrieve content until the Queries workspace is active', async () => {
    await act(async () => root.render(<Harness active={false} />));
    expect(mocks.list).not.toHaveBeenCalled();

    await act(async () => {
      root.render(<Harness active />);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.list).toHaveBeenCalledWith({ offset: 0, limit: 50, provider: null });

    await act(async () => root.render(<Harness active={false} />));
    expect(latest.entries).toEqual([]);
    expect(latest.total).toBe(0);
  });

  it('uses the selected provider and ignores a superseded response', async () => {
    let resolveFirst!: (page: QueryHistoryPageV1) => void;
    let resolveSecond!: (page: QueryHistoryPageV1) => void;
    mocks.list
      .mockImplementationOnce(() => new Promise<QueryHistoryPageV1>((resolve) => { resolveFirst = resolve; }))
      .mockImplementationOnce(() => new Promise<QueryHistoryPageV1>((resolve) => { resolveSecond = resolve; }));
    await act(async () => root.render(<Harness active />));

    await act(async () => latest.setProvider('codex'));
    expect(mocks.list).toHaveBeenLastCalledWith({ offset: 0, limit: 50, provider: 'codex' });

    const codexEntry = {
      schemaVersion: 1 as const,
      id: '1123456789abcdef0123456789abcdef',
      timestampMs: 20,
      provider: 'codex' as const,
      question: 'new',
      answer: 'answer',
      tokens: null,
      durationMs: 2,
      errorCode: null,
    };
    await act(async () => {
      resolveSecond({ schemaVersion: 1, entries: [codexEntry], total: 1, offset: 0, hasMore: false });
      await Promise.resolve();
    });
    await act(async () => {
      resolveFirst(emptyPage());
      await Promise.resolve();
    });
    expect(latest.entries).toEqual([codexEntry]);
  });

  it('refreshes on a valid content-free insertion event and clears in one action', async () => {
    await act(async () => {
      root.render(<Harness active />);
      await Promise.resolve();
      await Promise.resolve();
    });
    mocks.list.mockClear();
    await act(async () => {
      mocks.listeners.get('query-history-changed')?.({ payload: { kind: 'inserted' } });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(mocks.list).toHaveBeenCalledOnce();

    await act(async () => {
      await latest.clear();
    });
    expect(mocks.clear).toHaveBeenCalledOnce();
    expect(latest.entries).toEqual([]);
  });
});
