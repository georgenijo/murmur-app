import { useCallback, useEffect, useState } from 'react';
import {
  getKnowledgeStatus,
  listKnowledge,
  type KnowledgeEntry,
  type KnowledgeListRequest,
  type KnowledgeStoreStatus,
} from '../knowledge';

const UNAVAILABLE: KnowledgeStoreStatus = {
  availability: 'unavailable',
  schemaVersion: 0,
  recordCount: 0,
  storeRevision: 0,
  recoveryAtMs: null,
  message: 'The local knowledge store is unavailable.',
};

export function useKnowledge(request: KnowledgeListRequest, active: boolean) {
  const [status, setStatus] = useState<KnowledgeStoreStatus>(UNAVAILABLE);
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!active) return;
    setLoading(true);
    setError(null);
    try {
      const nextStatus = await getKnowledgeStatus();
      setStatus(nextStatus);
      if (nextStatus.availability === 'unavailable') {
        setEntries([]);
        setTotal(0);
        setNextOffset(null);
        return;
      }
      const page = await listKnowledge({ ...request, limit: 50, offset: 0 });
      setEntries(page.entries);
      setTotal(page.total);
      setNextOffset(page.nextOffset);
      setStatus((current) => ({
        ...current,
        recordCount: nextStatus.recordCount,
        storeRevision: page.storeRevision,
      }));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, [active, request.enabled, request.kind, request.query, request.scopeKind, request.voiceCommand]);

  const loadMore = useCallback(async () => {
    if (loading || nextOffset === null) return;
    setLoading(true);
    setError(null);
    try {
      const page = await listKnowledge({ ...request, limit: 50, offset: nextOffset });
      setEntries((current) => [...current, ...page.entries]);
      setTotal(page.total);
      setNextOffset(page.nextOffset);
      setStatus((current) => ({ ...current, storeRevision: page.storeRevision }));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  }, [loading, nextOffset, request.enabled, request.kind, request.query, request.scopeKind, request.voiceCommand]);

  useEffect(() => { void refresh(); }, [refresh]);

  return {
    status,
    entries,
    total,
    nextOffset,
    loading,
    error,
    refresh,
    loadMore,
    setStatus,
  };
}
