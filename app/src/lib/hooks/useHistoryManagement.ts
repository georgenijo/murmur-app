import { useState, useCallback } from 'react';
import { HistoryEntry, HistorySource, loadHistory, saveHistory, addHistoryEntry, clearHistory as clearPersistedHistory } from '../history';

export function useHistoryManagement() {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(() => loadHistory());

  const addEntry = useCallback((text: string, duration: number, source: HistorySource = 'recording', sourceName?: string) => {
    setHistoryEntries(prev => {
      const newHistory = addHistoryEntry(prev, text, duration, source, sourceName);
      saveHistory(newHistory);
      return newHistory;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistoryEntries([]);
    clearPersistedHistory();
  }, []);

  return { historyEntries, addEntry, clearHistory };
}
