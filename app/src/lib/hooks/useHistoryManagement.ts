import { useState, useCallback, useEffect, useRef } from 'react';
import type { TeachingContext } from '../correctAndTeach';
import {
  HistoryEntry,
  HistorySource,
  HistoryInterruption,
  HistoryRecordingContext,
  loadHistory,
  saveHistory,
  addHistoryEntry,
  updateHistoryEntry,
  removeHistoryEntries,
  clearHistory as clearPersistedHistory,
  toggleHistoryEntryPinned,
} from '../history';

export function useHistoryManagement(retainHistory = true) {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(() => loadHistory());
  const retainHistoryRef = useRef(retainHistory);
  useEffect(() => {
    retainHistoryRef.current = retainHistory;
  }, [retainHistory]);

  const addEntry = useCallback((text: string, duration: number, source: HistorySource = 'recording', sourceName?: string, teachingContext?: TeachingContext, interruption?: HistoryInterruption, details?: { rawText: string; recording: HistoryRecordingContext }) => {
    if (!retainHistoryRef.current) return;
    setHistoryEntries(prev => {
      const newHistory = addHistoryEntry(prev, text, duration, source, sourceName, teachingContext, interruption, details);
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

  const togglePinned = useCallback((target: HistoryEntry) => {
    setHistoryEntries(prev => {
      const result = toggleHistoryEntryPinned(prev, target);
      if (!result.changed) return prev;
      saveHistory(result.entries);
      return result.entries;
    });
  }, []);

  const deleteEntries = useCallback((targets: readonly HistoryEntry[]) => {
    setHistoryEntries(prev => {
      const next = removeHistoryEntries(prev, targets);
      if (next.length === prev.length) return prev;
      saveHistory(next);
      return next;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistoryEntries([]);
    clearPersistedHistory();
  }, []);

  return { historyEntries, addEntry, updateEntry, togglePinned, deleteEntries, clearHistory };
}
