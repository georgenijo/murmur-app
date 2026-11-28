import { useState, useCallback, useEffect, useRef } from 'react';
import {
  HistoryEntry,
  HistorySource,
  HistoryInterruption,
  HistoryRecordingContext,
  HistoryRetentionDays,
  loadHistory,
  saveHistory,
  addHistoryEntry,
  removeHistoryEntries,
  clearHistory as clearPersistedHistory,
  countExpiredHistory,
  pruneExpiredHistory,
  toggleHistoryEntryPinned,
} from '../history';

/** How often an open app re-checks the automatic-deletion window. */
export const HISTORY_RETENTION_SWEEP_MS = 60 * 60 * 1000;

export function useHistoryManagement(retainHistory = true, retentionDays: HistoryRetentionDays = 0) {
  const [historyEntries, setHistoryEntries] = useState<HistoryEntry[]>(() => loadHistory());
  const retainHistoryRef = useRef(retainHistory);
  useEffect(() => {
    retainHistoryRef.current = retainHistory;
  }, [retainHistory]);
  const entriesRef = useRef(historyEntries);
  useEffect(() => {
    entriesRef.current = historyEntries;
  }, [historyEntries]);

  // Opt-in automatic deletion: sweep when the window is set or changes, then
  // hourly while the app stays open. "Never" (0) never touches history.
  useEffect(() => {
    if (retentionDays === 0) return;
    const sweep = () => setHistoryEntries(prev => {
      const next = pruneExpiredHistory(prev, retentionDays, Date.now());
      if (next === prev) return prev;
      saveHistory(next);
      return next;
    });
    sweep();
    const timer = window.setInterval(sweep, HISTORY_RETENTION_SWEEP_MS);
    return () => window.clearInterval(timer);
  }, [retentionDays]);

  /** How many current entries a window would delete, for a confirmation prompt. */
  const countExpired = useCallback(
    (days: HistoryRetentionDays) => countExpiredHistory(entriesRef.current, days, Date.now()),
    [],
  );

  const addEntry = useCallback((text: string, duration: number, source: HistorySource = 'recording', sourceName?: string, interruption?: HistoryInterruption, details?: { rawText: string; recording: HistoryRecordingContext }) => {
    if (!retainHistoryRef.current) return;
    setHistoryEntries(prev => {
      const newHistory = addHistoryEntry(prev, text, duration, source, sourceName, interruption, details);
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

  return { historyEntries, addEntry, togglePinned, deleteEntries, clearHistory, countExpired };
}
