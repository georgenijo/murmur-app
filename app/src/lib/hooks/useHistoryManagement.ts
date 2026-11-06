import { useState, useCallback, useEffect } from 'react';
import { HistoryEntry, loadHistory, clearHistory as clearPersistedHistory } from '../history';
import { COMPLETION_CHANGED, completionMutation, initializeCompletionReader } from '../completionData';

export function useHistoryManagement(_retainHistory = true) {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(loadHistory);
  useEffect(() => {
    const changed = () => setHistoryEntries(loadHistory());
    window.addEventListener(COMPLETION_CHANGED, changed);
    void initializeCompletionReader().catch(() => {});
    return () => window.removeEventListener(COMPLETION_CHANGED, changed);
  }, []);
  const mutate = useCallback((mutation: Record<string, unknown>) => {
    void completionMutation('mutate_history', { mutation }).catch(() => {});
  }, []);
  const updateEntry = useCallback((id: string, text: string) => {
    const target = historyEntries.find(entry => entry.id === id);
    if (target) mutate({ kind: 'edit', target, text });
  }, [historyEntries, mutate]);
  const togglePinned = useCallback((target: HistoryEntry) => mutate({ kind: 'pin', target }), [mutate]);
  const deleteEntries = useCallback((targets: readonly HistoryEntry[]) => mutate({ kind: 'delete', targets }), [mutate]);
  const clearHistory = useCallback(() => { void clearPersistedHistory().catch(() => {}); }, []);
  return { historyEntries, updateEntry, togglePinned, deleteEntries, clearHistory };
}
