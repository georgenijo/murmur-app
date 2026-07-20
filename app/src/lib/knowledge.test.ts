import { beforeEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import {
  deleteAllKnowledge,
  deleteKnowledge,
  listKnowledge,
  setKnowledgeEnabled,
  upsertKnowledge,
  type KnowledgeEntry,
} from './knowledge';

const entry: KnowledgeEntry = {
  id: 'record-1',
  payload: { kind: 'replacement_rule', source: 'Tory', replacement: 'Tauri' },
  enabled: true,
  scope: { kind: 'global' },
  provenance: 'manual',
  createdAtMs: 1,
  updatedAtMs: 1,
  revision: 7,
};

describe('knowledge command boundary', () => {
  beforeEach(() => invoke.mockReset());

  it('passes bounded filters as one request object', async () => {
    invoke.mockResolvedValueOnce({ entries: [], total: 0, nextOffset: null, storeRevision: 0 });
    await listKnowledge({ query: 'local only', limit: 50, offset: 0 });
    expect(invoke).toHaveBeenCalledWith('list_knowledge', {
      request: { query: 'local only', limit: 50, offset: 0 },
    });
  });

  it('uses optimistic revisions for update, toggle, delete, and delete-all', async () => {
    invoke.mockResolvedValue(undefined);
    await upsertKnowledge({
      id: entry.id,
      expectedRevision: entry.revision,
      payload: entry.payload,
      enabled: true,
      scope: entry.scope,
    });
    await setKnowledgeEnabled(entry, false);
    await deleteKnowledge(entry);
    await deleteAllKnowledge(12);

    expect(invoke.mock.calls).toEqual([
      ['upsert_knowledge', { draft: expect.objectContaining({ expectedRevision: 7 }) }],
      ['set_knowledge_enabled', { id: 'record-1', enabled: false, expectedRevision: 7 }],
      ['delete_knowledge', { id: 'record-1', expectedRevision: 7 }],
      ['delete_all_knowledge', { expectedRevision: 12 }],
    ]);
  });
});
