import { useState, useCallback } from 'react';
import { HistoryEntry, loadHistory, saveHistory, addHistoryEntry, clearHistory as clearPersistedHistory } from '../history';

export function useHistoryManagement() {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(() => loadHistory());

  const addEntry = useCallback((text: string, duration: number) => {
    setHistoryEntries(prev => {
      const newHistory = addHistoryEntry(prev, text, duration);
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
