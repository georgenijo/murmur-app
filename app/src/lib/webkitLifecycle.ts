import { invoke, isTauri } from '@tauri-apps/api/core';

// This cache contains presentation source state only. Migration ownership and
// validation belong to murmur-core; malformed raw strings reach Rust intact.
export const V2_KEYS = [
  'dictation-settings', 'murmur-theme-library', 'murmur-appearance',
  'murmur_discovery_v1', 'murmur.settings-disclosure.v1', 'usage-dashboard-collapsed',
  'murmur-microphone-startup-benchmark-reports', 'murmur_onboarding_complete',
] as const;
const LEGACY_KEYS = ['dictation-history', 'dictation-stats', 'murmur-benchmark-report', 'murmur-benchmark-reports', ...V2_KEYS];
interface StateReceipt { v1: boolean; v2: boolean; values: Record<string, unknown> }
export const SOURCE_STORAGE_FAILED = 'murmur-source-storage-error';
export const SOURCE_STORAGE_SAVED = 'murmur-source-storage-saved';
let writes: Promise<unknown> = Promise.resolve();
let generation = 0;
export type OwnedEdits = Partial<Record<'autoPaste' | 'disabled', boolean>>;
const pending = new Map<string, { raw: string; generation: number; ownedEdits?: OwnedEdits }>();
/** Ordered, retryable commits. A failed write retains the newest intent for
 * each key; an older async completion cannot discard a newer pending edit. */
export function persistSourceState(key: string, raw: string, ownedEdits?: OwnedEdits): void {
  try { if (!isTauri()) return; } catch { return; }
  pending.set(key, { raw, generation: ++generation, ...(ownedEdits || pending.get(key)?.ownedEdits ? { ownedEdits: { ...pending.get(key)?.ownedEdits, ...ownedEdits } } : {}) });
  void flushSourceState().catch(() => {});
}
export function flushSourceState(): Promise<unknown> {
  writes = writes.catch(() => {}).then(async () => {
    try {
      for (const [key, intent] of pending) {
        if (key === 'dictation-settings' || key === 'murmur-theme-library') {
          await invoke(key === 'dictation-settings' ? 'save_settings_blob' : 'save_theme_library_blob', { blob: intent.raw, ...(intent.ownedEdits ? { ownedEdits: intent.ownedEdits } : {}) });
        } else {
          await invoke('save_webkit_source', { key, value: JSON.parse(intent.raw) as unknown });
        }
        if (pending.get(key)?.generation === intent.generation) pending.delete(key);
      }
      window.dispatchEvent(new Event(SOURCE_STORAGE_SAVED));
    } catch {
      window.dispatchEvent(new Event(SOURCE_STORAGE_FAILED));
      throw new Error('Source storage unavailable');
    }
  });
  return writes;
}
/** Rebase a retained intent after a newer native quick-control commit. */
export function rebasePendingOwnedSettings(settings: { autoPaste: boolean; disabled: boolean; ownedFieldsRevision?: number }): void {
  const intent = pending.get('dictation-settings');
  if (!intent) return;
  const value = JSON.parse(intent.raw) as Record<string, unknown>;
  pending.set('dictation-settings', { generation: ++generation, raw: JSON.stringify({ ...value, autoPaste: settings.autoPaste, disabled: settings.disabled, ownedFieldsRevision: settings.ownedFieldsRevision }) });
}
export const sourceStorage = {
  getItem: (key: string) => localStorage.getItem(key),
  setItem(key: string, value: string) {
    persistSourceState(key, value);
    try { localStorage.setItem(key, value); } catch { /* Rust intent remains queued. */ }
  },
};
export async function initializeWebkitState(importLegacy = false): Promise<void> {
  try { if (!isTauri()) return; } catch { return; }
  try {
  let state = await invoke<StateReceipt>('webkit_state_snapshot');
  if (importLegacy && (!state.v1 || !state.v2)) {
    // Do not catch getItem failures or parse JSON here: either would turn an
    // unreadable/corrupt store into a destructive empty-source receipt.
    const source = Object.fromEntries(LEGACY_KEYS.map(key => [key, localStorage.getItem(key)]));
    state = await invoke<StateReceipt>('import_webkit_state', { source });
  }

  for (const [key, value] of Object.entries(state.values)) {
    localStorage.setItem(key, JSON.stringify(value));
  }
  if (state.v2) {
    localStorage.removeItem('murmur-overlay-vertical-offset');
    localStorage.removeItem('murmur-ui-latency-v1');
  }
  } finally {
    if (importLegacy) {
      const { refreshCompletion, removeImportedKeys } = await import('./completionData');
      try { await removeImportedKeys(await refreshCompletion()); } catch { /* Retry on next startup. */ }
    }
  }
}
