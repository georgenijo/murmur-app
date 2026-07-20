import { useState, useCallback } from 'react';
import type { TeachingContext } from '../correctAndTeach';
import { HistoryEntry, HistorySource, loadHistory, saveHistory, addHistoryEntry, updateHistoryEntry, clearHistory as clearPersistedHistory } from '../history';

export function useHistoryManagement() {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(() => loadHistory());

  const addEntry = useCallback((text: string, duration: number, source: HistorySource = 'recording', sourceName?: string, teachingContext?: TeachingContext) => {
    setHistoryEntries(prev => {
      const newHistory = addHistoryEntry(prev, text, duration, source, sourceName, teachingContext);
      saveHistory(newHistory);
      return newHistory;
    });
  }, []);

  const updateEntry = useCallback((id: string, text: string) => {
    setHistoryEntries(prev => {
      const newHistory = updateHistoryEntry(prev, id, text);
      saveHistory(newHistory);
      return newHistory;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistoryEntries([]);
    clearPersistedHistory();
  }, []);

  return { historyEntries, addEntry, updateEntry, clearHistory };
}
