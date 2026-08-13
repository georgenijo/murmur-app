import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  QUERY_HISTORY_PAGE_SIZE,
  clearQueryHistory,
  isQueryHistoryChanged,
  listQueryHistory,
  type QueryHistoryEntryV1,
} from '../queryHistory';
import type { QueryProviderId } from '../settings';

export type QueryHistoryProviderFilter = 'all' | QueryProviderId;

export function mergeQueryHistoryEntries(
  current: QueryHistoryEntryV1[],
  additions: QueryHistoryEntryV1[],
): QueryHistoryEntryV1[] {
  const byId = new Map(current.map((entry) => [entry.id, entry]));
  additions.forEach((entry) => byId.set(entry.id, entry));
  return [...byId.values()]
    .sort((left, right) => right.timestampMs - left.timestampMs || right.id.localeCompare(left.id))
    .slice(0, 200);
}

export function useQueryHistory(active: boolean) {
  const [entries, setEntries] = useState<QueryHistoryEntryV1[]>([]);
  const [provider, setProviderState] = useState<QueryHistoryProviderFilter>('all');
  const [total, setTotal] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestRef = useRef(0);
  const activeRef = useRef(active);
  const providerRef = useRef(provider);
  useEffect(() => { activeRef.current = active; }, [active]);
  useEffect(() => { providerRef.current = provider; }, [provider]);

  const refresh = useCallback(async () => {
    if (!activeRef.current) return;
    const request = ++requestRef.current;
    setLoading(true);
    try {
      const page = await listQueryHistory({
        offset: 0,
        limit: QUERY_HISTORY_PAGE_SIZE,
        provider: providerRef.current === 'all' ? null : providerRef.current,
      });
      if (request !== requestRef.current || !activeRef.current) return;
      setEntries(page.entries);
      setTotal(page.total);
      setHasMore(page.hasMore);
      setError(null);
    } catch {
      if (request === requestRef.current && activeRef.current) {
        setError('Voice Query history is unavailable.');
      }
    } finally {
      if (request === requestRef.current && activeRef.current) setLoading(false);
    }
  }, []);

  const loadMore = useCallback(async () => {
    if (!activeRef.current || loading || !hasMore) return;
    const request = ++requestRef.current;
    setLoading(true);
    try {
      const page = await listQueryHistory({
        offset: entries.length,
        limit: QUERY_HISTORY_PAGE_SIZE,
        provider: providerRef.current === 'all' ? null : providerRef.current,
      });
      if (request !== requestRef.current || !activeRef.current) return;
      setEntries((current) => mergeQueryHistoryEntries(current, page.entries));
      setTotal(page.total);
      setHasMore(page.hasMore);
      setError(null);
    } catch {
      if (request === requestRef.current && activeRef.current) {
        setError('More Voice Query history could not be loaded.');
      }
    } finally {
      if (request === requestRef.current && activeRef.current) setLoading(false);
    }
  }, [entries.length, hasMore, loading]);

  const clear = useCallback(async () => {
    if (clearing) return false;
    const request = ++requestRef.current;
    setClearing(true);
    try {
      await clearQueryHistory();
      if (request !== requestRef.current) return true;
      setEntries([]);
      setTotal(0);
      setHasMore(false);
      setError(null);
      return true;
    } catch {
      if (request === requestRef.current) setError('Voice Query history could not be cleared.');
      return false;
    } finally {
      if (request === requestRef.current) setClearing(false);
    }
  }, [clearing]);

  const setProvider = useCallback((next: QueryHistoryProviderFilter) => {
    if (next === providerRef.current) return;
    requestRef.current += 1;
    providerRef.current = next;
    setProviderState(next);
    setEntries([]);
    setTotal(0);
    setHasMore(false);
    setError(null);
  }, []);

  useEffect(() => {
    if (!active) {
      requestRef.current += 1;
      setEntries([]);
      setTotal(0);
      setHasMore(false);
      setError(null);
      setLoading(false);
      return;
    }
    void refresh();
  }, [active, provider, refresh]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<unknown>('query-history-changed', (event) => {
      if (disposed || !isQueryHistoryChanged(event.payload)) return;
      if (event.payload.kind === 'cleared') {
        requestRef.current += 1;
        setEntries([]);
        setTotal(0);
        setHasMore(false);
        setError(null);
        setClearing(false);
      } else if (activeRef.current) {
        void refresh();
      }
    }).then((value) => {
      if (disposed) value();
      else unlisten = value;
    }).catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refresh]);

  return {
    entries,
    provider,
    total,
    hasMore,
    loading,
    clearing,
    error,
    setProvider,
    refresh,
    loadMore,
    clear,
  };
}
