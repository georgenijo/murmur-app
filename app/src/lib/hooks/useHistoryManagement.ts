import { useState, useCallback } from 'react';
import type { TeachingContext } from '../correctAndTeach';
import {
  HistoryEntry,
  HistorySource,
  loadHistory,
  saveHistory,
  addHistoryEntry,
  updateHistoryEntry,
  togglePinned,
  removeUnpinned,
  clearHistory as clearPersistedHistory,
} from '../history';

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

  /** Pin/unpin one entry. Refused at the pin ceiling — `togglePinned` returns
   *  the same array, so nothing is written and the panel keeps its state. */
  const togglePin = useCallback((id: string) => {
    setHistoryEntries(prev => {
      const newHistory = togglePinned(prev, id);
      if (newHistory === prev) return prev;
      saveHistory(newHistory);
      return newHistory;
    });
  }, []);

  /** Clear everything the user did not explicitly pin. */
  const clearUnpinnedEntries = useCallback(() => {
    setHistoryEntries(prev => {
      const newHistory = removeUnpinned(prev);
      if (newHistory.length === 0) {
        clearPersistedHistory();
      } else {
        saveHistory(newHistory);
      }
      return newHistory;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistoryEntries([]);
    clearPersistedHistory();
  }, []);

  return { historyEntries, addEntry, updateEntry, togglePin, clearUnpinnedEntries, clearHistory };
}
