import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { HistoryEntry } from './history';
import type { DictationStats } from './stats';
import type { BenchmarkReport } from './benchmark';

export interface CompletionSnapshot {
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
export const completionSnapshot = () => current;
export function acceptCompletionSnapshot(snapshot: CompletionSnapshot): CompletionSnapshot {
  current = snapshot;
  window.dispatchEvent(new Event(COMPLETION_CHANGED));
  return snapshot;
}
function removeImportedKeys(snapshot: CompletionSnapshot): void {
  for (const key of snapshot.removeKeys) localStorage.removeItem(key);
}
export async function refreshCompletion(): Promise<CompletionSnapshot> {
  return acceptCompletionSnapshot(await invoke<CompletionSnapshot>('completion_snapshot'));
}
export async function completionMutation(command: string, args?: Record<string, unknown>): Promise<CompletionSnapshot> {
  const snapshot = acceptCompletionSnapshot(await invoke<CompletionSnapshot>(command, args));
  removeImportedKeys(snapshot);
  return snapshot;
}
function legacy(key: string): unknown {
  const raw = localStorage.getItem(key);
  if (!raw) return null;
  try { return JSON.parse(raw) as unknown; } catch { return null; }
}
export async function importCompletionV1(): Promise<void> {
  try {
    const snapshot = await refreshCompletion();
    removeImportedKeys(snapshot);
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
    try { removeImportedKeys(await refreshCompletion()); } catch { /* Retry notice owns failure. */ }
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
