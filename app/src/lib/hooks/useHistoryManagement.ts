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
  addDerivedHistoryEntry,
  updateHistoryEntry,
  clearHistory as clearPersistedHistory,
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

  const addDerivedEntry = useCallback((source: HistoryEntry, text: string, modeId: string, stages: import('../history').HistoryStageResult[]) => {
    if (!retainHistoryRef.current) return;
    setHistoryEntries(prev => {
      const next = addDerivedHistoryEntry(prev, source, text, modeId, stages);
      saveHistory(next);
      return next;
    });
  }, []);

  const clearHistory = useCallback(() => {
    setHistoryEntries([]);
    clearPersistedHistory();
  }, []);

  return { historyEntries, addEntry, addDerivedEntry, updateEntry, clearHistory };
}
