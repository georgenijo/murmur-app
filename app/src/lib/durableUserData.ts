import { persistSourceState } from './webkitLifecycle';
import { invoke, isTauri } from '@tauri-apps/api/core';

export interface DurableBlobStore {
  label: string;
  storageKey: string;
  loadCommand: string;
  saveCommand: string;
  clearCommand: string;
}

export const HISTORY_STORE: DurableBlobStore = {
  label: 'history',
  storageKey: 'dictation-history',
  loadCommand: 'load_history_blob',
  saveCommand: 'save_history_blob',
  clearCommand: 'clear_history_blob',
};

export const STATS_STORE: DurableBlobStore = {
  label: 'statistics',
  storageKey: 'dictation-stats',
  loadCommand: 'load_stats_blob',
  saveCommand: 'save_stats_blob',
  clearCommand: 'clear_stats_blob',
};

export const THEME_LIBRARY_STORE: DurableBlobStore = {
  label: 'theme library',
  storageKey: 'murmur-theme-library',
  loadCommand: 'load_theme_library_blob',
  saveCommand: 'save_theme_library_blob',
  clearCommand: 'clear_theme_library_blob',
};

function tauriAvailable(): boolean {
  try {
    return isTauri();
  } catch {
    return false;
  }
}

function reportStorageError(_store: DurableBlobStore, message: string, _error: unknown): void { console.error(message); }

/**
 * Keep localStorage as the synchronous frontend cache while mirroring the exact
 * serialized blob to the Rust-owned durable file. Plain-browser builds and
 * tests continue to work without a Tauri bridge.
 */
export function mirrorDurableBlob(store: DurableBlobStore, blob: string): void {
  if (store.storageKey === HISTORY_STORE.storageKey || store.storageKey === STATS_STORE.storageKey) throw new Error('Completion data is Rust-owned');
  persistSourceState(store.storageKey, blob);
}

export function saveDurableBlob(store: DurableBlobStore, blob: string): void {
  if (store.storageKey === HISTORY_STORE.storageKey || store.storageKey === STATS_STORE.storageKey) throw new Error('Completion data is Rust-owned');
  try {
    localStorage.setItem(store.storageKey, blob);
  } catch (error) {
    reportStorageError(store, `Failed to cache ${store.label}:`, error);
  }
  // The cache is an optimization, not a prerequisite for the durable write.
  mirrorDurableBlob(store, blob);
}

export function clearDurableBlob(store: DurableBlobStore): void {
  if (store.storageKey === HISTORY_STORE.storageKey || store.storageKey === STATS_STORE.storageKey) throw new Error('Completion data is Rust-owned');
  try {
    localStorage.removeItem(store.storageKey);
  } catch (error) {
    reportStorageError(store, `Failed to clear cached ${store.label}:`, error);
  }
  try {
    if (!tauriAvailable()) return;
    void invoke(store.clearCommand).catch((error) => {
      reportStorageError(store, `Failed to clear durable ${store.label}:`, error);
    });
  } catch (error) {
    reportStorageError(store, `Failed to clear durable ${store.label}:`, error);
  }
}

export async function hydrateDurableStore(store: DurableBlobStore): Promise<void> {
  if (store.storageKey === HISTORY_STORE.storageKey || store.storageKey === STATS_STORE.storageKey) throw new Error('Completion data is Rust-owned');
  try {
    if (!tauriAvailable()) return;
    const blob = await invoke<string | null>(store.loadCommand);
    if (typeof blob === 'string') {
      // The owning loader applies its full schema validation after startup.
      localStorage.setItem(store.storageKey, blob);
      return;
    }

    // One-time migration for clients whose only copy predates the durable
    // store. The Rust side re-checks the bounded JSON container.
    const cached = localStorage.getItem(store.storageKey);
    if (cached !== null) {
      await invoke(store.saveCommand, { blob: cached });
    }
  } catch (error) {
    // A storage problem must not block startup. Keep the localStorage cache as
    // the session fallback, matching the durable settings contract.
    reportStorageError(store, `Failed to hydrate ${store.label} from disk:`, error);
  }
}

/** Seed history and usage-stat caches before the main React tree renders. */
export async function hydrateUserDataFromDisk(): Promise<void> {
  const { initializeCompletionReader } = await import('./completionData');
  await initializeCompletionReader(true);
}

/** Hydrate the main-window-only installed theme library before its provider mounts. */
export async function hydrateThemeLibraryFromDisk(): Promise<void> {
  await hydrateDurableStore(THEME_LIBRARY_STORE);
}
