import { useCallback, useLayoutEffect, useRef, useState } from 'react';
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
  const generation = useRef(0);
  const pending = useRef(false);
  const requestKey = JSON.stringify([request.query, request.kind, request.enabled, request.scopeKind, request.voiceCommand]);
  const currentScope = useRef({ requestKey, active });
  const [loadedRevision, setLoadedRevision] = useState<number | null>(null);
  const [loadedKey, setLoadedKey] = useState<string | null>(null);

  useLayoutEffect(() => {
    currentScope.current = { requestKey, active };
    return () => {
      generation.current += 1;
      pending.current = false;
      currentScope.current = { requestKey, active: false };
    };
  }, [requestKey, active]);

  const refresh = useCallback(async () => {
    if (!active || !currentScope.current.active || currentScope.current.requestKey !== requestKey) return;
    const run = ++generation.current;
    const isCurrent = () => generation.current === run && currentScope.current.active;
    pending.current = true;
    setLoading(true);
    setError(null);
    try {
      const nextStatus = await getKnowledgeStatus();
      if (!isCurrent()) return;
      setStatus(nextStatus);
      if (nextStatus.availability === 'unavailable') {
        setEntries([]);
        setTotal(0);
        setNextOffset(null);
        setLoadedKey(requestKey);
        return;
      }
      const page = await listKnowledge({ ...request, limit: 50, offset: 0 });
      if (!isCurrent()) return;
      setLoadedKey(requestKey);
      setLoadedRevision(page.storeRevision);
      setEntries(page.entries);
      setTotal(page.total);
      setNextOffset(page.nextOffset);
      setStatus((current) => ({
        ...current,
        recordCount: nextStatus.recordCount,
        storeRevision: page.storeRevision,
      }));
    } catch (cause) {
      if (isCurrent()) setError(String(cause));
    } finally {
      if (isCurrent()) {
        pending.current = false;
        setLoading(false);
      }
    }
  }, [active, requestKey, request.enabled, request.kind, request.query, request.scopeKind, request.voiceCommand]);

  const loadMore = useCallback(async () => {
    if (!active || !currentScope.current.active || currentScope.current.requestKey !== requestKey
      || pending.current || loadedKey !== requestKey || nextOffset === null) return;
    const run = ++generation.current;
    const isCurrent = () => generation.current === run && currentScope.current.active;
    pending.current = true;
    setLoading(true);
    setError(null);
    try {
      const page = await listKnowledge({ ...request, limit: 50, offset: nextOffset });
      if (!isCurrent()) return;
      if (page.storeRevision !== loadedRevision) {
        await refresh();
        return;
      }
      setEntries((current) => [...current, ...page.entries]);
      setTotal(page.total);
      setNextOffset(page.nextOffset);
      setStatus((current) => ({ ...current, storeRevision: page.storeRevision }));
    } catch (cause) {
      if (isCurrent()) setError(String(cause));
    } finally {
      if (isCurrent()) {
        pending.current = false;
        setLoading(false);
      }
    }
  }, [active, loadedKey, nextOffset, refresh, requestKey, loadedRevision, request.enabled, request.kind, request.query, request.scopeKind, request.voiceCommand]);

  useLayoutEffect(() => { void refresh(); }, [refresh]);

  return {
    status,
    entries: loadedKey === requestKey && active ? entries : [],
    total: loadedKey === requestKey && active ? total : 0,
    nextOffset: loadedKey === requestKey && active ? nextOffset : null,
    loading: active && loading,
    error,
    refresh,
    loadMore,
    setStatus,
  };
}
