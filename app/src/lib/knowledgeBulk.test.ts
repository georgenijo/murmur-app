import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { KnowledgeEntry, KnowledgeListRequest } from './knowledge';
import { applyKnowledgeBulk, collectKnowledgeSnapshot, type KnowledgeBulkAction } from './knowledgeBulk';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const entries: KnowledgeEntry[] = Array.from({ length: 55 }, (_, index) => ({
  id: `matching-${index}`,
  payload: { kind: 'replacement_rule', source: `term ${index}`, replacement: `written ${index}` },
  enabled: true,
  scope: { kind: 'app', bundleId: 'test.app' },
  provenance: 'manual',
  createdAtMs: 1,
  updatedAtMs: 1,
  revision: index + 1,
}));

describe('filtered knowledge batches', () => {
  beforeEach(() => { invoke.mockReset(); });

  it.each<KnowledgeBulkAction>(['enable', 'disable', 'delete'])('collects every page before %s changes the matching set', async (action) => {
    const enabled = action !== 'enable';
    const matching = entries.map((entry) => ({ ...entry, enabled }));
    const unrelated: KnowledgeEntry[] = [
      { ...entries[0], id: 'global', scope: { kind: 'global' } },
      { ...entries[0], id: 'other-state', enabled: !enabled },
      { ...entries[0], id: 'other-type', payload: { kind: 'snippet', trigger: 'term', body: 'snippet' } },
      { ...entries[0], id: 'other-query', payload: { kind: 'replacement_rule', source: 'other', replacement: 'word' } },
    ];
    const records = new Map([...matching, ...unrelated].map((entry) => [entry.id, entry]));
    let revision = 100;
    invoke.mockImplementation(async (command: string, args: { request?: KnowledgeListRequest; id?: string; expectedRevision?: number; enabled?: boolean }) => {
      if (command === 'list_knowledge') {
        const request = args.request;
        if (!request) throw new Error('Missing request');
        const matches = [...records.values()].filter((entry) => entry.enabled === request.enabled
          && entry.scope.kind === request.scopeKind && entry.payload.kind === request.kind
          && entry.payload.kind === 'replacement_rule' && entry.payload.source.includes(request.query ?? ''));
        const offset = request.offset ?? 0;
        const limit = request.limit ?? 50;
        return { entries: matches.slice(offset, offset + limit), total: matches.length,
          nextOffset: offset + limit < matches.length ? offset + limit : null, storeRevision: revision };
      }
      const entry = args.id ? records.get(args.id) : undefined;
      if (!entry || entry.revision !== args.expectedRevision) throw new Error('Revision conflict');
      if (command === 'delete_knowledge') records.delete(entry.id);
      else records.set(entry.id, { ...entry, enabled: args.enabled ?? entry.enabled, revision: entry.revision + 1 });
      revision += 1;
      return entry;
    });

    const signal = new AbortController().signal;
    const snapshot = await collectKnowledgeSnapshot({ query: 'term', kind: 'replacement_rule', enabled, scopeKind: 'app' }, signal);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_knowledge', 'list_knowledge']);
    const result = await applyKnowledgeBulk(snapshot, action, signal);
    expect(result).toEqual({ action, succeeded: 55, unchanged: 0, failures: [] });
    expect(invoke.mock.calls.slice(2).map(([, args]) => args.expectedRevision)).toEqual(matching.map((entry) => entry.revision));
    expect(unrelated.map((entry) => records.get(entry.id))).toEqual(unrelated);
    expect(matching.every((entry) => action === 'delete' ? !records.has(entry.id) : records.get(entry.id)?.enabled === (action === 'enable'))).toBe(true);
  });

  it('refuses a snapshot when the store changes between pages', async () => {
    invoke.mockResolvedValueOnce({ entries: entries.slice(0, 50), total: 55, nextOffset: 50, storeRevision: 100 });
    invoke.mockResolvedValueOnce({ entries: entries.slice(50), total: 55, nextOffset: null, storeRevision: 101 });
    await expect(collectKnowledgeSnapshot({}, new AbortController().signal)).rejects.toThrow('Refresh and try again. No records were changed.');
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(['list_knowledge', 'list_knowledge']);
  });

  it.each<KnowledgeBulkAction>(['enable', 'disable', 'delete'])('accounts for conflicts without retrying or stopping %s', async (action) => {
    invoke.mockResolvedValueOnce(undefined).mockRejectedValueOnce('Record revision conflict').mockResolvedValueOnce(undefined);
    const targets = entries.slice(0, 3).map((entry) => ({ ...entry, enabled: action !== 'enable' }));
    const result = await applyKnowledgeBulk(targets, action, new AbortController().signal);
    expect(result).toEqual({ action, succeeded: 2, unchanged: 0, failures: [{ entry: targets[1], message: 'Record revision conflict' }] });
    expect(invoke.mock.calls.map(([, args]) => [args.id, args.expectedRevision])).toEqual([
      ['matching-0', 1], ['matching-1', 2], ['matching-2', 3],
    ]);
  });

  it.each<KnowledgeBulkAction>(['enable', 'disable'])('keeps timestamps and precedence unchanged for records already matching %s', async (action) => {
    const unchanged = entries.map((entry) => ({ ...entry, enabled: action === 'enable' }));
    const result = await applyKnowledgeBulk(unchanged, action, new AbortController().signal);
    expect(result).toEqual({ action, succeeded: 0, unchanged: 55, failures: [] });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('stops before the next mutation when its owner aborts', async () => {
    const controller = new AbortController();
    invoke.mockImplementationOnce(async () => controller.abort());
    await expect(applyKnowledgeBulk(entries, 'delete', controller.signal)).rejects.toThrow();
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('does not use a page delivered after cancellation', async () => {
    const controller = new AbortController();
    invoke.mockImplementationOnce(async () => {
      controller.abort();
      return { entries: entries.slice(0, 50), total: 55, nextOffset: 50, storeRevision: 100 };
    });
    await expect(collectKnowledgeSnapshot({}, controller.signal)).rejects.toThrow();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
