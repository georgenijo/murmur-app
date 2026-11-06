import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from './targetedEvents';
import type { HistoryEntry } from './history';
import type { DictationStats } from './stats';
import type { BenchmarkReport } from './benchmark';

export interface CompletionSnapshot {
  revision?: number;
  history: HistoryEntry[];
  statistics: DictationStats;
  reports: BenchmarkReport[];
  v1Committed: boolean;
  removeKeys: string[];
  retryNotice: boolean;
}
// Presentation snapshots only. No WebKit cache and no client-side durable writer.
let current: CompletionSnapshot | null = null;
export const COMPLETION_CHANGED = 'murmur-completion-changed';
export const COMPLETION_FAILED = 'murmur-completion-failed';
export const completionSnapshot = () => current;
export function acceptCompletionSnapshot(snapshot: CompletionSnapshot): CompletionSnapshot {
  if (snapshot.revision !== undefined && current?.revision !== undefined && snapshot.revision < current.revision) return current;
  current = snapshot;
  window.dispatchEvent(new Event(COMPLETION_CHANGED));
  return snapshot;
}
async function removeImportedKeys(snapshot: CompletionSnapshot): Promise<void> {
  for (const key of snapshot.removeKeys) localStorage.removeItem(key);
  const destinations = [
    snapshot.removeKeys.includes('dictation-history') ? 'history' : null,
    snapshot.removeKeys.includes('dictation-stats') ? 'statistics' : null,
    snapshot.removeKeys.includes('murmur-benchmark-reports') ? 'reports' : null,
  ].filter((value): value is string => value !== null);
  if (destinations.length) await invoke('acknowledge_completion_keys', { destinations });
}
export async function refreshCompletion(): Promise<CompletionSnapshot> {
  return acceptCompletionSnapshot(await invoke<CompletionSnapshot>('completion_snapshot'));
}
export async function completionMutation(command: string, args?: Record<string, unknown>): Promise<CompletionSnapshot> {
  try {
    const snapshot = acceptCompletionSnapshot(await invoke<CompletionSnapshot>(command, args));
    await removeImportedKeys(snapshot);
    return snapshot;
  } catch (error) {
    window.dispatchEvent(new Event(COMPLETION_FAILED));
    throw error;
  }
}
function legacy(key: string): unknown {
  const raw = localStorage.getItem(key);
  if (!raw) return null;
  try { return JSON.parse(raw) as unknown; } catch { return null; }
}
export async function importCompletionV1(): Promise<void> {
  try {
    const snapshot = await refreshCompletion();
    await removeImportedKeys(snapshot);
    if (snapshot.v1Committed) return;
    const reports = legacy('murmur-benchmark-reports');
    const single = legacy('murmur-benchmark-report');
    await completionMutation('import_completion_v1', { source: {
      history: legacy('dictation-history'), statistics: legacy('dictation-stats'),
      reports: reports ?? (single ? [single] : null),
    } });
  } finally {
    // Partial imports remove only destinations with durable markers, even if
    // another destination failed. Retry never reimports a marked destination.
    try { await removeImportedKeys(await refreshCompletion()); } catch { /* Retry notice owns failure. */ }
  }
}
let subscribed = false;
export async function initializeCompletionReader(importLegacy = false): Promise<void> {
  if (!isTauri()) return;
  if (!subscribed) {
    subscribed = true;
    await listen('history-changed', () => { void refreshCompletion().catch(() => {}); });
    await listen('statistics-changed', () => { void refreshCompletion().catch(() => {}); });
  }
  if (importLegacy) await importCompletionV1();
  else await refreshCompletion();
}
