import {
  deleteKnowledge,
  listKnowledge,
  setKnowledgeEnabled,
  type KnowledgeEntry,
  type KnowledgeListRequest,
} from './knowledge';

export type KnowledgeBulkAction = 'enable' | 'disable' | 'delete';
export interface KnowledgeBulkResult {
  action: KnowledgeBulkAction;
  succeeded: number;
  unchanged: number;
  failures: { entry: KnowledgeEntry; message: string }[];
}

export async function collectKnowledgeSnapshot(
  request: KnowledgeListRequest,
  signal: AbortSignal,
): Promise<KnowledgeEntry[]> {
  const entries: KnowledgeEntry[] = [];
  const ids = new Set<string>();
  let revision: number | undefined;
  let total: number | undefined;
  let offset = 0;
  do {
    signal.throwIfAborted();
    const page = await listKnowledge({ ...request, limit: 50, offset });
    signal.throwIfAborted();
    if ((revision !== undefined && revision !== page.storeRevision)
      || (total !== undefined && total !== page.total)) {
      throw new Error('Knowledge changed while collecting matches. Refresh and try again. No records were changed.');
    }
    revision = page.storeRevision;
    total = page.total;
    for (const entry of page.entries) {
      if (ids.has(entry.id)) throw new Error('Knowledge pages changed. Refresh and try again. No records were changed.');
      ids.add(entry.id);
      entries.push(entry);
    }
    if (page.nextOffset === null) {
      if (entries.length !== total) throw new Error('Knowledge matches changed. Refresh and try again. No records were changed.');
      return entries;
    }
    if (page.nextOffset !== entries.length || page.nextOffset <= offset) {
      throw new Error('Knowledge pages changed. Refresh and try again. No records were changed.');
    }
    offset = page.nextOffset;
  } while (true);
}

export async function applyKnowledgeBulk(
  entries: KnowledgeEntry[],
  action: KnowledgeBulkAction,
  signal: AbortSignal,
): Promise<KnowledgeBulkResult> {
  const result: KnowledgeBulkResult = { action, succeeded: 0, unchanged: 0, failures: [] };
  for (const entry of entries) {
    signal.throwIfAborted();
    if (action !== 'delete' && entry.enabled === (action === 'enable')) {
      result.unchanged += 1;
      continue;
    }
    try {
      if (action === 'delete') await deleteKnowledge(entry);
      else await setKnowledgeEnabled(entry, action === 'enable');
      result.succeeded += 1;
    } catch (cause) {
      result.failures.push({ entry, message: String(cause) });
    }
  }
  return result;
}
