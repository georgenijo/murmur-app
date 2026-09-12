import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeEntry, KnowledgeListRequest, KnowledgeListResponse } from '../knowledge';
import { useKnowledge } from './useKnowledge';

const mocks = vi.hoisted(() => ({
  status: vi.fn(),
  list: vi.fn<(request: KnowledgeListRequest) => Promise<KnowledgeListResponse>>(),
}));
vi.mock('../knowledge', () => ({ getKnowledgeStatus: mocks.status, listKnowledge: mocks.list }));

const entry: KnowledgeEntry = {
  id: 'first', payload: { kind: 'snippet', trigger: 'first', body: 'text' },
  enabled: true, scope: { kind: 'global' }, provenance: 'manual',
  createdAtMs: 1, updatedAtMs: 1, revision: 1,
};

function page(entries: KnowledgeEntry[], storeRevision = 10, nextOffset: number | null = null): KnowledgeListResponse {
  return { entries, total: nextOffset === null ? entries.length : entries.length + 1, storeRevision, nextOffset };
}

function pendingPage() {
  let resolvePage: (page: KnowledgeListResponse) => void = () => { throw new Error('Promise was not initialized'); };
  const promise = new Promise<KnowledgeListResponse>((resolve) => { resolvePage = resolve; });
  return { promise, resolve: resolvePage };
}

describe('useKnowledge request ownership', () => {
  let container: HTMLDivElement;
  let root: Root;
  let current: ReturnType<typeof useKnowledge>;

  function Harness({ request, active = true }: { request: KnowledgeListRequest; active?: boolean }) {
    current = useKnowledge(request, active);
    return <div>{current.entries.map((item) => item.id).join(',')} {current.error}</div>;
  }

  beforeEach(() => {
    mocks.list.mockReset();
    mocks.status.mockReset().mockResolvedValue({ availability: 'ready', schemaVersion: 2, recordCount: 2, storeRevision: 10, recoveryAtMs: null, message: null });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it('ignores an older filter response after the new filter has loaded', async () => {
    const old = pendingPage();
    mocks.list.mockReturnValueOnce(old.promise).mockResolvedValueOnce(page([{ ...entry, id: 'current' }]));
    await act(async () => root.render(<Harness request={{ query: 'old' }} />));
    const oldRefresh = current.refresh;
    await act(async () => root.render(<Harness request={{ query: 'current' }} />));
    expect(container.textContent).toContain('current');
    await act(async () => old.resolve(page([{ ...entry, id: 'obsolete' }])));
    await act(async () => oldRefresh());
    expect(container.textContent).not.toContain('obsolete');
    expect(current.entries.map((item) => item.id)).toEqual(['current']);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(current.loading).toBe(false);
  });

  it('hides previous matches while the new filter is still loading', async () => {
    const next = pendingPage();
    mocks.list.mockResolvedValueOnce(page([entry])).mockReturnValueOnce(next.promise);
    await act(async () => root.render(<Harness request={{ scopeKind: 'global' }} />));
    expect(current.entries).toHaveLength(1);
    await act(async () => root.render(<Harness request={{ scopeKind: 'app' }} />));
    expect(current.entries).toEqual([]);
    expect(current.total).toBe(0);
    expect(current.loading).toBe(true);
    await act(async () => next.resolve(page([])));
  });

  it('ignores a pending loadMore after a scope change and rejects repeat loads', async () => {
    const old = pendingPage();
    mocks.list.mockResolvedValueOnce(page([entry], 10, 1)).mockReturnValueOnce(old.promise)
      .mockResolvedValueOnce(page([{ ...entry, id: 'app-match' }]));
    await act(async () => root.render(<Harness request={{ scopeKind: 'global' }} />));
    await act(async () => { void current.loadMore(); void current.loadMore(); });
    expect(mocks.list).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<Harness request={{ scopeKind: 'app' }} />));
    await act(async () => old.resolve(page([{ ...entry, id: 'old-page' }])));
    expect(current.entries.map((item) => item.id)).toEqual(['app-match']);
    expect(current.loading).toBe(false);
  });

  it('refreshes instead of appending when store revision changes between visible pages', async () => {
    mocks.list.mockResolvedValueOnce(page([entry], 10, 1))
      .mockResolvedValueOnce(page([{ ...entry, id: 'drifted-page' }], 11))
      .mockResolvedValueOnce(page([{ ...entry, id: 'fresh-first-page' }], 11));
    await act(async () => root.render(<Harness request={{ enabled: true }} />));
    await act(async () => current.loadMore());
    expect(current.entries.map((item) => item.id)).toEqual(['fresh-first-page']);
    expect(mocks.list.mock.calls.map(([request]) => request.offset)).toEqual([0, 1, 0]);
    expect(current.loading).toBe(false);
  });

  it('allows refresh after a failed new filter load', async () => {
    mocks.list.mockRejectedValueOnce('Temporary failure').mockResolvedValueOnce(page([entry]));
    await act(async () => root.render(<Harness request={{ query: 'first' }} />));
    expect(current.loading).toBe(false);
    expect(current.error).toBe('Temporary failure');
    await act(async () => current.refresh());
    expect(current.entries).toEqual([entry]);
    expect(current.error).toBeNull();
  });

  it.each(['manual refresh', 'drift refresh'])('keeps the loaded page revision after a failed %s', async (trigger) => {
    mocks.list.mockResolvedValueOnce(page([entry], 10, 1));
    await act(async () => root.render(<Harness request={{ enabled: true }} />));
    mocks.status.mockResolvedValue({ availability: 'ready', schemaVersion: 2, recordCount: 2, storeRevision: 11, recoveryAtMs: null, message: null });
    if (trigger === 'drift refresh') mocks.list.mockResolvedValueOnce(page([{ ...entry, id: 'drifted-page' }], 11));
    mocks.list.mockRejectedValueOnce('Temporary failure');
    await act(async () => trigger === 'manual refresh' ? current.refresh() : current.loadMore());
    expect(current.error).toBe('Temporary failure');
    mocks.list.mockResolvedValueOnce(page([{ ...entry, id: 'must-not-append' }], 11))
      .mockResolvedValueOnce(page([{ ...entry, id: 'fresh-revision-11' }], 11));
    await act(async () => current.loadMore());
    expect(current.entries.map((item) => item.id)).toEqual(['fresh-revision-11']);
    expect(current.error).toBeNull();
    expect(mocks.list.mock.calls.slice(-2).map(([request]) => request.offset)).toEqual([1, 0]);
  });

  it('invalidates pending work on deactivation and refreshes on return', async () => {
    const old = pendingPage();
    mocks.list.mockReturnValueOnce(old.promise).mockResolvedValueOnce(page([entry]));
    await act(async () => root.render(<Harness request={{}} />));
    await act(async () => root.render(<Harness request={{}} active={false} />));
    await act(async () => old.resolve(page([{ ...entry, id: 'obsolete' }])));
    expect(current.entries).toEqual([]);
    await act(async () => root.render(<Harness request={{}} />));
    expect(current.entries).toEqual([entry]);
  });
});
